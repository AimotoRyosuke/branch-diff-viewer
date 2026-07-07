//! Tauri IPC commands exposed by the git service.
//!
//! Every command here is **read-only**: it never runs a git subcommand that
//! mutates the index, working tree, or config (DESIGN.md 1 / 8). Repository
//! paths and refs are ref/path values only, and all commands are invoked
//! through `Command`'s argv array (never a shell) with a trailing `--`
//! pathspec separator on every `diff` invocation (DESIGN.md 4.0 / 8).

use std::path::{Path, PathBuf};

use super::parse::{merge_entries, parse_name_status, parse_numstat};
use super::process::{git_version, run_git, stderr_trimmed, stdout_trimmed};
use super::types::{
    CompareMode, DiffParams, DiffSummary, DiffTotals, FileContents, RepoInfo, SourceScope,
};

const DIFF_GLOBAL_ARGS: &[&str] = &["-c", "core.quotepath=false", "-c", "core.fsmonitor=false"];
const DIFF_COMMON_ARGS: &[&str] = &["diff", "--no-color", "--no-ext-diff", "-M", "-z"];

/// 1MB size guard threshold (DESIGN.md 4.3/4.4).
const MAX_FILE_DIFF_BYTES: u64 = 1_048_576;
/// Bytes inspected for a NUL byte to decide `isBinary` (DESIGN.md 4.4 task step 1).
const BINARY_SNIFF_BYTES: usize = 8000;

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
    if params.compare_mode != CompareMode::MergeBase {
        return Err(
            "this build only supports compareMode=\"merge-base\" (Phase 1 scope)".to_string(),
        );
    }
    if params.source_scope != SourceScope::Committed {
        return Err(
            "this build only supports sourceScope=\"committed\" (Phase 1 scope)".to_string(),
        );
    }

    let repo = Path::new(&params.repo_path);
    let meta = std::fs::metadata(repo).map_err(|e| format!("repoPath not accessible: {e}"))?;
    if !meta.is_dir() {
        return Err("repoPath is not a directory".to_string());
    }

    let mut warnings = Vec::new();

    let mb = compute_merge_base(repo, &params.target, &params.source, &mut warnings)?;
    let ignore_whitespace = params.options.ignore_whitespace.unwrap_or(true);

    // `-w` is intentionally never passed to `--name-status`: empirically (git
    // 2.50) `--name-status -w` still lists whitespace-only-changed files
    // (name-status only compares blob ids, it never runs the line-level
    // algorithm that `-w` affects), so passing it there would be a no-op at
    // best and misleading in intent. `--numstat -w`, by contrast, correctly
    // drops those files. `merge_entries` reconciles the two by path and,
    // when `ignore_whitespace` is set, treats a name-status entry with no
    // numstat match as "hidden by -w" rather than an error (DESIGN.md 3.5).
    let name_status_out = run_diff(repo, "--name-status", &mb, &params.source, false)?;
    if !name_status_out.status.success() {
        return Err(format!(
            "git diff --name-status failed: {}",
            stderr_trimmed(&name_status_out)
        ));
    }

    let numstat_out = run_diff(repo, "--numstat", &mb, &params.source, ignore_whitespace)?;
    if !numstat_out.status.success() {
        return Err(format!(
            "git diff --numstat failed: {}",
            stderr_trimmed(&numstat_out)
        ));
    }

    let name_entries = parse_name_status(&name_status_out.stdout)?;
    let numstat_entries = parse_numstat(&numstat_out.stdout)?;
    let files = merge_entries(name_entries, numstat_entries, ignore_whitespace)?;

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
        omitted_untracked: None,
        warnings,
    })
}

/// `git merge-base <target> <source>`; takes the first line when multiple
/// merge bases exist (criss-cross merge) and records a warning
/// (DESIGN.md 4.1 / 7).
fn compute_merge_base(
    repo: &Path,
    target: &str,
    source: &str,
    warnings: &mut Vec<String>,
) -> Result<String, String> {
    let out = run_git(repo, &["merge-base", target, source])?;
    if !out.status.success() {
        return Err(format!(
            "git merge-base failed (unrelated histories or unknown ref?): {}",
            stderr_trimmed(&out)
        ));
    }
    let stdout = stdout_trimmed(&out);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    match lines.len() {
        0 => Err("no merge base found between target and source".to_string()),
        1 => Ok(lines[0].to_string()),
        _ => {
            warnings.push(
                "multiple merge bases found (criss-cross merge); using the first one".to_string(),
            );
            Ok(lines[0].to_string())
        }
    }
}

fn run_diff(
    repo: &Path,
    stat_flag: &str,
    mb: &str,
    source: &str,
    ignore_whitespace: bool,
) -> Result<std::process::Output, String> {
    let mut args: Vec<&str> = Vec::with_capacity(DIFF_GLOBAL_ARGS.len() + DIFF_COMMON_ARGS.len() + 6);
    args.extend_from_slice(DIFF_GLOBAL_ARGS);
    args.extend_from_slice(DIFF_COMMON_ARGS);
    if ignore_whitespace {
        args.push("-w");
    }
    args.push(stat_flag);
    args.push(mb);
    args.push(source);
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

    let base_rev = match params.compare_mode {
        CompareMode::MergeBase => {
            let mut warnings = Vec::new();
            compute_merge_base(repo, &params.target, &params.source, &mut warnings)?
        }
        // DESIGN.md 4.1/4.2: "tips" compares against the target branch tip
        // directly rather than the merge-base.
        CompareMode::Tips => params.target.clone(),
    };
    // Renames read the base side under the old path (task step 1).
    let base_path = old_path.unwrap_or_else(|| path.clone());
    let base_side = Side::Blob(format!("{base_rev}:{base_path}"));

    let head_side = match params.source_scope {
        SourceScope::Committed => Side::Blob(format!("{}:{}", params.source, path)),
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

    #[test]
    fn get_diff_summary_rejects_unsupported_modes() {
        let dir = init_repo();
        let repo = dir.path();
        fs::write(repo.join("a.txt"), "x\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "base"]);

        let mut params = base_params(repo, "main", "main");
        params.compare_mode = CompareMode::Tips;
        let err = get_diff_summary_impl(params).unwrap_err();
        assert!(err.contains("merge-base"));

        let mut params2 = base_params(repo, "main", "main");
        params2.source_scope = SourceScope::Unstaged;
        let err2 = get_diff_summary_impl(params2).unwrap_err();
        assert!(err2.contains("committed"));
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
