//! Tauri IPC commands exposed by the git service.
//!
//! Every command here is **read-only**: it never runs a git subcommand that
//! mutates the index, working tree, or config (DESIGN.md 1 / 8). Repository
//! paths and refs are ref/path values only, and all commands are invoked
//! through `Command`'s argv array (never a shell) with a trailing `--`
//! pathspec separator on every `diff` invocation (DESIGN.md 4.0 / 8).

use std::path::Path;

use super::parse::{merge_entries, parse_name_status, parse_numstat};
use super::process::{git_version, run_git, stderr_trimmed, stdout_trimmed};
use super::types::{CompareMode, DiffParams, DiffSummary, DiffTotals, RepoInfo, SourceScope};

const DIFF_GLOBAL_ARGS: &[&str] = &["-c", "core.quotepath=false", "-c", "core.fsmonitor=false"];
const DIFF_COMMON_ARGS: &[&str] = &["diff", "--no-color", "--no-ext-diff", "-M", "-z"];

#[tauri::command]
pub fn validate_repo(path: String) -> Result<RepoInfo, String> {
    validate_repo_impl(&path)
}

#[tauri::command]
pub fn get_diff_summary(params: DiffParams) -> Result<DiffSummary, String> {
    get_diff_summary_impl(params)
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
}
