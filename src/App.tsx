import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import "./App.css";
import type {
  BranchList,
  BranchRef,
  CompareMode,
  DiffFile,
  DiffParams,
  DiffSummary,
  FileContents,
  RepoInfo,
  SourceScope,
} from "./types";
import { MonacoDiffView } from "./MonacoDiffView";
import { BranchDropdown } from "./BranchDropdown";
import { ProjectChip } from "./ProjectChip";
import { EmptyState } from "./EmptyState";
import { formatBytes } from "./utils";

type Theme = "light" | "dark";

/** Whether `source` (a fully-qualified ref, as selected via `BranchDropdown`)
 * is the checked-out branch (DESIGN.md 3.3 HEAD constraint). Mirrors the
 * Rust-side `source_matches_head` in src-tauri/src/git_service/commands.rs. */
function sourceMatchesHead(source: string, currentBranch: string | null): boolean {
  if (!currentBranch) return false;
  return source === `refs/heads/${currentBranch}`;
}

/** Picks sensible default target/source branches once the branch list loads
 * (DESIGN.md doesn't prescribe this — a reasonable default keeps the app
 * usable without forcing the user to pick both every time): target prefers
 * `main`/`master`, source prefers the checked-out branch. */
function pickDefaultBranches(branches: BranchList): { target: BranchRef | null; source: BranchRef | null } {
  const byShort = (name: string) => branches.local.find((b) => b.short === name) ?? null;
  const target =
    byShort("main") ??
    byShort("master") ??
    branches.local.find((b) => b.short !== branches.current) ??
    branches.local[0] ??
    null;
  const source =
    (branches.current ? byShort(branches.current) : null) ??
    branches.local.find((b) => b.full !== target?.full) ??
    branches.local[0] ??
    null;
  return { target, source };
}

