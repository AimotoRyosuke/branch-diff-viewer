//! Tauri IPC commands exposed by the git service.
//!
//! Every command here is **read-only**: it never runs a git subcommand that
//! mutates the index, working tree, or config (DESIGN.md 1 / 8). Repository
//! paths and refs are ref/path values only, and all commands are invoked
//! through `Command`'s argv array (never a shell) with a trailing `--`
//! pathspec separator on every `diff` invocation (DESIGN.md 4.0 / 8).

use std::io::Read;
use std::path::{Path, PathBuf};

use super::parse::{merge_entries, parse_name_status, parse_numstat, split_nul};
use super::process::{git_version, run_git, stderr_trimmed, stdout_trimmed};
use super::refs::normalize_ref;
use super::types::{
    CompareMode, DiffFile, DiffFileStatus, DiffParams, DiffSummary, DiffTotals, FileContents,
    RepoInfo, SourceScope,
};

const DIFF_GLOBAL_ARGS: &[&str] = &["-c", "core.quotepath=false", "-c", "core.fsmonitor=false"];
const DIFF_COMMON_ARGS: &[&str] = &["diff", "--no-color", "--no-ext-diff", "-M", "-z"];

/// 1MB size guard threshold (DESIGN.md 4.3/4.4).
const MAX_FILE_DIFF_BYTES: u64 = 1_048_576;
/// Bytes inspected for a NUL byte to decide `isBinary` (DESIGN.md 4.4 task step 1).
const BINARY_SNIFF_BYTES: usize = 8000;
/// Max untracked entries merged into the file list (DESIGN.md 3.3 / 7).
const UNTRACKED_LIMIT: usize = 100;

#[tauri::command]
pub fn validate_repo(path: String) -> Result<RepoInfo, String> {
    validate_repo_impl(&path)
}

#[tauri::command]
pub fn get_diff_summary(params: DiffParams) -> Result<DiffSummary, String> {
    get_diff_summary_impl(params)
}

#[tauri::command]
pub fn get_file_diff(
    params: DiffParams,
    path: String,
    old_path: Option<String>,
    force: bool,
) -> Result<FileContents, String> {
    get_file_diff_impl(params, path, old_path, force)
}

