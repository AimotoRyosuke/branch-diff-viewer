// Types shared with the Rust backend over Tauri IPC.
// Mirrors DESIGN.md chapter 5 verbatim (camelCase on the wire).

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

export interface DiffParams {
  repoPath: string;
  target: string; // merge target (short or full; normalized internally)
  source: string; // merge source
  compareMode: CompareMode;
  sourceScope: SourceScope;
  options: {
    ignoreWhitespace?: boolean; // default true (Hide whitespace ON)
    contextLines?: number; // patch-view only; unused by full-text Monaco view
  };
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
