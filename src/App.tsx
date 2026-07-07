import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";
import type { DiffFile, DiffParams, DiffSummary } from "./types";

function App() {
  const [repoPath, setRepoPath] = useState("");
  const [target, setTarget] = useState("main");
  const [source, setSource] = useState("");
  const [summary, setSummary] = useState<DiffSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function showDiff() {
    setLoading(true);
    setError(null);
    setSummary(null);
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
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
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
              <FileRow key={f.path} file={f} />
            ))}
          </ul>
        </section>
      )}
    </main>
  );
}

function FileRow({ file }: { file: DiffFile }) {
  const label = STATUS_LABEL[file.status] ?? file.status;
  return (
    <li className="file-row">
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
    </li>
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