fn validate_repo_impl(path: &str) -> Result<RepoInfo, String> {
    let repo = Path::new(path);
    let meta = std::fs::metadata(repo).map_err(|e| format!("path not accessible: {e}"))?;
    if !meta.is_dir() {
        return Err("path is not a directory".to_string());
    }

    let is_tree_out = run_git(repo, &["rev-parse", "--is-inside-work-tree"])?;
    if stdout_trimmed(&is_tree_out) != "true" {
        return Err(
            "not a git working tree (bare repositories and non-git directories are not supported)"
                .to_string(),
        );
    }

    let toplevel_out = run_git(repo, &["rev-parse", "--show-toplevel"])?;
    if !toplevel_out.status.success() {
        return Err(format!(
            "failed to resolve repository root: {}",
            stderr_trimmed(&toplevel_out)
        ));
    }
    let toplevel = stdout_trimmed(&toplevel_out);
    let toplevel_path = Path::new(&toplevel);

    // symbolic-ref succeeds even on an unborn branch (HEAD is still a
    // symbolic ref to the not-yet-existent branch); it only fails when HEAD
    // is genuinely detached (DESIGN.md 4.3 M-5).
    let symbolic_out = run_git(toplevel_path, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let (current_branch, is_detached) = if symbolic_out.status.success() {
        (Some(stdout_trimmed(&symbolic_out)), false)
    } else {
        (None, true)
    };

    let head_verify_out = run_git(toplevel_path, &["rev-parse", "--verify", "--quiet", "HEAD"])?;
    let has_commits = head_verify_out.status.success();

    let version_out = git_version()?;
    let git_version_str = stdout_trimmed(&version_out);

    Ok(RepoInfo {
        toplevel,
        current_branch,
        is_detached,
        has_commits,
        git_version: git_version_str,
    })
}

fn get_diff_summary_impl(params: DiffParams) -> Result<DiffSummary, String> {
    let repo = Path::new(&params.repo_path);
    let meta = std::fs::metadata(repo).map_err(|e| format!("repoPath not accessible: {e}"))?;
    if !meta.is_dir() {
        return Err("repoPath is not a directory".to_string());
    }

    let mut warnings = Vec::new();

    // Normalize both refs to fully-qualified `refs/heads/...` /
    // `refs/remotes/...` form before they touch any other `git` invocation
    // (DESIGN.md 3.2 / 8 H-3).
    let target = normalize_ref(repo, &params.target)?;
    let source = normalize_ref(repo, &params.source)?;

    // HEAD constraint (DESIGN.md 3.3 / 4.1 / 4.2): staged/unstaged only exist
    // in the working tree of whatever is currently checked out. If `source`
    // isn't that branch (always true for a remote-tracking ref), fall back
    // to `committed` rather than erroring, and say so.
    let mut effective_scope = params.source_scope;
    if effective_scope != SourceScope::Committed {
        let current_branch = symbolic_ref_short(repo)?;
        if !source_matches_head(&source, current_branch.as_deref()) {
            warnings.push(format!(
                "source \"{}\" is not the checked-out branch (HEAD is {}) — scope fixed to committed",
                params.source,
                current_branch.as_deref().unwrap_or("detached"),
            ));
            effective_scope = SourceScope::Committed;
        }
    }

    // First operand of the diff: the merge-base commit (3-dot) or the target
    // tip itself (2-dot) — DESIGN.md 4.1/4.2.
    let base_rev = match params.compare_mode {
        CompareMode::MergeBase => compute_merge_base(repo, &target, &source, &mut warnings)?,
        CompareMode::Tips => Some(target.clone()),
    };
    let Some(base_rev) = base_rev else {
        // No merge base (unrelated histories): DESIGN.md 7 says this is a
        // warning, not an error — return an empty file list.
        return Ok(DiffSummary {
            files: Vec::new(),
            summary: DiffTotals { files_changed: 0, additions: 0, deletions: 0 },
            omitted_untracked: None,
            warnings,
        });
    };

    let ignore_whitespace = params.options.ignore_whitespace.unwrap_or(true);
    let scope_args = scope_diff_args(effective_scope, &base_rev, &source);

    // `-w` is intentionally never passed to `--name-status`: empirically (git
    // 2.50) `--name-status -w` still lists whitespace-only-changed files
    // (name-status only compares blob ids, it never runs the line-level
    // algorithm that `-w` affects), so passing it there would be a no-op at
    // best and misleading in intent. `--numstat -w`, by contrast, correctly
    // drops those files. `merge_entries` reconciles the two by path and,
    // when `ignore_whitespace` is set, treats a name-status entry with no
    // numstat match as "hidden by -w" rather than an error (DESIGN.md 3.5).
    let name_status_out = run_diff(repo, "--name-status", &scope_args, false)?;
    if !name_status_out.status.success() {
        return Err(format!(
            "git diff --name-status failed: {}",
            stderr_trimmed(&name_status_out)
        ));
    }

    let numstat_out = run_diff(repo, "--numstat", &scope_args, ignore_whitespace)?;
    if !numstat_out.status.success() {
        return Err(format!(
            "git diff --numstat failed: {}",
            stderr_trimmed(&numstat_out)
        ));
    }

    let name_entries = parse_name_status(&name_status_out.stdout)?;
    let numstat_entries = parse_numstat(&numstat_out.stdout)?;
    let mut files = merge_entries(name_entries, numstat_entries, ignore_whitespace)?;

    // Untracked files only exist "in the working tree" and only make sense
    // to fold in when the scope actually reaches the working tree
    // (DESIGN.md 3.3 / C-3).
    let mut omitted_untracked = None;
    if effective_scope == SourceScope::Unstaged {
        let all_untracked = list_untracked_paths(repo)?;
        let omitted = all_untracked.len().saturating_sub(UNTRACKED_LIMIT);
        if omitted > 0 {
            omitted_untracked = Some(omitted as u32);
        }
        for rel_path in all_untracked.into_iter().take(UNTRACKED_LIMIT) {
            files.push(build_untracked_entry(repo, &rel_path)?);
        }
    }

    let mut additions_total: i64 = 0;
    let mut deletions_total: i64 = 0;
    for f in &files {
        additions_total += f.additions.unwrap_or(0);
        deletions_total += f.deletions.unwrap_or(0);
    }

    Ok(DiffSummary {
        summary: DiffTotals {
            files_changed: files.len(),
            additions: additions_total,
            deletions: deletions_total,
        },
        files,
        omitted_untracked,
        warnings,
    })
}

/// Current checked-out branch's short name (`None` on detached/unborn HEAD),
/// via `symbolic-ref` rather than `--abbrev-ref` (DESIGN.md 4.3 M-5: the
/// latter returns the literal string `HEAD` when detached). `pub(super)` so
/// `branches::list_branches` can reuse it for `BranchList.current`.
pub(super) fn symbolic_ref_short(repo: &Path) -> Result<Option<String>, String> {
    let out = run_git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    if out.status.success() {
        Ok(Some(stdout_trimmed(&out)))
    } else {
        Ok(None)
    }
}

/// Whether `source` (already normalized to a fully-qualified ref by
/// [`normalize_ref`]) refers to the branch currently checked out (DESIGN.md
/// 3.3 HEAD constraint). A detached/unborn HEAD (`current_branch = None`)
/// never matches.
fn source_matches_head(source: &str, current_branch: Option<&str>) -> bool {
    match current_branch {
        None => false,
        Some(branch) => source == format!("refs/heads/{branch}"),
    }
}

/// Builds the diff-subcommand args placed right after the common flags and
/// before `--` (DESIGN.md 4.1/4.2's per-scope tables).
fn scope_diff_args(scope: SourceScope, base_rev: &str, source: &str) -> Vec<String> {
    match scope {
        SourceScope::Committed => vec![base_rev.to_string(), source.to_string()],
        SourceScope::Staged => vec!["--cached".to_string(), base_rev.to_string()],
        SourceScope::Unstaged => vec![base_rev.to_string()],
    }
}

/// `git merge-base <target> <source>` (via `--all` so a criss-cross's full
/// set of candidates can be counted); takes the first line when multiple
/// merge bases exist and records a warning (DESIGN.md 4.1 / 7). `Ok(None)`
/// means no merge base exists (unrelated histories) — DESIGN.md 7 treats
/// that as a warning rather than a hard error.
fn compute_merge_base(
    repo: &Path,
    target: &str,
    source: &str,
    warnings: &mut Vec<String>,
) -> Result<Option<String>, String> {
    let out = run_git(repo, &["merge-base", "--all", target, source])?;
    let stdout = stdout_trimmed(&out);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    if !out.status.success() {
        if lines.is_empty() {
            warnings.push(
                "no merge base found between target and source (unrelated histories?)"
                    .to_string(),
            );
            return Ok(None);
        }
        return Err(format!(
            "git merge-base failed: {}",
            stderr_trimmed(&out)
        ));
    }
    match lines.len() {
        0 => {
            warnings.push(
                "no merge base found between target and source (unrelated histories?)"
                    .to_string(),
            );
            Ok(None)
        }
        1 => Ok(Some(lines[0].to_string())),
        _ => {
            warnings.push(
                "multiple merge bases found (criss-cross merge); using the first one".to_string(),
            );
            Ok(Some(lines[0].to_string()))
        }
    }
}

/// Lists untracked (non-ignored) paths via `git ls-files --others
/// --exclude-standard -z` (DESIGN.md 3.3 / 4.3).
fn list_untracked_paths(repo: &Path) -> Result<Vec<String>, String> {
    let out = run_git(repo, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    if !out.status.success() {
        return Err(format!("git ls-files failed: {}", stderr_trimmed(&out)));
    }
    Ok(split_nul(&out.stdout))
}

/// Builds the synthetic `DiffFile` entry for one untracked path
/// (DESIGN.md 3.3): `status = added`, `isUntracked = true`, `deletions = 0`,
/// and `additions` = line count for text files up to the same 1MB size guard
/// used elsewhere (`None` for binaries or oversized files).
fn build_untracked_entry(repo: &Path, rel_path: &str) -> Result<DiffFile, String> {
    let abs_path = repo.join(rel_path);
    let size = std::fs::metadata(&abs_path)
        .map_err(|e| format!("failed to stat untracked file '{rel_path}': {e}"))?
        .len();

    let (is_binary, additions) = if size > MAX_FILE_DIFF_BYTES {
        // Oversized: only sniff a small prefix for the NUL-byte binary
        // check; skip reading the whole file just to count lines we won't
        // report anyway.
        let prefix = read_prefix(&abs_path, BINARY_SNIFF_BYTES)?;
        (looks_binary(&prefix), None)
    } else {
        let bytes = std::fs::read(&abs_path)
            .map_err(|e| format!("failed to read untracked file '{rel_path}': {e}"))?;
        if looks_binary(&bytes) {
            (true, None)
        } else {
            (false, Some(count_lines(&bytes)))
        }
    };

    Ok(DiffFile {
        path: rel_path.to_string(),
        old_path: None,
        status: DiffFileStatus::Added,
        additions,
        deletions: Some(0),
        is_binary,
        is_untracked: Some(true),
    })
}

/// Reads up to `limit` bytes from the start of `path`.
fn read_prefix(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let mut f = std::fs::File::open(path)
        .map_err(|e| format!("failed to open '{}': {e}", path.display()))?;
    let mut buf = vec![0u8; limit];
    let n = f
        .read(&mut buf)
        .map_err(|e| format!("failed to read '{}': {e}", path.display()))?;
    buf.truncate(n);
    Ok(buf)
}

/// Line count used for an untracked file's `additions` (DESIGN.md 3.3): the
/// number of `\n`-terminated lines, plus one more if the file has trailing
/// content with no final newline.
fn count_lines(bytes: &[u8]) -> i64 {
    if bytes.is_empty() {
        return 0;
    }
    let mut count = bytes.iter().filter(|&&b| b == b'\n').count() as i64;
    if *bytes.last().expect("checked non-empty above") != b'\n' {
        count += 1;
    }
    count
}

fn run_diff(
    repo: &Path,
    stat_flag: &str,
    scope_args: &[String],
    ignore_whitespace: bool,
) -> Result<std::process::Output, String> {
    let mut args: Vec<&str> =
        Vec::with_capacity(DIFF_GLOBAL_ARGS.len() + DIFF_COMMON_ARGS.len() + scope_args.len() + 3);
    args.extend_from_slice(DIFF_GLOBAL_ARGS);
    args.extend_from_slice(DIFF_COMMON_ARGS);
    if ignore_whitespace {
        args.push("-w");
    }
    args.push(stat_flag);
    for a in scope_args {
        args.push(a.as_str());
    }
    args.push("--");
    run_git(repo, &args)
}

/// Where to read one side's (base or head) full text from.
enum Side {
    /// A git object reference of the form `<rev>:<path>` or `:<path>`
    /// (stage 0), read via `git show`/`git cat-file -s`.
    Blob(String),
    /// A direct working-tree path (already validated to stay inside the
    /// repository root), read via `std::fs`.
    WorkingTree(PathBuf),
}

/// Outcome of a pre-flight existence/size probe for one [`Side`].
enum Probe {
    /// The path does not exist at that revision / in the index / on disk.
    /// Surfaces as `None` content (added/deleted file) rather than an error
    /// (DESIGN.md 4.3 task step 1).
    Missing,
    Size(u64),
}

fn get_file_diff_impl(
    params: DiffParams,
    path: String,
    old_path: Option<String>,
    force: bool,
) -> Result<FileContents, String> {
    let repo = Path::new(&params.repo_path);
    let meta = std::fs::metadata(repo).map_err(|e| format!("repoPath not accessible: {e}"))?;
    if !meta.is_dir() {
        return Err("repoPath is not a directory".to_string());
    }

    // Defense-in-depth (DESIGN.md 8): re-validate path-shaped inputs on the
    // Rust side regardless of scope, even though only the working-tree read
    // is a real filesystem traversal risk.
    validate_repo_relative_path(&path)?;
    if let Some(op) = &old_path {
        validate_repo_relative_path(op)?;
    }

    // Normalize both refs to fully-qualified form before they touch any
    // other `git` invocation (DESIGN.md 3.2 / 8 H-3), same as
    // `get_diff_summary_impl`.
    let target = normalize_ref(repo, &params.target)?;
    let source = normalize_ref(repo, &params.source)?;

    let base_rev = match params.compare_mode {
        CompareMode::MergeBase => {
            let mut warnings = Vec::new();
            compute_merge_base(repo, &target, &source, &mut warnings)?.ok_or_else(|| {
                "no merge base found between target and source (unrelated histories?)".to_string()
            })?
        }
        // DESIGN.md 4.1/4.2: "tips" compares against the target branch tip
        // directly rather than the merge-base.
        CompareMode::Tips => target.clone(),
    };
    // Renames read the base side under the old path (task step 1).
    let base_path = old_path.unwrap_or_else(|| path.clone());
    let base_side = Side::Blob(format!("{base_rev}:{base_path}"));

    let head_side = match params.source_scope {
        SourceScope::Committed => Side::Blob(format!("{source}:{path}")),
        // Stage 0 of the index (DESIGN.md 4.3).
        SourceScope::Staged => Side::Blob(format!(":{path}")),
        SourceScope::Unstaged => {
            Side::WorkingTree(safe_join_repo_path(repo, &path)?)
        }
    };

    let base_probe = probe_side(repo, &base_side)?;
    let head_probe = probe_side(repo, &head_side)?;

    if !force {
        let max_size = [&base_probe, &head_probe]
            .into_iter()
            .filter_map(|p| match p {
                Probe::Size(n) => Some(*n),
                Probe::Missing => None,
            })
            .max();
        if let Some(size) = max_size {
            if size > MAX_FILE_DIFF_BYTES {
                return Ok(FileContents {
                    path,
                    base: None,
                    head: None,
                    is_binary: false,
                    is_too_large: Some(true),
                    size_bytes: Some(size),
                    note: None,
                });
            }
        }
    }

    let base_bytes = match base_probe {
        Probe::Missing => None,
        Probe::Size(_) => Some(read_side(repo, &base_side)?),
    };
    let head_bytes = match head_probe {
        Probe::Missing => None,
        Probe::Size(_) => Some(read_side(repo, &head_side)?),
    };

    let is_binary = base_bytes.as_deref().is_some_and(looks_binary)
        || head_bytes.as_deref().is_some_and(looks_binary);

    if is_binary {
        return Ok(FileContents {
            path,
            base: None,
            head: None,
            is_binary: true,
            is_too_large: None,
            size_bytes: None,
            note: None,
        });
    }

    Ok(FileContents {
        path,
        base: base_bytes.map(|b| String::from_utf8_lossy(&b).into_owned()),
        head: head_bytes.map(|b| String::from_utf8_lossy(&b).into_owned()),
        is_binary: false,
        is_too_large: None,
        size_bytes: None,
        note: None,
    })
}

/// Rejects absolute paths, `..` traversal, and NUL bytes. Applies to every
/// path-shaped IPC input regardless of source scope (DESIGN.md 8: Rust
/// re-validates inputs as the defense layer against a compromised frontend).
fn validate_repo_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("path must not be empty".to_string());
    }
    if path.contains('\0') {
        return Err("path must not contain a NUL byte".to_string());
    }
    for component in Path::new(path).components() {
        match component {
            std::path::Component::Normal(_) | std::path::Component::CurDir => {}
            _ => {
                return Err(format!(
                    "path must be repository-relative with no traversal: '{path}'"
                ))
            }
        }
    }
    Ok(())
}

