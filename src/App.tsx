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
  UiSettings,
} from "./types";
import { MonacoDiffView } from "./MonacoDiffView";
import { ControlBar } from "./ControlBar";
import { EmptyState } from "./EmptyState";
import { FileList } from "./FileList";
import { WarningBanners } from "./WarningBanners";
import {
  BinaryPlaceholder,
  GitNotFound,
  LargeFilePlaceholder,
  LoadingPlaceholder,
  NoChangesPlaceholder,
  NoChangesWhitespacePlaceholder,
  SubmodulePlaceholder,
  SymlinkPlaceholder,
  TimeoutPlaceholder,
  UnselectedPlaceholder,
} from "./Placeholder";
import { gitVersionLabel, splitPath } from "./utils";

type Theme = "light" | "dark";

const isTimeout = (e: string | null) => !!e && e.startsWith("GIT_TIMEOUT:");
const isNotFound = (e: string | null) => !!e && e.startsWith("GIT_NOT_FOUND:");

/** Mirrors the Rust-side `source_matches_head` (DESIGN.md 3.3 HEAD constraint). */
function sourceMatchesHead(source: string, currentBranch: string | null): boolean {
  if (!currentBranch) return false;
  return source === `refs/heads/${currentBranch}`;
}

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

/** Short display name for a fully-qualified ref, via the branch list when
 * available, else by stripping the `refs/{heads,remotes}/` prefix. */
