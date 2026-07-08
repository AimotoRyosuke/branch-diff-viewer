import type { ReactNode } from "react";
import type { FileContents } from "./types";
import { formatBytes } from "./utils";

/** Shared centered right-pane placeholder frame (docs/design "Branch Diff
 * Viewer UI.dc.html" states 6b / 6c / 6e / 6f). Every special/empty state
 * fills the diff pane with a circular icon, a bold title, an optional body,
 * and optional actions — matching the design's card interior. */
function PaneMessage({
  variant = "neutral",
  icon,
  title,
  titleMuted,
  children,
  actions,
}: {
  variant?: "neutral" | "warning" | "error" | "success";
  icon: ReactNode;
  title: string;
  titleMuted?: boolean;
  children?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="pane-placeholder">
      <div className={`pane-icon pane-icon-${variant}`}>{icon}</div>
      <div className={`pane-title${titleMuted ? " pane-title-muted" : ""}`}>{title}</div>
      {children}
      {actions && <div className="pane-actions">{actions}</div>}
    </div>
  );
}

/** 6f — no file selected yet. */
export function UnselectedPlaceholder() {
  return (
    <PaneMessage
      variant="neutral"
      titleMuted
      icon={
        <span className="pane-cursor-mark" aria-hidden="true">
          <span className="pane-cursor-bar" />
          <span className="pane-cursor-bar pane-cursor-bar-active" />
        </span>
      }
      title="Select a file to view its diff"
    >
      <div className="pane-kbd-hint">
        <span className="kbd">↑</span>
        <span className="kbd">↓</span>
        to move through changed files
      </div>
    </PaneMessage>
  );
}

/** 6b — binary file. Backend does not surface per-side sizes for binaries,
 * so the size line from the mock is intentionally omitted. */
export function BinaryPlaceholder() {
  return (
    <PaneMessage
      variant="neutral"
      icon={<span className="pane-icon-mono">01</span>}
      title="Binary file"
    >
      <div className="pane-body">Binary file — cannot display diff.</div>
    </PaneMessage>
  );
}

/** Pulls the "Subproject commit <sha>" SHA out of the synthesized submodule
 * blob content the backend returns (see resolve_side_content). */
function subprojectSha(text: string | null): string | null {
  if (!text) return null;
  const m = text.match(/Subproject commit\s+(\S+)/);
  return m ? m[1] : text.trim() || null;
}

/** 6b — submodule commit changed. */
export function SubmodulePlaceholder({ contents }: { contents: FileContents }) {
  const base = subprojectSha(contents.base);
  const head = subprojectSha(contents.head);
  return (
    <PaneMessage variant="neutral" icon={<span className="pane-glyph">▣</span>} title="Submodule commit changed">
      <div className="pane-diff-lines">
        {base !== null && <div className="pane-diff-del">− Subproject commit {base}</div>}
        {head !== null && <div className="pane-diff-add">+ Subproject commit {head}</div>}
      </div>
      <div className="pane-note">Contents of the submodule are not compared.</div>
    </PaneMessage>
  );
}

/** 6b — symlink target changed. base/head hold the raw link-target text. */
export function SymlinkPlaceholder({ contents }: { contents: FileContents }) {
  const base = contents.base?.trim() ?? null;
  const head = contents.head?.trim() ?? null;
  return (
    <PaneMessage variant="neutral" icon={<span className="pane-glyph">↪</span>} title="Symlink target changed">
      <div className="pane-diff-lines">
        {base !== null && <div className="pane-diff-del">− {base}</div>}
        {head !== null && <div className="pane-diff-add">+ {head}</div>}
      </div>
      <div className="pane-note">A symlink's content is the single line of its target path.</div>
    </PaneMessage>
  );
}

/** 3d / 5d — large-file guard. */
export function LargeFilePlaceholder({
  sizeBytes,
  onLoadAnyway,
}: {
  sizeBytes: number;
  onLoadAnyway: () => void;
}) {
  return (
    <PaneMessage
      variant="warning"
      icon={<span className="pane-bang">!</span>}
      title={`Large file (${formatBytes(sizeBytes)})`}
      actions={
        <button type="button" className="pane-btn" onClick={onLoadAnyway}>
          Load anyway
        </button>
      }
    >
      <div className="pane-body">Diff is not loaded automatically for files over 1 MB.</div>
    </PaneMessage>
  );
}

