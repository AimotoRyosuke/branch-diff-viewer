//! Types shared between the Rust backend and the frontend (via IPC).
//!
//! Field naming follows DESIGN.md chapter 5 (`camelCase` on the wire, matching
//! the TypeScript type definitions in `src/types.ts`).

use serde::{Deserialize, Serialize};

/// Result of `validate_repo`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    /// Normalized repository root (`git rev-parse --show-toplevel`).
    pub toplevel: String,
    /// `None` when HEAD is detached.
    pub current_branch: Option<String>,
    pub is_detached: bool,
    /// `false` for a fresh repository with no commits yet (unborn HEAD).
    pub has_commits: bool,
    pub git_version: String,
}

/// Comparison basis. Named by meaning rather than "two-dot"/"three-dot" to
/// avoid the git/log ambiguity called out in DESIGN.md 5 (L-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompareMode {
    MergeBase,
    Tips,
}

/// How much of the source side's working tree is included in the diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceScope {
    Committed,
    Staged,
    Unstaged,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffOptions {
    /// Defaults to `true` (Hide whitespace ON) per DESIGN.md 3.5.
    pub ignore_whitespace: Option<bool>,
    /// Patch-context only; unused by the full-text Monaco view (kept for
    /// wire-format parity with DESIGN.md 5).
    pub context_lines: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffParams {
    pub repo_path: String,
    pub target: String,
    pub source: String,
    pub compare_mode: CompareMode,
    pub source_scope: SourceScope,
    #[serde(default)]
    pub options: DiffOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffFileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Typechange,
    Unmerged,
    Submodule,
    Other,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffFile {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: DiffFileStatus,
    /// `None` for binary files.
    pub additions: Option<i64>,
    /// `None` for binary files.
    pub deletions: Option<i64>,
    pub is_binary: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_untracked: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffTotals {
    pub files_changed: usize,
    pub additions: i64,
    pub deletions: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSummary {
    pub files: Vec<DiffFile>,
    pub summary: DiffTotals,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omitted_untracked: Option<u32>,
    pub warnings: Vec<String>,
}

/// One branch entry returned by `list_branches` (DESIGN.md 5).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchRef {
    /// Display name (e.g. `origin/main`).
    pub short: String,
    /// Fully-qualified (`refs/heads/...` / `refs/remotes/...`).
    pub full: String,
    pub is_remote: bool,
}

/// Result of `list_branches` (DESIGN.md 3.2 / 5).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchList {
    pub local: Vec<BranchRef>,
    pub remote: Vec<BranchRef>,
    /// `None` on detached/unborn HEAD.
    pub current: Option<String>,
    /// `.git/FETCH_HEAD` mtime as ISO 8601, `None` if it doesn't exist
    /// (never fetched).
    pub last_fetch: Option<String>,
}

/// Result of `get_file_diff`: both sides' full text for one file
/// (DESIGN.md chapter 5).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContents {
    pub path: String,
    /// `None` for an added file (no base-side content) or when the size
    /// guard / binary check suppressed content.
    pub base: Option<String>,
    /// `None` for a deleted file (no head-side content) or when the size
    /// guard / binary check suppressed content.
    pub head: Option<String>,
    pub is_binary: bool,
    /// `true` when the 1MB size guard tripped and `force` was not set
    /// (DESIGN.md 4.3/4.4). `base`/`head` are `None` in that case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_too_large: Option<bool>,
    /// Size in bytes of the larger side, present when `isTooLarge` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}