function shortName(full: string, branches: BranchList | null): string {
  const b = branches ? [...branches.local, ...branches.remote].find((x) => x.full === full) : null;
  if (b) return b.short;
  return full.replace(/^refs\/(heads|remotes)\//, "");
}

function App() {
  const [theme, setTheme] = useState<Theme>("light");

  const [repoPath, setRepoPath] = useState("");
  const [repoInfo, setRepoInfo] = useState<RepoInfo | null>(null);
  const [projectError, setProjectError] = useState<string | null>(null);
  const [recentProjects, setRecentProjects] = useState<string[]>([]);
  const lastAttemptRef = useRef<string>("");

  const [branches, setBranches] = useState<BranchList | null>(null);
  const [target, setTarget] = useState("");
  const [source, setSource] = useState("");

  const [sourceScope, setSourceScope] = useState<SourceScope>("committed");
  const [compareMode, setCompareMode] = useState<CompareMode>("merge-base");
  const [hideWhitespace, setHideWhitespace] = useState(true); // DESIGN.md 3.5: default ON
  const [summary, setSummary] = useState<DiffSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [whitespaceOnlyHidden, setWhitespaceOnlyHidden] = useState(false);

  const [selectedFile, setSelectedFile] = useState<DiffFile | null>(null);
  const [fileContents, setFileContents] = useState<FileContents | null>(null);
  const [fileError, setFileError] = useState<string | null>(null);
  const [fileLoading, setFileLoading] = useState(false);
  const [timedOut, setTimedOut] = useState(false);
  const [wrap, setWrap] = useState(false);
  const [lastParams, setLastParams] = useState<DiffParams | null>(null);

  // Refs mirroring state for use inside window-level event handlers.
  const selectedRef = useRef<DiffFile | null>(null);
  selectedRef.current = selectedFile;
  const lastParamsRef = useRef<DiffParams | null>(null);
  lastParamsRef.current = lastParams;
  const summaryRef = useRef<DiffSummary | null>(null);
  summaryRef.current = summary;
  const fingerprintRef = useRef<string | null>(null);
  const fileReqRef = useRef(0);
  const bigSizeRef = useRef<Record<string, number>>({});
  const busyRef = useRef(false);
  busyRef.current = loading || refreshing;

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

  useEffect(() => {
    invoke<string[]>("get_recent_projects").then(setRecentProjects).catch(() => {});
    invoke<UiSettings>("get_ui_settings")
      .then((s) => setHideWhitespace(s.hideWhitespace))
      .catch(() => {});
  }, []);

  const updateHideWhitespace = useCallback((value: boolean) => {
    setHideWhitespace(value);
    const settings: UiSettings = { hideWhitespace: value };
    invoke("set_ui_settings", { settings }).catch(() => {});
  }, []);

  const validateAndSetRepo = useCallback(async (path: string) => {
    lastAttemptRef.current = path;
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
      setError(null);
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
    if (typeof selection === "string") await validateAndSetRepo(selection);
  }, [validateAndSetRepo]);

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
  const detachedOrUnborn = !!repoInfo && (repoInfo.isDetached || !repoInfo.hasCommits);
  const scopeLocked = !isSourceHead || detachedOrUnborn;
  const lockReason = scopeLocked
    ? `"${shortName(source, branches) || "(source)"}" is not the checked-out branch (HEAD is ${
        repoInfo?.currentBranch ?? "detached/unborn"
      }) — staged and unstaged changes only exist in the working tree of HEAD.`
    : undefined;
  const effectiveScope: SourceScope = scopeLocked ? "committed" : sourceScope;

  const loadFileDiff = useCallback(async (file: DiffFile, force: boolean, params: DiffParams) => {
    const myId = ++fileReqRef.current;
    setSelectedFile(file);
    setFileLoading(true);
    setFileError(null);
    setTimedOut(false);
    if (!force) setFileContents(null);
    try {
      const result = await invoke<FileContents>("get_file_diff", {
        params,
        path: file.path,
        oldPath: file.oldPath,
        force,
      });
      if (fileReqRef.current !== myId) return; // superseded / cancelled
      if (result.isTooLarge && result.sizeBytes) bigSizeRef.current[file.path] = result.sizeBytes;
      setFileContents(result);
    } catch (e) {
      if (fileReqRef.current !== myId) return;
      setFileError(String(e));
    } finally {
      if (fileReqRef.current === myId) setFileLoading(false);
    }
  }, []);

  const cancelFileLoad = useCallback(() => {
    fileReqRef.current++; // invalidate the in-flight request; keep prior contents
    setFileLoading(false);
  }, []);

  const fetchSummary = useCallback(
    async (quiet: boolean) => {
      if (!repoPath || !target || !source) return;
      const params: DiffParams = {
        repoPath,
        target,
        source,
        compareMode,
        sourceScope,
        options: { ignoreWhitespace: hideWhitespace },
      };
      if (quiet) setRefreshing(true);
      else {
        setLoading(true);
        setSummary(null);
        setSelectedFile(null);
        setFileContents(null);
        setFileError(null);
      }
      setError(null);
      try {
        const result = await invoke<DiffSummary>("get_diff_summary", { params });
        setSummary(result);
        setLastParams(params);

        // Distinguish "truly no changes" from "all diffs were whitespace-only
        // and Hide whitespace dropped them" (design 6e) with a cheap probe.
        let wsHidden = false;
        if (result.files.length === 0 && hideWhitespace) {
          try {
            const probe = await invoke<DiffSummary>("get_diff_summary", {
              params: { ...params, options: { ignoreWhitespace: false } },
            });
            wsHidden = probe.files.length > 0;
          } catch {
            /* ignore probe failure */
          }
        }
        setWhitespaceOnlyHidden(wsHidden);

        // Reconcile the selection across the refresh (DESIGN.md 3.6): keep the
        // selected file if it survived (re-reading its contents against the new
        // params), otherwise drop back to the unselected placeholder.
        const prev = selectedRef.current;
        const still = prev ? result.files.find((f) => f.path === prev.path) : undefined;
        if (still) loadFileDiff(still, false, params);
        else {
          setSelectedFile(null);
          setFileContents(null);
          setFileError(null);
        }

        invoke<string>("get_repo_fingerprint", { params: { repoPath, target, source } })
          .then((fp) => {
            fingerprintRef.current = fp;
          })
          .catch(() => {});
      } catch (e) {
        setError(String(e));
        if (!quiet) setSummary(null);
      } finally {
        setRefreshing(false);
        setLoading(false);
      }
    },
    [repoPath, target, source, compareMode, sourceScope, hideWhitespace, loadFileDiff],
  );

  // Auto-fetch the summary whenever the comparison inputs change.
  const autoFetchKey = useRef("");
  useEffect(() => {
    if (!repoPath || !target || !source) return;
    const key = JSON.stringify({ repoPath, target, source, compareMode, sourceScope, hideWhitespace });
    if (autoFetchKey.current === key) return;
    autoFetchKey.current = key;
    fetchSummary(false);
  }, [repoPath, target, source, compareMode, sourceScope, hideWhitespace, fetchSummary]);

  // Window-focus stale check (DESIGN.md 3.6): compare the fingerprint and only
  // re-fetch (quietly) when it actually changed.
  useEffect(() => {
    function onFocus() {
      if (!repoPath || !target || !source || busyRef.current) return;
      invoke<string>("get_repo_fingerprint", { params: { repoPath, target, source } })
        .then((fp) => {
          if (fingerprintRef.current !== null && fp !== fingerprintRef.current) fetchSummary(true);
          fingerprintRef.current = fp;
        })
        .catch(() => {});
    }
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [repoPath, target, source, fetchSummary]);

  // ⌘R / Ctrl+R manual refresh (DESIGN.md 3.6).
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && (e.key === "r" || e.key === "R")) {
        e.preventDefault();
        if (!busyRef.current) fetchSummary(true);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fetchSummary]);

  // ↑/↓ to move through changed files (design 6f hint).
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      const s = summaryRef.current;
      const params = lastParamsRef.current;
      if (!s || s.files.length === 0 || !params) return;
      e.preventDefault();
      const files = s.files;
      const idx = files.findIndex((f) => f.path === selectedRef.current?.path);
      let next: number;
      if (idx < 0) next = e.key === "ArrowDown" ? 0 : files.length - 1;
      else if (e.key === "ArrowDown") next = Math.min(files.length - 1, idx + 1);
      else next = Math.max(0, idx - 1);
      loadFileDiff(files[next], false, params);
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [loadFileDiff]);

  // ── fullscreen / empty states ──────────────────────────────────────────
  const gitNotFound = isNotFound(projectError) || isNotFound(error);
  if (gitNotFound) {
    return (
      <GitNotFound
        detail={(isNotFound(projectError) ? projectError : error) ?? undefined}
        onRetry={() => {
          setProjectError(null);
          setError(null);
          if (lastAttemptRef.current) validateAndSetRepo(lastAttemptRef.current);
        }}
      />
    );
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

  // ── derived labels ─────────────────────────────────────────────────────
  const targetShort = shortName(target, branches);
  const sourceShort = shortName(source, branches);
  const baseColLabel =
    compareMode === "merge-base" ? `merge-base ${summary?.mergeBase ?? ""}`.trim() : `${targetShort} (tip)`;
  const headColLabel =
    effectiveScope === "committed"
      ? `${sourceShort} (tip)`
      : effectiveScope === "staged"
        ? "index (staged)"
        : "working tree";
  const statusLeft =
    (compareMode === "merge-base" ? `merge-base ${summary?.mergeBase ?? ""}`.trim() : "tips") +
    ` · ${targetShort}…${sourceShort}` +
    (hideWhitespace ? " · hide whitespace (-w)" : "");
  const statusRight = `${repoInfo ? gitVersionLabel(repoInfo.gitVersion) : "git"} · offline · read-only`;

  return (
    <div className="app-shell">
      <ControlBar
        repoPath={repoPath}
        recentProjects={recentProjects}
        onChoose={chooseRepository}
        onPickRecent={validateAndSetRepo}
        branches={branches}
        target={target}
        source={source}
        onTarget={(b) => setTarget(b.full)}
        onSource={(b) => setSource(b.full)}
        sourceScope={effectiveScope}
        onSourceScope={setSourceScope}
        compareMode={compareMode}
        onCompareMode={setCompareMode}
        hideWhitespace={hideWhitespace}
        onHideWhitespace={updateHideWhitespace}
        scopeLocked={scopeLocked}
        lockReason={lockReason}
        headText={sourceShort}
        theme={theme}
        onToggleTheme={() => setTheme((t) => (t === "light" ? "dark" : "light"))}
        refreshing={refreshing}
        onRefresh={() => !busyRef.current && fetchSummary(true)}
      />

      {summary && (
        <WarningBanners
          warnings={summary.warnings}
          mergeBase={summary.mergeBase}
          targetShort={targetShort}
          sourceShort={sourceShort}
          isDetached={!!repoInfo?.isDetached}
          hasCommits={repoInfo ? repoInfo.hasCommits : true}
          onCompareTips={() => setCompareMode("tips")}
        />
      )}

      {isTimeout(error) && !summary ? (
        <div className="main-area">
          <div className="diff-pane">
            <div className="diff-body">
              <TimeoutPlaceholder command={error ?? undefined} onRetry={() => fetchSummary(false)} />
            </div>
            <StatusBar left={statusLeft} right={statusRight} />
          </div>
        </div>
      ) : error && !summary ? (
        <div className="main-area">
          <div className="diff-pane">
            <div className="diff-body">
              <div className="pane-error">{error}</div>
            </div>
            <StatusBar left={statusLeft} right={statusRight} />
          </div>
        </div>
      ) : loading && !summary ? (
        <div className="main-area">
          <div className="diff-pane">
            <div className="diff-body">
              <div className="pane-placeholder">
                <span className="pane-spinner" aria-hidden="true" />
                <div className="pane-title">Loading diff…</div>
              </div>
            </div>
            <StatusBar left={statusLeft} right={statusRight} />
          </div>
        </div>
      ) : summary ? (
        <div className="main-area">
          <FileList
            summary={summary}
            selectedPath={selectedFile?.path ?? null}
            onSelect={(f) => lastParams && loadFileDiff(f, false, lastParams)}
            ariaLabel="Changed files"
          />
          <DiffPane
            summary={summary}
            selectedFile={selectedFile}
            contents={fileContents}
            fileError={fileError}
            fileLoading={fileLoading}
            timedOut={timedOut}
            theme={theme}
            wrap={wrap}
            onToggleWrap={() => setWrap((w) => !w)}
            hideWhitespace={hideWhitespace}
            whitespaceOnlyHidden={whitespaceOnlyHidden}
            compareMode={compareMode}
            targetShort={targetShort}
            sourceShort={sourceShort}
            baseColLabel={baseColLabel}
            headColLabel={headColLabel}
            statusLeft={statusLeft}
            statusRight={statusRight}
            bigSize={selectedFile ? bigSizeRef.current[selectedFile.path] : undefined}
            onLoadAnyway={() => selectedFile && lastParams && loadFileDiff(selectedFile, true, lastParams)}
            onCancel={cancelFileLoad}
            onRetryFile={() =>
              selectedFile && lastParams && loadFileDiff(selectedFile, !!fileContents?.isTooLarge, lastParams)
            }
            onShowWhitespace={() => updateHideWhitespace(false)}
            onTimeoutChange={setTimedOut}
          />
        </div>
      ) : null}
    </div>
  );
}