function App() {
  const [theme, setTheme] = useState<Theme>("light");

  // Project selection (DESIGN.md 3.1).
  const [repoPath, setRepoPath] = useState("");
  const [repoInfo, setRepoInfo] = useState<RepoInfo | null>(null);
  const [projectError, setProjectError] = useState<string | null>(null);
  const [recentProjects, setRecentProjects] = useState<string[]>([]);

  // Branch selection (DESIGN.md 3.2).
  const [branches, setBranches] = useState<BranchList | null>(null);
  const [target, setTarget] = useState(""); // fully-qualified ref
  const [source, setSource] = useState(""); // fully-qualified ref

  const [sourceScope, setSourceScope] = useState<SourceScope>("committed");
  const [compareMode, setCompareMode] = useState<CompareMode>("merge-base");
  const [summary, setSummary] = useState<DiffSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  // Currently selected file + its lazily-fetched full-text contents.
  const [selectedFile, setSelectedFile] = useState<DiffFile | null>(null);
  const [fileContents, setFileContents] = useState<FileContents | null>(null);
  const [fileError, setFileError] = useState<string | null>(null);
  const [fileLoading, setFileLoading] = useState(false);
  const [lastParams, setLastParams] = useState<DiffParams | null>(null);

  // Drive both app CSS ([data-theme]) and Monaco theme from one toggle.
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

  // Load the recent-projects list once on startup (DESIGN.md 3.1).
  useEffect(() => {
    invoke<string[]>("get_recent_projects")
      .then(setRecentProjects)
      .catch(() => {
        /* non-fatal: the empty-state screen just shows no recents */
      });
  }, []);

  /** Validates `path` as a git repository and, on success, adopts it as the
   * active project (DESIGN.md 3.1). Used by both the folder-picker dialog
   * and clicking a recent-project entry. */
  const validateAndSetRepo = useCallback(async (path: string) => {
    try {
      const info = await invoke<RepoInfo>("validate_repo", { path });
      setRepoInfo(info);
      setRepoPath(info.toplevel);
      setProjectError(null);
      setBranches(null);
      setTarget("");
      setSource("");
      setSummary(null);
      setSelectedFile(null);
      setFileContents(null);

      // Fetched directly here (DESIGN.md 3.2), rather than via a `repoPath`-
      // keyed effect: re-picking the *same* project (e.g. from Recent) would
      // otherwise leave `branches` stuck at the `null` reset above, since
      // `repoPath` wouldn't actually change.
      try {
        const list = await invoke<BranchList>("list_branches", { path: info.toplevel });
        setBranches(list);
      } catch (e) {
        setError(String(e));
      }

      const updated = await invoke<string[]>("add_recent_project", { path: info.toplevel });
      setRecentProjects(updated);
    } catch (e) {
      setProjectError(String(e));
    }
  }, []);

  const chooseRepository = useCallback(async () => {
    const selection = await open({ directory: true, multiple: false });
    if (typeof selection === "string") {
      await validateAndSetRepo(selection);
    }
  }, [validateAndSetRepo]);

  // Fill in default target/source once branches load, if nothing is picked
  // yet or the previous picks no longer exist in the new repo's branch list.
  useEffect(() => {
    if (!branches) return;
    const known = new Set([...branches.local, ...branches.remote].map((b) => b.full));
    const needsTarget = !target || !known.has(target);
    const needsSource = !source || !known.has(source);
    if (!needsTarget && !needsSource) return;
    const defaults = pickDefaultBranches(branches);
    if (needsTarget && defaults.target) setTarget(defaults.target.full);
    if (needsSource && defaults.source) setSource(defaults.source.full);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [branches]);

  const isSourceHead = sourceMatchesHead(source, repoInfo?.currentBranch ?? null);
  const scopeLocked = !isSourceHead;
  const lockReason = scopeLocked
    ? `"${source || "(source)"}" is not the checked-out branch (HEAD is ${
        repoInfo?.currentBranch ?? "detached/unknown"
      }) — staged and unstaged changes only exist in the working tree of HEAD.`
    : undefined;

  const showDiff = useCallback(async () => {
    if (!repoPath || !target || !source) return;
    setLoading(true);
    setError(null);
    setSummary(null);
    setSelectedFile(null);
    setFileContents(null);
    setFileError(null);
    try {
      const params: DiffParams = {
        repoPath,
        target,
        source,
        compareMode,
        sourceScope,
        options: {},
      };
      const result = await invoke<DiffSummary>("get_diff_summary", { params });
      setSummary(result);
      setLastParams(params);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [repoPath, target, source, compareMode, sourceScope]);

  // Auto-fetch the summary once target/source (and repo/mode/scope) are all
  // set, so the user doesn't have to press "Show diff" after every change
  // (task step 9). The button below stays as an explicit re-fetch.
  const autoFetchKey = useRef("");
  useEffect(() => {
    if (!repoPath || !target || !source) return;
    const key = JSON.stringify({ repoPath, target, source, compareMode, sourceScope });
    if (autoFetchKey.current === key) return;
    autoFetchKey.current = key;
    showDiff();
  }, [repoPath, target, source, compareMode, sourceScope, showDiff]);

  async function loadFileDiff(file: DiffFile, force: boolean) {
    if (!lastParams) return;
    setSelectedFile(file);
    setFileLoading(true);
    setFileError(null);
    if (!force) {
      setFileContents(null);
    }
    try {
      const result = await invoke<FileContents>("get_file_diff", {
        params: lastParams,
        path: file.path,
        oldPath: file.oldPath,
        force,
      });
      setFileContents(result);
    } catch (e) {
      setFileError(String(e));
    } finally {
      setFileLoading(false);
    }
  }

  if (!repoPath) {
    return (
      <main className="container empty-state-container">
        <EmptyState
          error={projectError}
          recentProjects={recentProjects}
          onChoose={chooseRepository}
          onPickRecent={validateAndSetRepo}
        />
      </main>
    );
  }

  return (
    <main className="container">
      <div className="app-header">
        <div className="app-header-left">
          <h1>Branch Diff Viewer</h1>
          <ProjectChip
            repoPath={repoPath}
            recentProjects={recentProjects}
            onChoose={chooseRepository}
            onPickRecent={validateAndSetRepo}
          />
        </div>
        <button
          type="button"
          className="theme-toggle"
          onClick={() => setTheme((t) => (t === "light" ? "dark" : "light"))}
        >
          {theme === "light" ? "Dark" : "Light"} theme
        </button>
      </div>

      {projectError && <p className="error">{projectError}</p>}

      <form
        className="diff-form"
        onSubmit={(e) => {
          e.preventDefault();
          showDiff();
        }}
      >
        <div className="branch-row">
          <BranchDropdown
            label="Base · target"
            branches={branches}
            value={target}
            onChange={(b) => setTarget(b.full)}
          />
          <span className="branch-row-arrow">←</span>
          <BranchDropdown
            label="Head · source"
            branches={branches}
            value={source}
            onChange={(b) => setSource(b.full)}
          />
        </div>

        <div className="segmented-row">
          <div className="segmented-group">
            <span className="segmented-label">Source scope</span>
            <SegmentedControl<SourceScope>
              value={sourceScope}
              onChange={setSourceScope}
              options={[
                { value: "committed", label: "Committed" },
                { value: "staged", label: "Staged", disabled: scopeLocked, title: lockReason },
                { value: "unstaged", label: "Unstaged", disabled: scopeLocked, title: lockReason },
              ]}
            />
          </div>
          <div className="segmented-group">
            <span className="segmented-label">Compare</span>
            <SegmentedControl<CompareMode>
              value={compareMode}
              onChange={setCompareMode}
              options={[
                { value: "merge-base", label: "merge-base" },
                { value: "tips", label: "tips" },
              ]}
            />
          </div>
        </div>

        {scopeLocked && sourceScope !== "committed" && (
          <p className="banner banner-warning">{lockReason}</p>
        )}

        <button type="submit" disabled={loading || !target || !source}>
          {loading ? "Loading…" : "Show diff"}
        </button>
      </form>

      {error && <p className="error">{error}</p>}

      {summary && (
        <section className="results">
          {summary.warnings.map((w, i) => (
            <p key={i} className="banner banner-warning">
              {w}
            </p>
          ))}
          <p className="totals">
            {summary.summary.filesChanged} files changed, +{summary.summary.additions} -
            {summary.summary.deletions}
          </p>
          <ul className="file-list">
            {summary.files.map((f) => (
              <FileRow
                key={f.path}
                file={f}
                isSelected={selectedFile?.path === f.path}
                onSelect={() => loadFileDiff(f, false)}
              />
            ))}
            {typeof summary.omittedUntracked === "number" && summary.omittedUntracked > 0 && (
              <li className="file-row file-row-omitted">
                +{summary.omittedUntracked} more (untracked)
              </li>
            )}
          </ul>

          <FileDiffPane
            file={selectedFile}
            contents={fileContents}
            loading={fileLoading}
            error={fileError}
            theme={theme}
            onLoadAnyway={() => selectedFile && loadFileDiff(selectedFile, true)}
          />
        </section>
      )}
    </main>
  );
}

function FileRow({
  file,
  isSelected,
  onSelect,
}: {
  file: DiffFile;
  isSelected: boolean;
  onSelect: () => void;
}) {
  const label = STATUS_LABEL[file.status] ?? file.status;
  return (
    <li className={`file-row${isSelected ? " file-row-selected" : ""}`}>
      <button type="button" className="file-row-button" onClick={onSelect}>
        <span className={`status status-${file.status}`}>{label}</span>
        <span className="path">
          {file.oldPath ? `${file.oldPath} → ${file.path}` : file.path}
        </span>
        {file.isUntracked && <span className="badge badge-untracked">untracked</span>}
        {file.isBinary ? (
          <span className="stats binary">binary</span>
        ) : (
          <span className="stats">
            <span className="additions">+{file.additions ?? 0}</span>{" "}
            <span className="deletions">-{file.deletions ?? 0}</span>
          </span>
        )}
      </button>
    </li>
  );
}

interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  disabled?: boolean;
  title?: string;
}

function SegmentedControl<T extends string>({
  value,
  onChange,
  options,
}: {
  value: T;
  onChange: (v: T) => void;
  options: SegmentedOption<T>[];
}) {
  return (
    <div className="segmented">
      {options.map((opt) => (
        <button
          key={opt.value}
          type="button"
          className={`segmented-option${value === opt.value ? " segmented-option-active" : ""}`}
          disabled={opt.disabled}
          title={opt.disabled ? opt.title : undefined}
          onClick={() => onChange(opt.value)}
        >
          {opt.label}
        </button>
      ))}
    </div>
  );
}

function FileDiffPane({
  file,
  contents,
  loading,
  error,
  theme,
  onLoadAnyway,
}: {
  file: DiffFile | null;
  contents: FileContents | null;
  loading: boolean;
  error: string | null;
  theme: Theme;
  onLoadAnyway: () => void;
}) {
  if (!file) return null;

  const showEditor =
    !loading && !error && contents && !contents.isTooLarge && !contents.isBinary;

  return (
    <section className="file-diff-pane">
      <h2 className="file-diff-title">{file.path}</h2>

      {loading && <p>Loading…</p>}
      {error && <p className="error">{error}</p>}

      {!loading && !error && contents?.isTooLarge && (
        <p className="file-diff-notice">
          Large file ({formatBytes(contents.sizeBytes ?? 0)}) —{" "}
          <button type="button" onClick={onLoadAnyway}>
            Load anyway
          </button>
        </p>
      )}

      {!loading && !error && contents?.isBinary && (
        <p className="file-diff-notice">Binary file</p>
      )}

      {showEditor && (
        <MonacoDiffView
          path={file.path}
          original={contents.base ?? ""}
          modified={contents.head ?? ""}
          theme={theme}
        />
      )}
    </section>
  );
}

const STATUS_LABEL: Record<string, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  typechange: "T",
  unmerged: "U",
  submodule: "S",
  other: "?",
};

export default App;