/** 4a / 5e — loading both sides, cancellable. */
export function LoadingPlaceholder({
  path,
  sizeBytes,
  onCancel,
}: {
  path: string;
  sizeBytes?: number;
  onCancel: () => void;
}) {
  return (
    <PaneMessage
      variant="neutral"
      icon={<span className="pane-spinner" aria-hidden="true" />}
      title="Loading diff…"
      actions={
        <button type="button" className="pane-btn pane-btn-muted" onClick={onCancel}>
          Cancel
        </button>
      }
    >
      <div className="pane-body">
        Reading both sides of <span className="mono">{path}</span>
        {sizeBytes ? ` (${formatBytes(sizeBytes)})` : ""}
      </div>
    </PaneMessage>
  );
}

/** 6c (right) — git command timed out (>30 s), with Retry. Used both for a
 * summary-level and a file-level GIT_TIMEOUT. */
export function TimeoutPlaceholder({
  command,
  onRetry,
}: {
  command?: string;
  onRetry: () => void;
}) {
  return (
    <PaneMessage
      variant="error"
      icon={<span className="pane-cross">✕</span>}
      title="Git command timed out"
      actions={
        <button type="button" className="pane-btn pane-btn-primary" onClick={onRetry}>
          Retry
        </button>
      }
    >
      <div className="pane-body">
        The command did not finish within 30 seconds. The repository may be on a slow network
        volume, or locked by another process.
      </div>
      {command && <div className="pane-command mono">{command}</div>}
    </PaneMessage>
  );
}

/** 6e (left) — no differences at all. */
export function NoChangesPlaceholder({
  sourceShort,
  targetShort,
  tips,
}: {
  sourceShort: string;
  targetShort: string;
  tips: boolean;
}) {
  return (
    <PaneMessage variant="success" icon={<span className="pane-check">✓</span>} title="No changes">
      <div className="pane-body">
        <span className="mono">{sourceShort}</span>{" "}
        {tips ? (
          <>
            has no differences from <span className="mono">{targetShort}</span>.
          </>
        ) : (
          <>
            has no differences from the merge base with <span className="mono">{targetShort}</span>.
          </>
        )}
      </div>
    </PaneMessage>
  );
}

/** 6e (right) — everything left was whitespace-only and Hide whitespace
 * dropped it; the button turns Hide whitespace OFF. */
export function NoChangesWhitespacePlaceholder({ onShowWhitespace }: { onShowWhitespace: () => void }) {
  return (
    <PaneMessage
      variant="success"
      icon={<span className="pane-check">✓</span>}
      title="No changes"
      actions={
        <button type="button" className="pane-btn pane-btn-link" onClick={onShowWhitespace}>
          Show whitespace-only changes
        </button>
      }
    >
      <div className="pane-body">
        All differences are whitespace-only and hidden by <strong>Hide whitespace</strong>.
      </div>
    </PaneMessage>
  );
}

/** 6c (left) — full-screen GIT_NOT_FOUND detection error. */
export function GitNotFound({ detail, onRetry }: { detail?: string; onRetry: () => void }) {
  return (
    <main className="container empty-state-container">
      <div className="empty-state">
        <div className="empty-state-mark">
          <span className="empty-state-dot empty-state-dot-a" />
          <span className="empty-state-dash" />
          <span className="empty-state-dot empty-state-dot-b" />
        </div>
        <h1 className="empty-state-title">Branch Diff Viewer</h1>
        <div className="empty-state-error">
          <span className="empty-state-error-icon">✕</span>
          <div>
            <div className="empty-state-error-title">Git not found</div>
            <div className="empty-state-error-detail">
              No <span className="mono">git</span> executable was found on your PATH. Install the
              Xcode Command Line Tools (<span className="mono">xcode-select --install</span>) or
              Homebrew git, then retry.
              {detail && <div className="pane-command mono">{detail}</div>}
            </div>
          </div>
        </div>
        <button type="button" className="empty-state-choose" onClick={onRetry}>
          Retry detection
        </button>
      </div>
    </main>
  );
}
