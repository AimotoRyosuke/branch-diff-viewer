//! `list_branches` IPC command (DESIGN.md 3.2 / 4.3): enumerates local and
//! remote-tracking branches, the checked-out branch (HEAD), and the
//! last-fetch timestamp. Read-only — this app never runs `git fetch`, so the
//! remote-tracking list only ever reflects whatever was last fetched outside
//! the app (DESIGN.md 3.2).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::commands::symbolic_ref_short;
use super::process::{run_git, stderr_trimmed, stdout_trimmed};
use super::types::{BranchList, BranchRef};

#[tauri::command]
pub fn list_branches(path: String) -> Result<BranchList, String> {
    list_branches_impl(&path)
}

fn list_branches_impl(path: &str) -> Result<BranchList, String> {
    let repo = Path::new(path);
    let meta = std::fs::metadata(repo).map_err(|e| format!("path not accessible: {e}"))?;
    if !meta.is_dir() {
        return Err("path is not a directory".to_string());
    }

    // `%(symref)` is empty for a normal ref and non-empty for a symbolic ref
    // like `refs/remotes/origin/HEAD` — used below to exclude it (DESIGN.md
    // 3.2 / 4.3 M-3). Fields are NUL-separated within a record; records are
    // themselves newline-separated (ref names can't contain newlines, so
    // this doesn't need `-z` framing the way path lists do).
    let out = run_git(
        repo,
        &[
            "for-each-ref",
            "refs/heads",
            "refs/remotes",
            "--format=%(refname:short)%00%(refname)%00%(symref)",
        ],
    )?;
    if !out.status.success() {
        return Err(format!("git for-each-ref failed: {}", stderr_trimmed(&out)));
    }

    let mut local = Vec::new();
    let mut remote = Vec::new();
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.split('\n') {
        if line.is_empty() {
            continue;
        }
        let mut fields = line.splitn(3, '\0');
        let short = fields.next().unwrap_or("").to_string();
        let full = fields.next().unwrap_or("").to_string();
        let symref = fields.next().unwrap_or("");
        if full.is_empty() {
            continue;
        }
        // Symbolic refs (origin/HEAD etc.) are excluded (DESIGN.md 3.2 / M-3).
        if !symref.is_empty() {
            continue;
        }
        let is_remote = full.starts_with("refs/remotes/");
        let branch_ref = BranchRef { short, full, is_remote };
        if is_remote {
            remote.push(branch_ref);
        } else {
            local.push(branch_ref);
        }
    }

    let current = symbolic_ref_short(repo)?;
    let last_fetch = last_fetch_time(repo)?;

    Ok(BranchList { local, remote, current, last_fetch })
}

/// `.git/FETCH_HEAD` mtime as ISO 8601, `None` if it doesn't exist (never
/// fetched) — DESIGN.md 3.2. Resolves the real git-dir via
/// `rev-parse --git-dir` rather than assuming `<repo>/.git` so this also
/// works from a linked worktree.
fn last_fetch_time(repo: &Path) -> Result<Option<String>, String> {
    let out = run_git(repo, &["rev-parse", "--git-dir"])?;
    if !out.status.success() {
        return Ok(None);
    }
    let git_dir_raw = stdout_trimmed(&out);
    let git_dir = if Path::new(&git_dir_raw).is_absolute() {
        PathBuf::from(&git_dir_raw)
    } else {
        repo.join(&git_dir_raw)
    };
    let fetch_head = git_dir.join("FETCH_HEAD");
    match std::fs::metadata(&fetch_head) {
        Ok(meta) => {
            let modified = meta
                .modified()
                .map_err(|e| format!("failed to read FETCH_HEAD mtime: {e}"))?;
            let dt: DateTime<Utc> = modified.into();
            Ok(Some(dt.to_rfc3339()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("failed to stat FETCH_HEAD: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

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
        fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-m", "initial"]);
        dir
    }

    fn head_sha(repo: &Path) -> String {
        let out = run_git(repo, &["rev-parse", "HEAD"]).unwrap();
        stdout_trimmed(&out)
    }

    #[test]
    fn classifies_local_and_remote_and_excludes_origin_head() {
        let dir = init_repo();
        let repo = dir.path();
        git(repo, &["branch", "feature"]);
        let sha = head_sha(repo);
        git(repo, &["update-ref", "refs/remotes/origin/main", &sha]);
        git(repo, &["update-ref", "refs/remotes/origin/feature", &sha]);
        // origin/HEAD: a symbolic ref, must be excluded from the result.
        git(repo, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);

        let result = list_branches_impl(repo.to_str().unwrap()).unwrap();

        let local_names: Vec<&str> = result.local.iter().map(|b| b.short.as_str()).collect();
        assert!(local_names.contains(&"main"));
        assert!(local_names.contains(&"feature"));
        assert_eq!(result.local.len(), 2);
        assert!(result.local.iter().all(|b| !b.is_remote));
        assert!(result.local.iter().all(|b| b.full.starts_with("refs/heads/")));

        let remote_names: Vec<&str> = result.remote.iter().map(|b| b.short.as_str()).collect();
        assert!(remote_names.contains(&"origin/main"));
        assert!(remote_names.contains(&"origin/feature"));
        assert!(
            !remote_names.contains(&"origin/HEAD"),
            "origin/HEAD (a symref) must be excluded, got {remote_names:?}"
        );
        assert_eq!(result.remote.len(), 2);
        assert!(result.remote.iter().all(|b| b.is_remote));
        assert!(result.remote.iter().all(|b| b.full.starts_with("refs/remotes/")));

        assert_eq!(result.current.as_deref(), Some("main"));
    }

    #[test]
    fn reports_none_current_on_detached_head() {
        let dir = init_repo();
        let repo = dir.path();
        let sha = head_sha(repo);
        git(repo, &["checkout", &sha]);

        let result = list_branches_impl(repo.to_str().unwrap()).unwrap();
        assert_eq!(result.current, None);
    }

    #[test]
    fn last_fetch_is_none_when_fetch_head_absent_and_set_after_it_exists() {
        let dir = init_repo();
        let repo = dir.path();

        let before = list_branches_impl(repo.to_str().unwrap()).unwrap();
        assert_eq!(before.last_fetch, None);

        // Simulate a fetch by writing FETCH_HEAD directly (no network access
        // in tests) — only the file's existence/mtime matters here.
        fs::write(repo.join(".git/FETCH_HEAD"), "").unwrap();

        let after = list_branches_impl(repo.to_str().unwrap()).unwrap();
        assert!(after.last_fetch.is_some());
        // Must parse as RFC3339 (ISO 8601).
        DateTime::parse_from_rfc3339(&after.last_fetch.unwrap()).unwrap();
    }
}
