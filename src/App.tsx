import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";
import type {
  CompareMode,
  DiffFile,
  DiffParams,
  DiffSummary,
  FileContents,
  RepoInfo,
  SourceScope,
} from "./types";
import { MonacoDiffView } from "./MonacoDiffView";

type Theme = "light" | "dark";

/** Whether `source` (short or fully-qualified) is the checked-out branch
 * (DESIGN.md 3.3 HEAD constraint). Mirrors the Rust-side
 * `source_matches_head` in src-tauri/src/git_service/commands.rs. */
function sourceMatchesHead(source: string, currentBranch: string | null): boolean {
  if (!currentBranch) return false;
  return source === currentBranch || source === `refs/heads/${currentBranch}`;
}

function App() {
  const [theme, setTheme] = useState<Theme>("light");
  const [repoPath, setRepoPath] = useState("");
  const [target, setTarget] = useState("main");
  const [source, setSource] = useState("");
  const [sourceScope, setSourceScope] = useState<SourceScope>("committed");
  const [compareMode, setCompareMode] = useState<CompareMode>("merge-base");
  const [summary, setSummary] = useState<DiffSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  // Currently selected file + its lazily-fetched full-text contents
  // (Phase 3 replaces the <pre> pair below with Monaco Diff View).
  const [selectedFile, setSelectedFile] = useState<DiffFile | null>(null);
  const [fileContents, setFileContents] = useState<FileContents | null>(null);
  const [fileError, setFileError] = useState<string | null>(null);
  const [fileLoading, setFileLoading] = useState(false);
  const [lastParams, setLastParams] = useState<DiffParams | null>(null);

  // RepoInfo (in particular `currentBranch`) drives the HEAD-constraint UI
  // lock on Staged/Unstaged (DESIGN.md 3.3). Re-validated whenever the repo
  // path changes; failures are swallowed here (the main `error` banner from
  // `showDiff` is the primary error surface).
  const [repoInfo, setRepoInfo] = useState<RepoInfo | null>(null);

  // Drive both app CSS ([data-theme]) and Monaco theme from one toggle.
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

  useEffect(() => {
    let cancelled = false;
    if (!repoPath) {
      setRepoInfo(null);
      return;
    }
    invoke<RepoInfo>("validate_repo", { path: repoPath })
      .then((info) => {
        if (!cancelled) setRepoInfo(info);
      })
      .catch(() => {
        if (!cancelled) setRepoInfo(null);
      });
    return () => {
      cancelled = true;
    };
  }, [repoPath]);

  const isSourceHead = sourceMatchesHead(source, repoInfo?.currentBranch ?? null);
  const scopeLocked = !isSourceHead;
  const lockReason = scopeLocked
    ? `"${source || "(source)"}" is not the checked-out branch (HEAD is ${
        repoInfo?.currentBranch ?? "detached/unknown"
      }) — staged and unstaged changes only exist in the working tree of HEAD.`
    : undefined;

  async function showDiff() {
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
  }

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

  return (
    <main className="container">
      <div className="app-header">
        <h1>Branch Diff Viewer</h1>
        <button
          type="button"
          className="theme-toggle"
          onClick={() => setTheme((t) => (t === "light" ? "dark" : "light"))}
        >
          {theme === "light" ? "Dark" : "Light"} theme
        </button>
      </div>

      <form
        className="diff-form"
        onSubmit={(e) => {
          e.preventDefault();
          showDiff();
        }}
      >
        <label>
          Repository path
          <input
            value={repoPath}
            onChange={(e) => setRepoPath(e.currentTarget.value)}
            placeholder="/path/to/repo"
          />
        </label>
        <label>
          Base (target)
          <input
            value={target}
            onChange={(e) => setTarget(e.currentTarget.value)}
            placeholder="main"
          />
        </label>
        <label>
          Head (source)
          <input
            value={source}
            onChange={(e) => setSource(e.currentTarget.value)}
            placeholder="feature/foo"
          />
          {repoInfo?.currentBranch && (
            <span className="head-hint">HEAD: {repoInfo.currentBranch}</span>
          )}
        </label>

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

        <button type="submit" disabled={loading}>
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

function formatBytes(n: number): string {
  return `${(n / (1024 * 1024)).toFixed(2)} MB`;
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
