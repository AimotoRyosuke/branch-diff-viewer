import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";
import type { DiffFile, DiffParams, DiffSummary, FileContents } from "./types";

function App() {
  const [repoPath, setRepoPath] = useState("");
  const [target, setTarget] = useState("main");
  const [source, setSource] = useState("");
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
        compareMode: "merge-base",
        sourceScope: "committed",
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
      <h1>Branch Diff Viewer</h1>

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
        </label>
        <button type="submit" disabled={loading}>
          {loading ? "Loading…" : "Show diff"}
        </button>
      </form>

      {error && <p className="error">{error}</p>}

      {summary && (
        <section className="results">
          {summary.warnings.length > 0 && (
            <ul className="warnings">
              {summary.warnings.map((w, i) => (
                <li key={i}>{w}</li>
              ))}
            </ul>
          )}
          <p className="totals">
            {summary.summary.filesChanged} files changed, +{summary.summary.additions} -
            {summary.summary.deletions}
            {typeof summary.omittedUntracked === "number" && summary.omittedUntracked > 0
              ? ` (+${summary.omittedUntracked} more untracked)`
              : ""}
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
          </ul>

          <FileDiffPane
            file={selectedFile}
            contents={fileContents}
            loading={fileLoading}
            error={fileError}
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

function FileDiffPane({
  file,
  contents,
  loading,
  error,
  onLoadAnyway,
}: {
  file: DiffFile | null;
  contents: FileContents | null;
  loading: boolean;
  error: string | null;
  onLoadAnyway: () => void;
}) {
  if (!file) return null;

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

      {!loading && !error && contents && !contents.isTooLarge && !contents.isBinary && (
        <div className="file-diff-columns">
          <pre className="file-diff-pre">
            <div className="file-diff-pre-label">base</div>
            {contents.base ?? "(no content — added file)"}
          </pre>
          <pre className="file-diff-pre">
            <div className="file-diff-pre-label">head</div>
            {contents.head ?? "(no content — deleted file)"}
          </pre>
        </div>
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
