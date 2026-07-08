/** Formats an ISO 8601 timestamp as a short relative-time string (e.g. "2h
 * ago", "just now"), for the "last fetch" annotation on remote-tracking
 * branches (DESIGN.md 3.2 / docs/design UI 3b). Falls back to the raw
 * timestamp if it doesn't parse. */
export function formatRelativeTime(iso: string): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return iso;

  const diffMs = Date.now() - then;
  const diffSec = Math.round(diffMs / 1000);
  if (diffSec < 0) return "just now";
  if (diffSec < 60) return "just now";

  const diffMin = Math.round(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;

  const diffHour = Math.round(diffMin / 60);
  if (diffHour < 24) return `${diffHour}h ago`;

  const diffDay = Math.round(diffHour / 24);
  if (diffDay < 30) return `${diffDay}d ago`;

  const diffMonth = Math.round(diffDay / 30);
  if (diffMonth < 12) return `${diffMonth}mo ago`;

  const diffYear = Math.round(diffMonth / 12);
  return `${diffYear}y ago`;
}

export function formatBytes(n: number): string {
  return `${(n / (1024 * 1024)).toFixed(2)} MB`;
}

/** Compact relative-time (e.g. "2h", "3d") with no "ago" suffix, for the
 * `remote · 2h` badge on a remote-tracking branch trigger (docs/design
 * Prototype control bar). Returns "" for a null/unparseable timestamp. */
export function formatRelativeShort(iso: string | null): string {
  if (!iso) return "";
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";
  const sec = Math.max(0, Math.round((Date.now() - then) / 1000));
  if (sec < 60) return "now";
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}m`;
  const hour = Math.round(min / 60);
  if (hour < 24) return `${hour}h`;
  const day = Math.round(hour / 24);
  if (day < 30) return `${day}d`;
  const month = Math.round(day / 30);
  if (month < 12) return `${month}mo`;
  return `${Math.round(month / 12)}y`;
}

/** Normalizes `RepoInfo.gitVersion` (which may be "git version 2.45.2" or a
 * bare "2.45.2") to the status-bar form "git 2.45.2" (docs/design Prototype
 * status bar). */
export function gitVersionLabel(raw: string): string {
  const m = raw.match(/(\d+\.\d+(?:\.\d+)*)/);
  return `git ${m ? m[1] : raw.replace(/^git\s+version\s+/i, "").trim()}`;
}

/** Splits a path into its directory prefix (with trailing slash) and file
 * name, for the "dimmed dir + bold name" file-list / diff-header rendering. */
export function splitPath(path: string): { dir: string; name: string } {
  const idx = path.lastIndexOf("/");
  if (idx < 0) return { dir: "", name: path };
  return { dir: path.slice(0, idx + 1), name: path.slice(idx + 1) };
}

/** Last path segment, used as the display name for a project chip (e.g.
 * "myapp" from "/Users/x/dev/myapp"). */
export function projectName(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, "");
  const parts = trimmed.split(/[/\\]/);
  return parts[parts.length - 1] || trimmed;
}