/// Joins `path` onto `repo`, rejecting the result if `path` could escape the
/// repository root (DESIGN.md 4.3: "作業ツリー直読み...パストラバーサル防止").
fn safe_join_repo_path(repo: &Path, path: &str) -> Result<PathBuf, String> {
    validate_repo_relative_path(path)?;
    Ok(repo.join(path))
}

/// Probes existence and, when present, byte size of one [`Side`] without
/// reading its full content (DESIGN.md 4.3 size guard).
fn probe_side(repo: &Path, side: &Side) -> Result<Probe, String> {
    match side {
        Side::Blob(spec) => {
            let out = run_git(repo, &["cat-file", "-s", spec])?;
            if out.status.success() {
                let stdout = stdout_trimmed(&out);
                let size: u64 = stdout
                    .parse()
                    .map_err(|_| format!("unexpected `git cat-file -s` output: '{stdout}'"))?;
                Ok(Probe::Size(size))
            } else {
                let err = stderr_trimmed(&out);
                if is_missing_path_error(&err) {
                    Ok(Probe::Missing)
                } else {
                    Err(format!("git cat-file -s {spec} failed: {err}"))
                }
            }
        }
        Side::WorkingTree(abs_path) => match std::fs::metadata(abs_path) {
            Ok(meta) => Ok(Probe::Size(meta.len())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Probe::Missing),
            Err(e) => Err(format!("failed to stat working tree file: {e}")),
        },
    }
}

