import { useCallback, useState } from "react";
import { useClickOutside } from "./useClickOutside";
import { projectName, projectParentPath } from "./utils";

/** Control-bar project chip (docs/design "Branch Diff Viewer Prototype.dc.html"
 * control bar). Clicking it opens a small menu to switch projects: choose a
 * new folder, or jump to a recent one (DESIGN.md 3.1). */
export function ProjectChip({
  repoPath,
  recentProjects,
  onChoose,
  onPickRecent,
}: {
  repoPath: string;
  recentProjects: string[];
  onChoose: () => void;
  onPickRecent: (path: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const containerRef = useClickOutside<HTMLDivElement>(
    open,
    useCallback(() => setOpen(false), []),
  );

  const otherRecent = recentProjects.filter((p) => p !== repoPath);

  return (
    <div className="project-chip" ref={containerRef}>
      <button type="button" className="project-chip-trigger" onClick={() => setOpen((o) => !o)}>
        <span className="project-chip-icon" aria-hidden="true" />
        <span className="project-chip-name">{projectName(repoPath)}</span>
        <span className="project-chip-path" title={repoPath}>
          {projectParentPath(repoPath)}
        </span>
        <span className="project-chip-arrow">▼</span>
      </button>

      {open && (
        <div className="project-chip-panel">
          <button
            type="button"
            className="project-chip-item project-chip-item-primary"
            onClick={() => {
              setOpen(false);
              onChoose();
            }}
          >
            Choose another folder…
          </button>
          {otherRecent.length > 0 && (
            <>
              <div className="project-chip-section-label">Recent</div>
              {otherRecent.map((path) => (
                <button
                  type="button"
                  key={path}
                  className="project-chip-item"
                  onClick={() => {
                    setOpen(false);
                    onPickRecent(path);
                  }}
                >
                  <span className="project-chip-item-name">{projectName(path)}</span>
                  <span className="project-chip-item-path" title={path}>
                    {projectParentPath(path, 36)}
                  </span>
                </button>
              ))}
            </>
          )}
        </div>
      )}
    </div>
  );
}
