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

/** Shared canvas context for `measureText` (avoids re-creating a canvas per
 * call; the file list re-fits every visible row on resize). */
let measureCtx: CanvasRenderingContext2D | null = null;

/** Pixel width of `text` rendered in the CSS shorthand `font`. */
export function measureText(text: string, font: string): number {
  if (!measureCtx) measureCtx = document.createElement("canvas").getContext("2d");
  if (!measureCtx) return 0;
  measureCtx.font = font;
  return measureCtx.measureText(text).width;
}

const ELL = "…";

/**
 * Fits `dir + name` (as produced by `splitPath`) into `maxWidth` px by
 * omitting, in priority order:
 *   1. middle directory segments   →  a/b/…/z/name
 *   2. the directory head          →  …ripts/name
 *   3. the tail of the file name   →  …/long_file_na…
 * so the file name stays fully visible until the directory alone can no
 * longer absorb the overflow.
 */
export function fitPath(
  dir: string,
  name: string,
  maxWidth: number,
  font: string,
): { dir: string; name: string } {
  const w = (s: string) => measureText(s, font);
  if (w(dir + name) <= maxWidth) return { dir, name };

  const nameW = w(name);

  if (dir) {
    // 1: keep the first `keep` segments and the last one, omit the middle.
    const segs = dir.slice(0, -1).split("/");
    const last = segs[segs.length - 1];
    for (let keep = segs.length - 2; keep >= 1; keep--) {
      const d = `${segs.slice(0, keep).join("/")}/${ELL}/${last}/`;
      if (w(d) + nameW <= maxWidth) return { dir: d, name };
    }

    // 2: omit the directory head, keeping the longest tail that still fits
    // ("…/" at minimum, mid-segment cuts like "…ripts/" allowed).
    if (w(ELL + "/") + nameW <= maxWidth) {
      let lo = 1;
      let hi = dir.length;
      while (lo < hi) {
        const mid = Math.ceil((lo + hi) / 2);
        if (w(ELL + dir.slice(dir.length - mid)) + nameW <= maxWidth) lo = mid;
        else hi = mid - 1;
      }
      return { dir: ELL + dir.slice(dir.length - lo), name };
    }

    // The full name still fits with the dir dropped entirely — prefer that
    // over cutting into the name.
    if (nameW <= maxWidth) return { dir: "", name };
  }

  // 3: omit the tail of the file name itself.
  let dirPart = dir ? ELL + "/" : "";
  let budget = maxWidth - w(dirPart + ELL);
  if (budget <= 0) {
    dirPart = "";
    budget = maxWidth - w(ELL);
  }
  let lo = 0;
  let hi = name.length;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (w(name.slice(0, mid)) <= budget) lo = mid;
    else hi = mid - 1;
  }
  if (lo === 0) return { dir: "", name: ELL };
  return { dir: dirPart, name: name.slice(0, lo) + ELL };
}

/** Last path segment, used as the display name for a project chip (e.g.
 * "myapp" from "/Users/x/dev/myapp"). */
export function projectName(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, "");
  const parts = trimmed.split(/[/\\]/);
  return parts[parts.length - 1] || trimmed;
}

/**
 * Parent directory of a project path, for display next to the project name
 * (which already shows the last segment — repeating it would be redundant):
 * "/Users/x/develop/lincwell/llm-customer-support" → "~/develop/lincwell/".
 * The home prefix is collapsed to "~", and when the result exceeds
 * `maxChars` the middle segments are omitted ("~/…/lincwell/"), keeping the
 * head and the immediate parent visible.
 */
export function projectParentPath(path: string, maxChars = 40): string {
  let p = path.replace(/\\/g, "/").replace(/\/+$/, "");
  const idx = p.lastIndexOf("/");
  if (idx < 0) return "";
  p = p.slice(0, idx + 1); // parent, trailing slash kept
  p = p.replace(/^\/(?:Users|home)\/[^/]+(?=\/)/, "~");
  if (p.length <= maxChars) return p;

  const segs = p.slice(0, -1).split("/");
  const last = segs[segs.length - 1];
  for (let keep = segs.length - 2; keep >= 1; keep--) {
    const cand = `${segs.slice(0, keep).join("/")}/…/${last}/`;
    if (cand.length <= maxChars) return cand;
  }
  const cand = `…/${last}/`;
  if (cand.length <= maxChars) return cand;
  return cand.slice(0, maxChars - 1) + "…";
}
