import type { BranchList, BranchRef, CompareMode, SourceScope } from "./types";
import { BranchDropdown } from "./BranchDropdown";
import { ProjectChip } from "./ProjectChip";

type Theme = "light" | "dark";

interface SegOption<T extends string> {
  value: T;
  label: string;
  disabled?: boolean;
  title?: string;
}

function Segmented<T extends string>({
  value,
  onChange,
  options,
}: {
  value: T;
  onChange: (v: T) => void;
  options: SegOption<T>[];
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

/**
 * Top control bar (docs/design "Branch Diff Viewer Prototype.dc.html"): project
 * chip · BASE/HEAD branch pickers · Source-scope + Compare segments · Hide
 * whitespace · Refresh · theme toggle, with the HEAD-constraint lock banner
 * beneath (DESIGN.md 3.3).
 */
export function ControlBar({
  repoPath,
  recentProjects,
  onChoose,
  onPickRecent,
  branches,
  target,
  source,
  onTarget,
  onSource,
  sourceScope,
  onSourceScope,
  compareMode,
  onCompareMode,
  hideWhitespace,
  onHideWhitespace,
  scopeLocked,
  lockReason,
  headText,
  theme,
  onToggleTheme,
  refreshing,
  onRefresh,
}: {
  repoPath: string;
  recentProjects: string[];
  onChoose: () => void;
  onPickRecent: (path: string) => void;
  branches: BranchList | null;
  target: string;
  source: string;
  onTarget: (b: BranchRef) => void;
  onSource: (b: BranchRef) => void;
  sourceScope: SourceScope;
  onSourceScope: (s: SourceScope) => void;
  compareMode: CompareMode;
  onCompareMode: (m: CompareMode) => void;
  hideWhitespace: boolean;
  onHideWhitespace: (v: boolean) => void;
  scopeLocked: boolean;
  lockReason?: string;
  headText: string;
  theme: Theme;
  onToggleTheme: () => void;
  refreshing: boolean;
  onRefresh: () => void;
}) {
  return (
    <div className="control-bar">
      <div className="control-row">
        <ProjectChip
          repoPath={repoPath}
          recentProjects={recentProjects}
          onChoose={onChoose}
          onPickRecent={onPickRecent}
        />
        <div className="control-divider" />

        <BranchDropdown role="base" label="Base · target" branches={branches} value={target} onChange={onTarget} />
        <span className="control-arrow">←</span>
        <BranchDropdown role="head" label="Head · source" branches={branches} value={source} onChange={onSource} />

        <div className="control-divider" />

        <div className="control-group">
          <span className="control-label">Source scope</span>
          <Segmented<SourceScope>
            value={sourceScope}
            onChange={onSourceScope}
            options={[
              { value: "committed", label: "Committed" },
              { value: "staged", label: "Staged", disabled: scopeLocked, title: lockReason },
              { value: "unstaged", label: "Unstaged", disabled: scopeLocked, title: lockReason },
            ]}
          />
        </div>

        <div className="control-group">
          <span className="control-label">Compare</span>
          <Segmented<CompareMode>
            value={compareMode}
            onChange={onCompareMode}
            options={[
              { value: "merge-base", label: "merge-base" },
              { value: "tips", label: "tips" },
            ]}
          />
        </div>

        <label className="ws-toggle" onClick={() => onHideWhitespace(!hideWhitespace)}>
          <span className={`ws-box${hideWhitespace ? " ws-box-on" : ""}`}>{hideWhitespace ? "✓" : ""}</span>
          Hide whitespace
        </label>

        <div className="spacer" />

        <button type="button" className="refresh-btn" onClick={onRefresh} disabled={refreshing} title="Refresh (⌘R)">
          {refreshing ? (
            <>
              <span className="refresh-spinner" aria-hidden="true" />
              Refreshing…
            </>
          ) : (
            <>
              <span className="refresh-glyph" aria-hidden="true">
                ⟳
              </span>
              Refresh
              <span className="refresh-kbd">⌘R</span>
            </>
          )}
        </button>

        <button type="button" className="theme-btn" onClick={onToggleTheme}>
          <span className="theme-swatch" aria-hidden="true" />
          {theme === "light" ? "Dark" : "Light"}
        </button>
      </div>

      {scopeLocked && (
        <div className="banner banner-warning lock-banner">
          <span className="banner-glyph">!</span>
          <div className="banner-text">
            <span className="mono">{headText}</span> is not the checked-out branch — staged and
            unstaged changes exist only in the working tree of HEAD. Scope is fixed to{" "}
            <strong>Committed only</strong>.
          </div>
        </div>
      )}
    </div>
  );
}
