import { projectName } from "./utils";

/** First-launch / no-project-selected screen (docs/design "Branch Diff
 * Viewer UI.dc.html" state 1c), plus its error variant (state 3a) when the
 * most recently chosen folder failed `validate_repo`. */
export function EmptyState({
  error,
  recentProjects,
  onChoose,
  onPickRecent,
}: {
  error: string | null;
  recentProjects: string[];
  onChoose: () => void;
  onPickRecent: (path: string) => void;
}) {
  return (
    <div className="empty-state">
      <div className="empty-state-mark">
        <span className="empty-state-dot empty-state-dot-a" />
        <span className="empty-state-dash" />
        <span className="empty-state-dot empty-state-dot-b" />
      </div>
      <h1 className="empty-state-title">Branch Diff Viewer</h1>

      {error ? (
        <div className="empty-state-error">
          <span className="empty-state-error-icon">✕</span>
          <div>
            <div className="empty-state-error-title">Not a Git repository</div>
            <div className="empty-state-error-detail">{error}</div>
          </div>
        </div>
      ) : (
        <p className="empty-state-subtitle">
          Compare a source branch against its merge target — including staged, unstaged and
          untracked changes.
        </p>
      )}

      <button type="button" className="empty-state-choose" onClick={onChoose}>
        {error ? "Choose another folder…" : "Choose repository…"}
      </button>

      {recentProjects.length > 0 && (
        <div className="empty-state-recent">
          <div className="empty-state-recent-label">Recent</div>
          <div className="empty-state-recent-list">
            {recentProjects.map((path) => (
              <button
                type="button"
                key={path}
                className="empty-state-recent-item"
                onClick={() => onPickRecent(path)}
              >
                <span className="empty-state-recent-name">{projectName(path)}</span>
                <span className="empty-state-recent-path">{path}</span>
              </button>
            ))}
          </div>
        </div>
      )}

      <p className="empty-state-footer">Fully offline · never modifies your repository</p>
    </div>
  );
}
