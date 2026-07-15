// Types shared with the Rust backend over Tauri IPC.
// Mirrors DESIGN.md chapter 5 verbatim (camelCase on the wire).

// Every IPC command rejects with a plain string (Tauri's `Result<T, String>`
// convention). Two error conditions carry a machine-checkable prefix so the
// frontend can special-case them (e.g. show a "Retry" button for a timeout,
// or "please install git" for a missing binary) instead of just displaying
// the raw text (Phase 6a / DESIGN.md 7):
//   - "GIT_TIMEOUT: ..."   — the git invocation exceeded 30s and was killed.
//   - "GIT_NOT_FOUND: ..." — the `git` executable could not be found on PATH.
// Any other error string has no reserved prefix and should just be shown
// as-is. Check with `err.startsWith("GIT_TIMEOUT:")` /
// `err.startsWith("GIT_NOT_FOUND:")`.

// Compare mode is named by meaning rather than "two-dot"/"three-dot" — those
// terms flip meaning between git and log and are avoided here (DESIGN.md 5 L-1).
export type CompareMode = "merge-base" | "tips"; // default "merge-base"
export type SourceScope = "committed" | "staged" | "unstaged"; // default "committed"

export interface RepoInfo {
  toplevel: string; // normalized root
  currentBranch: string | null; // null when detached/unborn
  isDetached: boolean;
  hasCommits: boolean; // false for an unborn branch
  gitVersion: string;
}

export interface BranchRef {
  short: string; // display name (e.g. origin/main)
  full: string; // fully-qualified (refs/heads/... / refs/remotes/...)
  isRemote: boolean;
}

export interface BranchList {
  local: BranchRef[];
  remote: BranchRef[];
  current: string | null; // short name of the checked-out branch; null when detached/unborn
  lastFetch: string | null; // ISO 8601 mtime of .git/FETCH_HEAD; null if never fetched
}

export interface DiffParams {
  repoPath: string;
  target: string; // merge target (short or full; normalized internally)
  source: string; // merge source
  compareMode: CompareMode;
  sourceScope: SourceScope;
  options: {
    ignoreWhitespace?: boolean; // default true (Hide whitespace ON)
  };
}

// Input to get_repo_fingerprint (DESIGN.md 3.6): a cheap, read-only
// change-detector used on window-focus to decide whether get_diff_summary
// needs to be re-run. Narrower than DiffParams — no compareMode/sourceScope/
// options, since those don't affect the fingerprint.
export interface FingerprintParams {
  repoPath: string;
  target: string;
  source: string;
}

export interface DiffFile {
  path: string;
  oldPath?: string; // set on rename
  status:
    | "added"
    | "modified"
    | "deleted"
    | "renamed"
    | "typechange"
    | "unmerged"
    | "submodule"
    | "other";
  additions: number | null; // null for binary files
  deletions: number | null;
  isBinary: boolean;
  isUntracked?: boolean; // from ls-files --others
}

export interface DiffSummary {
  files: DiffFile[];
  summary: { filesChanged: number; additions: number; deletions: number };
  omittedUntracked?: number; // untracked entries omitted past the 100-item cap
  warnings: string[];
  // Short SHA of `git merge-base <target> <source>` when compareMode ===
  // "merge-base"; always null for "tips" (DESIGN.md 3.4 / 5).
  mergeBase: string | null;
}

export interface FileContents {
  path: string;
  base: string | null; // null = added file
  head: string | null; // null = deleted file
  isBinary: boolean;
  isTooLarge?: boolean; // 1MB size guard tripped; base/head are null unless force:true
  sizeBytes?: number;
  note?: string; // e.g. submodule / symlink / mode-only change
}

export interface UiSettings {
  hideWhitespace: boolean; // default true (DESIGN.md 3.5)
}