/// Reads the full content of one [`Side`]. Only called after [`probe_side`]
/// reported it as present.
fn read_side(repo: &Path, side: &Side) -> Result<Vec<u8>, String> {
    match side {
        Side::Blob(spec) => {
            let out = run_git(repo, &["show", spec])?;
            if out.status.success() {
                Ok(out.stdout)
            } else {
                Err(format!("git show {spec} failed: {}", stderr_trimmed(&out)))
            }
        }
        Side::WorkingTree(abs_path) => {
            std::fs::read(abs_path).map_err(|e| format!("failed to read working tree file: {e}"))
        }
    }
}

/// Matches the handful of git error messages that mean "this path does not
/// exist at that revision / in the index / on disk" rather than a genuine
/// failure (verified empirically against git 2.50; DESIGN.md task step 1).
fn is_missing_path_error(stderr: &str) -> bool {
    stderr.contains("does not exist") || stderr.contains("exists on disk, but not")
}

/// `isBinary` heuristic: a NUL byte in the first [`BINARY_SNIFF_BYTES`]
/// bytes (DESIGN.md 4.4 task step 1; matches git's own `--numstat` binary
/// detection in spirit).
fn looks_binary(bytes: &[u8]) -> bool {
    let sniff_len = bytes.len().min(BINARY_SNIFF_BYTES);
    bytes[..sniff_len].contains(&0u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    /// Runs a git command in `repo` for test setup, panicking on failure.
    fn git(repo: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .expect("failed to run git for test setup");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_repo() -> TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git(dir.path(), &["init", "--initial-branch=main"]);
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        dir
    }

    #[test]
    fn validate_repo_detects_working_tree_and_head() {
        let dir = init_repo();
        fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-m", "initial"]);

        let info = validate_repo_impl(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(info.current_branch.as_deref(), Some("main"));
        assert!(!info.is_detached);
        assert!(info.has_commits);
        assert!(info.git_version.to_lowercase().contains("git version"));
        // toplevel should be the canonicalized repo path
        assert_eq!(
            fs::canonicalize(&info.toplevel).unwrap(),
            fs::canonicalize(dir.path()).unwrap()
        );
    }

    #[test]
    fn validate_repo_rejects_non_repo_path() {
        let dir = tempfile::tempdir().unwrap();
        let err = validate_repo_impl(dir.path().to_str().unwrap()).unwrap_err();
        assert!(err.contains("not a git working tree"), "unexpected error: {err}");
    }

    /// Bare repositories have no working tree, so `--is-inside-work-tree`
    /// prints `false` (DESIGN.md 3.1 / 7): `validate_repo` must reject them
    /// explicitly rather than treating exit code 0 as success.
    #[test]
    fn validate_repo_rejects_bare_repository() {
        let dir = tempfile::tempdir().unwrap();
        let bare_path = dir.path().join("bare.git");
        let out = Command::new("git")
            .args(["init", "--bare", bare_path.to_str().unwrap()])
            .output()
            .expect("failed to init bare repo for test");
        assert!(out.status.success(), "git init --bare failed");

        let err = validate_repo_impl(bare_path.to_str().unwrap()).unwrap_err();
        assert!(err.contains("not a git working tree"), "unexpected error: {err}");
    }

    #[test]
    fn validate_repo_detects_unborn_branch() {
        let dir = init_repo();
        let info = validate_repo_impl(dir.path().to_str().unwrap()).unwrap();
        assert!(!info.has_commits);
        // symbolic-ref succeeds even for an unborn branch.
        assert_eq!(info.current_branch.as_deref(), Some("main"));
        assert!(!info.is_detached);
    }

    fn base_params(repo: &Path, target: &str, source: &str) -> DiffParams {
        DiffParams {
            repo_path: repo.to_str().unwrap().to_string(),
            target: target.to_string(),
            source: source.to_string(),
            compare_mode: CompareMode::MergeBase,
            source_scope: SourceScope::Committed,
            options: super::super::types::DiffOptions {
                ignore_whitespace: Some(false),
                context_lines: None,
            },
        }
    }

    #[test]
    fn get_diff_summary_reports_added_modified_deleted_binary_rename_and_japanese_paths() {
        let dir = init_repo();
        let repo = dir.path();

        // Base commit on main: files that will be modified / deleted / renamed.
        fs::write(repo.join("modified.txt"), "line1\nline2\n").unwrap();
        fs::write(repo.join("deleted.txt"), "bye\n").unwrap();
        fs::write(repo.join("old_name.txt"), "rename me\nkeep me stable\nline three\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);

        git(repo, &["checkout", "-b", "feature"]);

        // added (plain text)
        fs::write(repo.join("added.txt"), "new content\n").unwrap();
        // added (Japanese filename + content)
        fs::write(repo.join("日本語ファイル.txt"), "日本語のコンテンツ\n").unwrap();
        // modified
        fs::write(repo.join("modified.txt"), "line1\nline2 changed\n").unwrap();
        // deleted
        fs::remove_file(repo.join("deleted.txt")).unwrap();
        // renamed (content mostly unchanged so git detects it as a rename with -M)
        fs::rename(repo.join("old_name.txt"), repo.join("new_name.txt")).unwrap();
        // binary file (added)
        fs::write(repo.join("image.bin"), [0u8, 159, 146, 150, 0, 1, 2, 3]).unwrap();

        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-m", "feature changes"]);

        let params = base_params(repo, "main", "feature");
        let summary = get_diff_summary_impl(params).unwrap();

        assert!(summary.warnings.is_empty(), "unexpected warnings: {:?}", summary.warnings);

        let find = |p: &str| {
            summary
                .files
                .iter()
                .find(|f| f.path == p)
                .unwrap_or_else(|| panic!("missing file {p} in {:#?}", summary.files))
        };

        let added = find("added.txt");
        assert_eq!(added.status, super::super::types::DiffFileStatus::Added);
        assert!(!added.is_binary);
        assert_eq!(added.additions, Some(1));
        assert_eq!(added.deletions, Some(0));

        let jp = find("日本語ファイル.txt");
        assert_eq!(jp.status, super::super::types::DiffFileStatus::Added);
        assert!(!jp.is_binary);

        let modified = find("modified.txt");
        assert_eq!(modified.status, super::super::types::DiffFileStatus::Modified);
        assert_eq!(modified.additions, Some(1));
        assert_eq!(modified.deletions, Some(1));

        let deleted = find("deleted.txt");
        assert_eq!(deleted.status, super::super::types::DiffFileStatus::Deleted);
        assert_eq!(deleted.deletions, Some(1));

        let renamed = find("new_name.txt");
        assert_eq!(renamed.status, super::super::types::DiffFileStatus::Renamed);
        assert_eq!(renamed.old_path.as_deref(), Some("old_name.txt"));

        let binary = find("image.bin");
        assert!(binary.is_binary);
        assert_eq!(binary.additions, None);
        assert_eq!(binary.deletions, None);

        assert_eq!(summary.summary.files_changed, summary.files.len());
        assert_eq!(summary.files.len(), 6);
    }

    #[test]
    fn get_diff_summary_applies_ignore_whitespace_when_requested() {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("ws.txt"), "line one\nline two\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);

        git(repo, &["checkout", "-b", "feature"]);
        // Only whitespace changes.
        fs::write(repo.join("ws.txt"), "line one   \nline two\n").unwrap();
        git(repo, &["commit", "-am", "whitespace only"]);

        let mut params = base_params(repo, "main", "feature");
        params.options.ignore_whitespace = Some(true);
        let summary = get_diff_summary_impl(params).unwrap();
        assert!(
            summary.files.is_empty(),
            "expected whitespace-only change to be hidden, got {:#?}",
            summary.files
        );

        let mut params2 = base_params(repo, "main", "feature");
        params2.options.ignore_whitespace = Some(false);
        let summary2 = get_diff_summary_impl(params2).unwrap();
        assert_eq!(summary2.files.len(), 1);
    }

    // --- Phase 4: full scope × compare-mode matrix, HEAD constraint,
    // untracked merge-in, and multiple-merge-base warnings ------------------

    /// Runs `git diff <global> <common> [-w] <stat_flag> <scope_args...> --`
    /// exactly as production code does, for use as the "manual" oracle in
    /// tests (kept independent of `run_diff`'s own arg list so a regression
    /// in arg assembly would actually be caught).
    fn manual_diff_bytes(repo: &Path, stat_flag: &str, scope_args: &[&str]) -> Vec<u8> {
        let mut args: Vec<&str> = vec![
            "-c",
            "core.quotepath=false",
            "-c",
            "core.fsmonitor=false",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "-M",
            "-z",
            stat_flag,
        ];
        args.extend_from_slice(scope_args);
        args.push("--");
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(&args)
            .output()
            .expect("failed to run manual git diff");
        assert!(
            out.status.success(),
            "manual git diff {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    }

    fn manual_merge_base(repo: &Path, target: &str, source: &str) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["merge-base", target, source])
            .output()
            .expect("failed to run git merge-base");
        assert!(out.status.success(), "git merge-base failed");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Fixture with a real fork point plus divergence in every layer
    /// (committed / staged / unstaged / untracked) so every scope×mode
    /// combination produces a distinguishable result:
    /// - `main` and `feature` share a common ancestor, then each gets its
    ///   own commit (`main` grows `main_only.txt`; `feature` changes
    ///   `shared.txt`) so merge-base and tips diffs differ.
    /// - `feature` (checked out, i.e. HEAD) additionally gets a staged file,
    ///   a further unstaged edit on top of the committed change, and an
    ///   untracked file — so committed/staged/unstaged all disagree too.
    fn setup_scope_matrix_fixture() -> TempDir {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("shared.txt"), "base\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);

        git(repo, &["checkout", "-b", "feature"]);
        fs::write(repo.join("shared.txt"), "feature change\n").unwrap();
        git(repo, &["commit", "-am", "feature change"]);

        git(repo, &["checkout", "main"]);
        fs::write(repo.join("main_only.txt"), "main only\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "main only"]);

        git(repo, &["checkout", "feature"]);
        fs::write(repo.join("staged.txt"), "staged content\n").unwrap();
        git(repo, &["add", "staged.txt"]);
        fs::write(repo.join("shared.txt"), "feature change + unstaged edit\n").unwrap();
        fs::write(repo.join("untracked.txt"), "brand new\n").unwrap();

        dir
    }

    #[test]
    fn get_diff_summary_matches_manual_git_diff_across_scope_and_compare_mode() {
        let dir = setup_scope_matrix_fixture();
        let repo = dir.path();

        for compare_mode in [CompareMode::MergeBase, CompareMode::Tips] {
            let base_rev = match compare_mode {
                CompareMode::MergeBase => manual_merge_base(repo, "main", "feature"),
                CompareMode::Tips => "main".to_string(),
            };

            for scope in [SourceScope::Committed, SourceScope::Staged, SourceScope::Unstaged] {
                let mut params = base_params(repo, "main", "feature");
                params.compare_mode = compare_mode;
                params.source_scope = scope;
                params.options.ignore_whitespace = Some(false);
                let summary = get_diff_summary_impl(params).unwrap();
                assert!(
                    summary.warnings.is_empty(),
                    "unexpected warnings for {compare_mode:?}/{scope:?}: {:?}",
                    summary.warnings
                );

                // Manually build the same scope args the production code
                // should have used, per DESIGN.md 4.1/4.2's tables.
                let scope_args: Vec<&str> = match scope {
                    SourceScope::Committed => vec![base_rev.as_str(), "feature"],
                    SourceScope::Staged => vec!["--cached", base_rev.as_str()],
                    SourceScope::Unstaged => vec![base_rev.as_str()],
                };
                let ns_bytes = manual_diff_bytes(repo, "--name-status", &scope_args);
                let nu_bytes = manual_diff_bytes(repo, "--numstat", &scope_args);
                let expected = merge_entries(
                    parse_name_status(&ns_bytes).unwrap(),
                    parse_numstat(&nu_bytes).unwrap(),
                    false,
                )
                .unwrap();

                let tracked: Vec<&DiffFile> = summary
                    .files
                    .iter()
                    .filter(|f| f.is_untracked != Some(true))
                    .collect();
                assert_eq!(
                    tracked.len(),
                    expected.len(),
                    "{compare_mode:?}/{scope:?}: tracked file count mismatch (got {:#?}, want {:#?})",
                    tracked,
                    expected
                );
                for ef in &expected {
                    let af = tracked
                        .iter()
                        .find(|f| f.path == ef.path)
                        .unwrap_or_else(|| {
                            panic!(
                                "{compare_mode:?}/{scope:?}: missing expected file '{}' in {:#?}",
                                ef.path, tracked
                            )
                        });
                    assert_eq!(af.status, ef.status, "status mismatch for {}", ef.path);
                    assert_eq!(af.old_path, ef.old_path, "oldPath mismatch for {}", ef.path);
                    assert_eq!(af.additions, ef.additions, "additions mismatch for {}", ef.path);
                    assert_eq!(af.deletions, ef.deletions, "deletions mismatch for {}", ef.path);
                    assert_eq!(af.is_binary, ef.is_binary, "isBinary mismatch for {}", ef.path);
                }

                // Untracked merge-in only happens for the unstaged scope.
                if scope == SourceScope::Unstaged {
                    let untracked: Vec<&str> = summary
                        .files
                        .iter()
                        .filter(|f| f.is_untracked == Some(true))
                        .map(|f| f.path.as_str())
                        .collect();
                    assert_eq!(untracked, vec!["untracked.txt"]);
                } else {
                    assert!(
                        summary.files.iter().all(|f| f.is_untracked != Some(true)),
                        "{compare_mode:?}/{scope:?} must not merge in untracked files"
                    );
                }

                let total_add: i64 = summary.files.iter().map(|f| f.additions.unwrap_or(0)).sum();
                let total_del: i64 = summary.files.iter().map(|f| f.deletions.unwrap_or(0)).sum();
                assert_eq!(summary.summary.files_changed, summary.files.len());
                assert_eq!(summary.summary.additions, total_add);
                assert_eq!(summary.summary.deletions, total_del);
            }
        }

        // `main_only.txt` only shows up when comparing against the target's
        // tip (compare_mode=Tips), never against the merge-base, since it
        // postdates the fork point (this is the whole point of merge-base
        // 3-dot comparison vs. a plain 2-dot tips comparison).
        let mut mb_params = base_params(repo, "main", "feature");
        mb_params.compare_mode = CompareMode::MergeBase;
        mb_params.source_scope = SourceScope::Committed;
        let mb_summary = get_diff_summary_impl(mb_params).unwrap();
        assert!(!mb_summary.files.iter().any(|f| f.path == "main_only.txt"));

        let mut tips_params = base_params(repo, "main", "feature");
        tips_params.compare_mode = CompareMode::Tips;
        tips_params.source_scope = SourceScope::Committed;
        let tips_summary = get_diff_summary_impl(tips_params).unwrap();
        let main_only = tips_summary
            .files
            .iter()
            .find(|f| f.path == "main_only.txt")
            .expect("tips comparison should surface main_only.txt as deleted from source's side");
        assert_eq!(main_only.status, DiffFileStatus::Deleted);
    }

    #[test]
    fn get_diff_summary_falls_back_to_committed_when_source_is_not_head() {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("a.txt"), "base\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);

        git(repo, &["checkout", "-b", "feature"]);
        fs::write(repo.join("a.txt"), "feature\n").unwrap();
        git(repo, &["commit", "-am", "feature change"]);

        // HEAD is now main; "feature" is no longer the checked-out branch, so
        // staged/unstaged scopes have no working tree to read from.
        git(repo, &["checkout", "main"]);

        for scope in [SourceScope::Staged, SourceScope::Unstaged] {
            let mut params = base_params(repo, "main", "feature");
            params.source_scope = scope;
            let summary = get_diff_summary_impl(params).unwrap();
            assert!(
                summary
                    .warnings
                    .iter()
                    .any(|w| w.contains("not the checked-out branch")),
                "{scope:?}: expected HEAD-constraint warning, got {:?}",
                summary.warnings
            );
            // Falls back to committed: exactly the one committed change,
            // and no untracked merge-in (that only applies to unstaged).
            assert_eq!(summary.files.len(), 1);
            assert_eq!(summary.files[0].path, "a.txt");
            assert_eq!(summary.files[0].is_untracked, None);
        }

        // Fully-qualified form of the checked-out branch must NOT trigger
        // the fallback.
        git(repo, &["checkout", "feature"]);
        let mut ok_params = base_params(repo, "main", "refs/heads/feature");
        ok_params.source_scope = SourceScope::Staged;
        let ok_summary = get_diff_summary_impl(ok_params).unwrap();
        assert!(
            ok_summary.warnings.is_empty(),
            "fully-qualified HEAD ref should not trigger fallback: {:?}",
            ok_summary.warnings
        );
    }

    /// Selecting a remote-tracking branch as `source` by its short name
    /// (`origin/feature`, not the fully-qualified `refs/remotes/origin/feature`)
    /// must still resolve via ref normalization and trip the HEAD constraint
    /// (DESIGN.md 3.2/3.3: a remote-tracking ref can never be the checked-out
    /// branch).
    #[test]
    fn get_diff_summary_normalizes_short_remote_tracking_source_and_locks_scope() {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("a.txt"), "base\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);
        git(repo, &["checkout", "-b", "feature"]);
        fs::write(repo.join("a.txt"), "feature\n").unwrap();
        git(repo, &["commit", "-am", "feature change"]);

        let sha = {
            let out = run_git(repo, &["rev-parse", "feature"]).unwrap();
            stdout_trimmed(&out)
        };
        git(repo, &["update-ref", "refs/remotes/origin/feature", &sha]);

        let mut params = base_params(repo, "main", "origin/feature");
        params.source_scope = SourceScope::Unstaged;
        let summary = get_diff_summary_impl(params).unwrap();

        assert!(
            summary.warnings.iter().any(|w| w.contains("not the checked-out branch")),
            "expected HEAD-constraint warning, got {:?}",
            summary.warnings
        );
        assert!(
            summary.files.iter().all(|f| f.is_untracked != Some(true)),
            "scope should have been fixed to committed, so no untracked merge-in"
        );
    }

    /// ref normalization/validation (DESIGN.md 8 H-3) is enforced through the
    /// public `get_diff_summary`/`get_file_diff` entry points too, not just
    /// unit-tested in isolation on `refs::normalize_ref`.
    #[test]
    fn get_diff_summary_rejects_dash_prefixed_and_nonexistent_refs() {
        let dir = init_repo();
        let repo = dir.path();
        fs::write(repo.join("a.txt"), "base\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);

        let dash_err =
            get_diff_summary_impl(base_params(repo, "-main", "main")).unwrap_err();
        assert!(dash_err.contains("must not start with '-'"), "unexpected error: {dash_err}");

        let missing_err =
            get_diff_summary_impl(base_params(repo, "main", "does-not-exist")).unwrap_err();
        assert!(missing_err.contains("not found"), "unexpected error: {missing_err}");
    }

    #[test]
    fn get_diff_summary_merges_untracked_files_and_caps_at_100() {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("a.txt"), "base\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);
        git(repo, &["checkout", "-b", "feature"]);

        for i in 0..105 {
            fs::write(
                repo.join(format!("untracked_{i:03}.txt")),
                format!("line one\nline two {i}\n"),
            )
            .unwrap();
        }

        let mut params = base_params(repo, "main", "feature");
        params.source_scope = SourceScope::Unstaged;
        let summary = get_diff_summary_impl(params).unwrap();

        let untracked: Vec<&DiffFile> =
            summary.files.iter().filter(|f| f.is_untracked == Some(true)).collect();
        assert_eq!(untracked.len(), UNTRACKED_LIMIT);
        assert_eq!(summary.omitted_untracked, Some(5));

        for f in &untracked {
            assert_eq!(f.status, DiffFileStatus::Added);
            assert_eq!(f.deletions, Some(0));
            assert_eq!(f.additions, Some(2));
            assert!(!f.is_binary);
        }
    }

    #[test]
    fn get_diff_summary_untracked_entries_null_additions_for_binary_and_oversized_files() {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("a.txt"), "base\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);
        git(repo, &["checkout", "-b", "feature"]);

        fs::write(repo.join("binary_untracked.bin"), [0u8, 1, 2, 3, 0, 4]).unwrap();
        let big_content = "x".repeat(MAX_FILE_DIFF_BYTES as usize + 1);
        fs::write(repo.join("big_untracked.txt"), &big_content).unwrap();

        let mut params = base_params(repo, "main", "feature");
        params.source_scope = SourceScope::Unstaged;
        let summary = get_diff_summary_impl(params).unwrap();

        let binary = summary
            .files
            .iter()
            .find(|f| f.path == "binary_untracked.bin")
            .unwrap();
        assert!(binary.is_binary);
        assert_eq!(binary.additions, None);
        assert_eq!(binary.deletions, Some(0));

        let big = summary
            .files
            .iter()
            .find(|f| f.path == "big_untracked.txt")
            .unwrap();
        assert!(!big.is_binary);
        assert_eq!(big.additions, None);
        assert_eq!(big.deletions, Some(0));
    }

    /// Criss-cross fixture (same shape as git's own `t6010-merge-base`
    /// test, but using non-overlapping files so both merges are conflict
    /// free): two branches each merge the other's pre-merge tip, so
    /// `merge-base --all` reports two candidates instead of one.
    #[test]
    fn get_diff_summary_warns_on_multiple_merge_bases() {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("base.txt"), "base\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "commit 1"]);
        git(repo, &["tag", "test1"]);

        fs::write(repo.join("m2.txt"), "m2\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "commit 2"]);
        git(repo, &["tag", "test2"]);

        git(repo, &["checkout", "-b", "side", "test1"]);
        fs::write(repo.join("s1.txt"), "s1\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "commit 3"]);
        git(repo, &["tag", "test3"]);

        git(repo, &["merge", "-m", "merge test2 into side", "test2"]);
        git(repo, &["tag", "test4"]);

        git(repo, &["checkout", "main"]);
        git(repo, &["merge", "-m", "merge test3 into main", "test3"]);
        git(repo, &["tag", "test5"]);

        let params = base_params(repo, "main", "side");
        let summary = get_diff_summary_impl(params).unwrap();
        assert!(
            summary.warnings.iter().any(|w| w.contains("multiple merge bases")),
            "expected multiple-merge-base warning, got {:?}",
            summary.warnings
        );
    }

    // --- get_file_diff -----------------------------------------------------

    /// (a) A normal modified file: both sides fetch their committed full text.
    #[test]
    fn get_file_diff_fetches_both_sides_for_a_modified_file() {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("modified.txt"), "line1\nline2\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);

        git(repo, &["checkout", "-b", "feature"]);
        fs::write(repo.join("modified.txt"), "line1\nline2 changed\n").unwrap();
        git(repo, &["commit", "-am", "feature change"]);

        let params = base_params(repo, "main", "feature");
        let fc = get_file_diff_impl(params, "modified.txt".to_string(), None, false).unwrap();

        assert_eq!(fc.base.as_deref(), Some("line1\nline2\n"));
        assert_eq!(fc.head.as_deref(), Some("line1\nline2 changed\n"));
        assert!(!fc.is_binary);
        assert_eq!(fc.is_too_large, None);
    }

    /// (b) Added file: base is None. Deleted file: head is None.
    #[test]
    fn get_file_diff_handles_added_and_deleted_files() {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("deleted.txt"), "bye\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);

        git(repo, &["checkout", "-b", "feature"]);
        fs::write(repo.join("added.txt"), "new content\n").unwrap();
        fs::remove_file(repo.join("deleted.txt")).unwrap();
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-m", "feature changes"]);

        let params = base_params(repo, "main", "feature");

        let added = get_file_diff_impl(params.clone(), "added.txt".to_string(), None, false)
            .unwrap();
        assert_eq!(added.base, None);
        assert_eq!(added.head.as_deref(), Some("new content\n"));

        let deleted =
            get_file_diff_impl(params, "deleted.txt".to_string(), None, false).unwrap();
        assert_eq!(deleted.base.as_deref(), Some("bye\n"));
        assert_eq!(deleted.head, None);
    }

    /// (c) Renamed file: base side is fetched under `oldPath`.
    #[test]
    fn get_file_diff_uses_old_path_for_base_side_on_rename() {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("old_name.txt"), "rename me\nkeep me stable\nline three\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);

        git(repo, &["checkout", "-b", "feature"]);
        fs::rename(repo.join("old_name.txt"), repo.join("new_name.txt")).unwrap();
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-m", "rename"]);

        let params = base_params(repo, "main", "feature");
        let fc = get_file_diff_impl(
            params,
            "new_name.txt".to_string(),
            Some("old_name.txt".to_string()),
            false,
        )
        .unwrap();

        assert_eq!(
            fc.base.as_deref(),
            Some("rename me\nkeep me stable\nline three\n")
        );
        assert_eq!(
            fc.head.as_deref(),
            Some("rename me\nkeep me stable\nline three\n")
        );
    }

    /// (d) Staged vs unstaged scope must diverge when the index and working
    /// tree disagree.
    #[test]
    fn get_file_diff_distinguishes_staged_from_unstaged_scope() {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("divergent.txt"), "base\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);

        git(repo, &["checkout", "-b", "feature"]);
        // Staged change.
        fs::write(repo.join("divergent.txt"), "staged\n").unwrap();
        git(repo, &["add", "divergent.txt"]);
        // Further unstaged change on top of the staged one.
        fs::write(repo.join("divergent.txt"), "unstaged\n").unwrap();

        let mut staged_params = base_params(repo, "main", "feature");
        staged_params.source_scope = SourceScope::Staged;
        let staged =
            get_file_diff_impl(staged_params, "divergent.txt".to_string(), None, false).unwrap();
        assert_eq!(staged.base.as_deref(), Some("base\n"));
        assert_eq!(staged.head.as_deref(), Some("staged\n"));

        let mut unstaged_params = base_params(repo, "main", "feature");
        unstaged_params.source_scope = SourceScope::Unstaged;
        let unstaged =
            get_file_diff_impl(unstaged_params, "divergent.txt".to_string(), None, false)
                .unwrap();
        assert_eq!(unstaged.base.as_deref(), Some("base\n"));
        assert_eq!(unstaged.head.as_deref(), Some("unstaged\n"));
    }

    /// (e) A file whose working-tree side exceeds 1MB trips the size guard
    /// unless `force` is set.
    #[test]
    fn get_file_diff_applies_size_guard_and_force_override() {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("big.txt"), "small\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);

        git(repo, &["checkout", "-b", "feature"]);
        let big_content = "x".repeat(MAX_FILE_DIFF_BYTES as usize + 1);
        fs::write(repo.join("big.txt"), &big_content).unwrap();
        // Left unstaged on purpose: sourceScope=Unstaged reads the working tree.

        let mut params = base_params(repo, "main", "feature");
        params.source_scope = SourceScope::Unstaged;

        let guarded =
            get_file_diff_impl(params.clone(), "big.txt".to_string(), None, false).unwrap();
        assert_eq!(guarded.base, None);
        assert_eq!(guarded.head, None);
        assert_eq!(guarded.is_too_large, Some(true));
        assert_eq!(guarded.size_bytes, Some(big_content.len() as u64));

        let forced = get_file_diff_impl(params, "big.txt".to_string(), None, true).unwrap();
        assert_eq!(forced.base.as_deref(), Some("small\n"));
        assert_eq!(forced.head.as_deref(), Some(big_content.as_str()));
        assert_eq!(forced.is_too_large, None);
    }

    /// (f) Binary content (NUL byte present) suppresses both sides' text and
    /// sets `isBinary`.
    #[test]
    fn get_file_diff_detects_binary_content() {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("bin.dat"), "hello\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);

        git(repo, &["checkout", "-b", "feature"]);
        fs::write(repo.join("bin.dat"), [0u8, 159, 146, 150, 0, 1, 2, 3]).unwrap();
        git(repo, &["commit", "-am", "binary change"]);

        let params = base_params(repo, "main", "feature");
        let fc = get_file_diff_impl(params, "bin.dat".to_string(), None, false).unwrap();

        assert!(fc.is_binary);
        assert_eq!(fc.base, None);
        assert_eq!(fc.head, None);
    }

    /// (g) A path escaping the repository root must be rejected, not read.
    #[test]
    fn get_file_diff_rejects_path_traversal() {
        let dir = init_repo();
        let repo = dir.path();

        fs::write(repo.join("a.txt"), "x\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);

        let mut params = base_params(repo, "main", "main");
        params.source_scope = SourceScope::Unstaged;

        let err = get_file_diff_impl(
            params.clone(),
            "../../../../etc/passwd".to_string(),
            None,
            false,
        )
        .unwrap_err();
        assert!(err.contains("traversal"), "unexpected error: {err}");

        let err_old_path =
            get_file_diff_impl(params, "a.txt".to_string(), Some("../secret".to_string()), false)
                .unwrap_err();
        assert!(err_old_path.contains("traversal"), "unexpected error: {err_old_path}");
    }
}
