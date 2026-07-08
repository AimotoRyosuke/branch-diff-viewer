import type { ReactNode } from "react";

/** One warning banner (docs/design "Branch Diff Viewer UI.dc.html" 6a): a
 * circular "!" glyph, bold lead-in, body, and an optional inline action. */
function WarningBanner({ title, children, action }: { title: string; children: ReactNode; action?: ReactNode }) {
  return (
    <div className="banner banner-warning">
      <span className="banner-glyph">!</span>
      <div className="banner-text">
        <span className="banner-title">{title}</span> — {children}
      </div>
      {action}
    </div>
  );
}

/** Maps backend `DiffSummary.warnings` strings (verbatim from
 * src-tauri/.../commands.rs) plus detached/unborn HEAD state to the design's
 * 6a banner wording. The HEAD-constraint ("not the checked-out branch")
 * warning is rendered separately as the control-bar lock banner and is
 * filtered out here to avoid duplication.
 *
 * Note: the design also lists a "Shallow clone" banner, but the backend never
 * emits a shallow warning (no `.git/shallow` probe), and Rust changes are out
 * of scope for this phase — so that banner is intentionally not shown. */
export function WarningBanners({
  warnings,
  mergeBase,
  targetShort,
  sourceShort,
  isDetached,
  hasCommits,
  onCompareTips,
}: {
  warnings: string[];
  mergeBase: string | null;
  targetShort: string;
  sourceShort: string;
  isDetached: boolean;
  hasCommits: boolean;
  onCompareTips: () => void;
}) {
  const banners: ReactNode[] = [];

  if (isDetached) {
    banners.push(
      <WarningBanner key="detached" title="Detached HEAD">
        no branch is checked out, so staged and unstaged scopes are unavailable. Scope is fixed to{" "}
        <strong>Committed only</strong>.
      </WarningBanner>,
    );
  }
  if (!hasCommits) {
    banners.push(
      <WarningBanner key="unborn" title="Unborn branch">
        the checked-out branch has no commits yet. Scope is fixed to <strong>Committed only</strong>.
      </WarningBanner>,
    );
  }

  warnings.forEach((w, i) => {
    if (w.includes("not the checked-out branch")) return; // shown as the lock banner
    if (w.includes("no merge base found")) {
      banners.push(
        <WarningBanner
          key={`w${i}`}
          title="No merge base"
          action={
            <button type="button" className="banner-action" onClick={onCompareTips}>
              Compare tips instead
            </button>
          }
        >
          <span className="mono">{targetShort}</span> and <span className="mono">{sourceShort}</span>{" "}
          have unrelated histories.
        </WarningBanner>,
      );
    } else if (w.includes("multiple merge bases")) {
      banners.push(
        <WarningBanner key={`w${i}`} title="Multiple merge bases found">
          using the first{mergeBase ? <>: <span className="mono">{mergeBase}</span></> : ""}. The diff
          may differ from what a merge would produce.
        </WarningBanner>,
      );
    } else if (w.includes("a rebase is in progress")) {
      banners.push(
        <WarningBanner key={`w${i}`} title="Rebase in progress">
          the working tree reflects an incomplete rebase. Results may change until it finishes or is
          aborted.
        </WarningBanner>,
      );
    } else if (w.includes("a merge is in progress")) {
      banners.push(
        <WarningBanner key={`w${i}`} title="Merge in progress">
          the working tree reflects an unfinished merge. Results may include unmerged entries until it
          is resolved or aborted.
        </WarningBanner>,
      );
    } else {
      // Unknown warning: surface the raw text rather than dropping it.
      banners.push(
        <div key={`w${i}`} className="banner banner-warning">
          <span className="banner-glyph">!</span>
          <div className="banner-text">{w}</div>
        </div>,
      );
    }
  });

  if (banners.length === 0) return null;
  return <div className="banner-stack">{banners}</div>;
}