function StatusBar({ left, right }: { left: string; right: string }) {
  return (
    <div className="status-bar">
      <span>{left}</span>
      <div className="spacer" />
      <span>{right}</span>
    </div>
  );
}

function DiffPane(props: {
  summary: DiffSummary;
  selectedFile: DiffFile | null;
  contents: FileContents | null;
  fileError: string | null;
  fileLoading: boolean;
  timedOut: boolean;
  theme: Theme;
  wrap: boolean;
  onToggleWrap: () => void;
  hideWhitespace: boolean;
  whitespaceOnlyHidden: boolean;
  compareMode: CompareMode;
  targetShort: string;
  sourceShort: string;
  baseColLabel: string;
  headColLabel: string;
  statusLeft: string;
  statusRight: string;
  bigSize?: number;
  onLoadAnyway: () => void;
  onCancel: () => void;
  onRetryFile: () => void;
  onShowWhitespace: () => void;
  onTimeoutChange: (v: boolean) => void;
}) {
  const {
    summary,
    selectedFile,
    contents,
    fileError,
    fileLoading,
    timedOut,
    theme,
    wrap,
    onToggleWrap,
    hideWhitespace,
    whitespaceOnlyHidden,
    compareMode,
    targetShort,
    sourceShort,
    baseColLabel,
    headColLabel,
    statusLeft,
    statusRight,
    bigSize,
    onLoadAnyway,
    onCancel,
    onRetryFile,
    onShowWhitespace,
    onTimeoutChange,
  } = props;

  // Classify the pane content.
  type Kind =
    | "empty-nochanges"
    | "empty-ws"
    | "unselected"
    | "timeout"
    | "error"
    | "loading"
    | "submodule"
    | "symlink"
    | "large"
    | "binary"
    | "editor";
  let kind: Kind;
  if (!selectedFile) {
    if (summary.files.length === 0) {
      kind = hideWhitespace && whitespaceOnlyHidden ? "empty-ws" : "empty-nochanges";
    } else {
      kind = "unselected";
    }
  } else if (fileError) {
    kind = isTimeout(fileError) ? "timeout" : "error";
  } else if (fileLoading && !contents?.isTooLarge) {
    kind = "loading";
  } else if (contents?.note === "submodule") kind = "submodule";
  else if (contents?.note === "symlink") kind = "symlink";
  else if (contents?.isTooLarge) kind = fileLoading ? "loading" : "large";
  else if (contents?.isBinary) kind = "binary";
  else if (contents) kind = "editor";
  else kind = "unselected";

  const size = contents?.sizeBytes ?? bigSize;
  const showColumns = kind === "editor" || kind === "loading";
  const { dir, name } = selectedFile ? splitPath(selectedFile.path) : { dir: "", name: "" };

  return (
    <div className="diff-pane">
      {selectedFile && (
        <div className="diff-head">
          <span className={`st ${statusColorClass(selectedFile.status)}`}>{statusLetter(selectedFile.status)}</span>
          <span className="diff-head-path">
            {dir && <span className="dir">{dir}</span>}
            <span className="diff-head-name">{name}</span>
          </span>
          {size && <span className="badge badge-size">{(size / (1024 * 1024)).toFixed(1)} MB</span>}
          {selectedFile.additions != null && selectedFile.additions > 0 && (
            <span className="add mono">+{selectedFile.additions.toLocaleString("en-US")}</span>
          )}
          {selectedFile.deletions != null && selectedFile.deletions > 0 && (
            <span className="del mono">−{selectedFile.deletions.toLocaleString("en-US")}</span>
          )}
          <div className="spacer" />
          {kind === "editor" && (
            <button
              type="button"
              className={`pane-toggle${wrap ? " pane-toggle-on" : ""}`}
              onClick={onToggleWrap}
            >
              Wrap
            </button>
          )}
        </div>
      )}

      {kind === "editor" && timedOut && (
        <div className="banner banner-info">
          <span className="banner-info-glyph">i</span>
          Diff computation timed out (&gt;5 s) — showing files without change highlighting.
        </div>
      )}

      {showColumns && (
        <div className="col-labels">
          <div className="col-label col-label-base">Base · {baseColLabel}</div>
          <div className="col-label">Head · {headColLabel}</div>
        </div>
      )}

      <div className="diff-body">
        {kind === "unselected" && <UnselectedPlaceholder />}
        {kind === "empty-nochanges" && (
          <NoChangesPlaceholder
            sourceShort={sourceShort}
            targetShort={targetShort}
            tips={compareMode === "tips"}
          />
        )}
        {kind === "empty-ws" && <NoChangesWhitespacePlaceholder onShowWhitespace={onShowWhitespace} />}
        {kind === "timeout" && <TimeoutPlaceholder command={fileError ?? undefined} onRetry={onRetryFile} />}
        {kind === "error" && <div className="pane-error">{fileError}</div>}
        {kind === "loading" && selectedFile && (
          <LoadingPlaceholder path={selectedFile.path} sizeBytes={size} onCancel={onCancel} />
        )}
        {kind === "submodule" && contents && <SubmodulePlaceholder contents={contents} />}
        {kind === "symlink" && contents && <SymlinkPlaceholder contents={contents} />}
        {kind === "large" && <LargeFilePlaceholder sizeBytes={size ?? 0} onLoadAnyway={onLoadAnyway} />}
        {kind === "binary" && <BinaryPlaceholder />}
        {kind === "editor" && selectedFile && contents && (
          <MonacoDiffView
            path={selectedFile.path}
            original={contents.base ?? ""}
            modified={contents.head ?? ""}
            theme={theme}
            ignoreWhitespace={hideWhitespace}
            wrap={wrap}
            onTimeoutChange={onTimeoutChange}
          />
        )}
      </div>

      <StatusBar left={statusLeft} right={statusRight} />
    </div>
  );
}

const STATUS_LETTER: Record<string, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  typechange: "T",
  unmerged: "U",
  submodule: "S",
  other: "?",
};
function statusLetter(status: string): string {
  return STATUS_LETTER[status] ?? "?";
}
function statusColorClass(status: string): string {
  switch (status) {
    case "added":
      return "st-green";
    case "modified":
    case "typechange":
      return "st-yellow";
    case "deleted":
      return "st-red";
    case "renamed":
      return "st-purple";
    default:
      return "st-muted";
  }
}

export default App;
