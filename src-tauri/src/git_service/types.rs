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
