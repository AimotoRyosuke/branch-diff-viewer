import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { DiffFile, DiffSummary } from "./types";
import { splitPath } from "./utils";

const ROW_H = 31; // fixed row height — rows are single-line (ellipsis), so this holds
const OVERSCAN = 8;

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

/** Design status color classes (docs/design 2a `ST_COLOR`): A green, M
 * yellow, D red, R purple; anything else muted. */
function statusClass(status: string): string {
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

/**
 * Left-hand changed-files list (docs/design Prototype file list + UI 3c).
 * Self-windowed (no external dep): only the rows intersecting the viewport
 * are mounted, positioned absolutely inside a full-height spacer. Scroll
 * position is owned by the scroll element, so a quiet Refresh that swaps
 * `summary` without remounting this component preserves it (DESIGN.md 3.6).
 */
export function FileList({
  summary,
  selectedPath,
  onSelect,
  ariaLabel,
}: {
  summary: DiffSummary;
  selectedPath: string | null;
  onSelect: (file: DiffFile) => void;
  ariaLabel?: string;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(400);

  const files = summary.files;
  const total = files.length;

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    setViewport(el.clientHeight);
    const ro = new ResizeObserver(() => setViewport(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Keep the selected row visible (arrow-key navigation in the parent moves
  // the selection; nudge scrollTop so the row isn't clipped).
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !selectedPath) return;
    const idx = files.findIndex((f) => f.path === selectedPath);
    if (idx < 0) return;
    const top = idx * ROW_H;
    const bottom = top + ROW_H;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (bottom > el.scrollTop + el.clientHeight) el.scrollTop = bottom - el.clientHeight;
  }, [selectedPath, files]);

  const start = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
  const end = Math.min(total, Math.ceil((scrollTop + viewport) / ROW_H) + OVERSCAN);
  const visible = files.slice(start, end);

  const { filesChanged, additions, deletions } = summary.summary;
  const omitted = summary.omittedUntracked ?? 0;

  return (
    <div className="file-panel">
      <div className="file-panel-head">
        Changed files <span className="file-panel-count">{total}</span>
        <div className="spacer" />
        <span className="file-panel-navhint">↑↓ to navigate</span>
      </div>

      <div
        className="file-scroll"
        ref={scrollRef}
        role="listbox"
        aria-label={ariaLabel}
        onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
      >
        <div className="file-spacer" style={{ height: total * ROW_H }}>
          {visible.map((f, i) => {
            const index = start + i;
            const selected = f.path === selectedPath;
            const { dir, name } = splitPath(f.path);
            const oldName = f.oldPath ? splitPath(f.oldPath).name : null;
            return (
              <div
                key={f.path}
                role="option"
                aria-selected={selected}
                className={`file-row2${selected ? " file-row2-sel" : ""}`}
                style={{ top: index * ROW_H, height: ROW_H }}
                onClick={() => onSelect(f)}
              >
                <span className={`st ${statusClass(f.status)}`}>
                  {STATUS_LETTER[f.status] ?? "?"}
                  {f.isUntracked && <span className="st-untracked-q">?</span>}
                </span>
                <span className={`file-name${f.status === "deleted" ? " file-name-del" : ""}`}>
                  {dir && <span className="dir">{dir}</span>}
                  {name}
                  {oldName && <span className="dir"> ← {oldName}</span>}
                </span>
                {f.isUntracked ? (
                  <span className="badge badge-untracked">untracked</span>
                ) : (
                  <>
                    {f.additions != null && f.additions > 0 && (
                      <span className="add">+{f.additions.toLocaleString("en-US")}</span>
                    )}
                    {f.deletions != null && f.deletions > 0 && (
                      <span className="del">−{f.deletions.toLocaleString("en-US")}</span>
                    )}
                    {f.isBinary && <span className="binary">binary</span>}
                  </>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {omitted > 0 && (
        <div className="file-more">
          <span className="file-more-chevron">▸</span> +{omitted} more (untracked)
          <div className="spacer" />
          <span className="file-more-note">showing first 100</span>
        </div>
      )}

      <div className="file-foot">
        <span className="muted">{filesChanged} files</span>
        <span className="add">+{additions.toLocaleString("en-US")}</span>
        <span className="del">−{deletions.toLocaleString("en-US")}</span>
      </div>
    </div>
  );
}
