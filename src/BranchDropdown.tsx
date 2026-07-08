import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { BranchList, BranchRef } from "./types";
import { useClickOutside } from "./useClickOutside";
import { formatRelativeShort, formatRelativeTime } from "./utils";

/** Branch picker used for both Base (target) and Head (source) — DESIGN.md
 * 3.2 / docs/design "Branch Diff Viewer UI.dc.html" state 3b: Local /
 * Remote-tracking grouped, filterable, current-HEAD badge, and a "last
 * fetch" annotation on the remote-tracking group (this app never fetches on
 * its own). */
export function BranchDropdown({
  label,
  branches,
  value,
  onChange,
  role,
}: {
  label: string;
  branches: BranchList | null;
  /** Currently selected branch's fully-qualified ref, or "" if none. */
  value: string;
  onChange: (branch: BranchRef) => void;
  /** Which control-bar slot this is: "base" shows a remote selection as a
   * `remote · <last-fetch>` badge, "head" shows `HEAD` (blue) for the
   * checked-out branch or `Remote` (yellow) otherwise (docs/design Prototype
   * control bar). */
  role: "base" | "head";
}) {
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState("");
  const filterInputRef = useRef<HTMLInputElement>(null);
  const containerRef = useClickOutside<HTMLDivElement>(
    open,
    useCallback(() => setOpen(false), []),
  );

  useEffect(() => {
    if (open) {
      setFilter("");
      filterInputRef.current?.focus();
    }
  }, [open]);

  const selected = useMemo(() => {
    if (!branches) return null;
    return [...branches.local, ...branches.remote].find((b) => b.full === value) ?? null;
  }, [branches, value]);

  const filterLower = filter.trim().toLowerCase();
  const localMatches = (branches?.local ?? []).filter((b) =>
    b.short.toLowerCase().includes(filterLower),
  );
  const remoteMatches = (branches?.remote ?? []).filter((b) =>
    b.short.toLowerCase().includes(filterLower),
  );

  const displayText = selected ? selected.short : value || "(select branch)";
  const isCurrentHead = (b: BranchRef) => !b.isRemote && branches?.current === b.short;

  return (
    <div className="branch-dropdown" ref={containerRef}>
      <span className="segmented-label">{label}</span>
      <button
        type="button"
        className={`branch-dropdown-trigger${open ? " branch-dropdown-trigger-open" : ""}`}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="branch-dropdown-value">{displayText}</span>
        {selected && isCurrentHead(selected) && <span className="badge badge-head">HEAD</span>}
        {selected?.isRemote &&
          (role === "base" ? (
            <span className="badge badge-remote">
              remote{formatRelativeShort(branches?.lastFetch ?? null) && ` · ${formatRelativeShort(branches?.lastFetch ?? null)}`}
            </span>
          ) : (
            <span className="badge badge-remote">Remote</span>
          ))}
        <span className="branch-dropdown-arrow">{open ? "▲" : "▼"}</span>
      </button>

      {open && (
        <div className="branch-dropdown-panel">
          <div className="branch-dropdown-filter-row">
            <input
              ref={filterInputRef}
              type="text"
              className="branch-dropdown-filter"
              placeholder="Filter branches…"
              value={filter}
              onChange={(e) => setFilter(e.currentTarget.value)}
            />
          </div>

          {!branches && <div className="branch-dropdown-empty">Loading branches…</div>}

          {branches && (
            <>
              <div className="branch-dropdown-section-label">Local</div>
              <div className="branch-dropdown-list">
                {localMatches.length === 0 && (
                  <div className="branch-dropdown-empty">No matching branches</div>
                )}
                {localMatches.map((b) => (
                  <BranchRow
                    key={b.full}
                    branch={b}
                    isSelected={b.full === value}
                    isHead={isCurrentHead(b)}
                    onClick={() => {
                      onChange(b);
                      setOpen(false);
                    }}
                  />
                ))}
              </div>

              <div className="branch-dropdown-section-row">
                <span className="branch-dropdown-section-label">Remote-tracking</span>
                {branches.lastFetch && (
                  <span className="branch-dropdown-last-fetch">
                    last fetch {formatRelativeTime(branches.lastFetch)}
                  </span>
                )}
              </div>
              <div className="branch-dropdown-list">
                {remoteMatches.length === 0 && (
                  <div className="branch-dropdown-empty">No matching branches</div>
                )}
                {remoteMatches.map((b) => (
                  <BranchRow
                    key={b.full}
                    branch={b}
                    isSelected={b.full === value}
                    isHead={false}
                    onClick={() => {
                      onChange(b);
                      setOpen(false);
                    }}
                  />
                ))}
              </div>
              <div className="branch-dropdown-footer">
                Remote-tracking refs reflect the last fetch — this app never fetches. Selecting one
                fixes scope to <strong>Committed only</strong>.
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}

function BranchRow({
  branch,
  isSelected,
  isHead,
  onClick,
}: {
  branch: BranchRef;
  isSelected: boolean;
  isHead: boolean;
  onClick: () => void;
}) {
  return (
    <button type="button" className="branch-dropdown-row" onClick={onClick}>
      <span className="branch-dropdown-check">{isSelected ? "✓" : ""}</span>
      <span className="branch-dropdown-row-name">{branch.short}</span>
      {isHead && <span className="badge badge-head">HEAD</span>}
    </button>
  );
}
