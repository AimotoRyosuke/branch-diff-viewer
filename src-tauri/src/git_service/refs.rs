//! ref normalization and validation shared by `get_diff_summary` and
//! `get_file_diff` (DESIGN.md 3.2 / 8 H-3).
//!
//! `target`/`source` arrive over IPC as either a short branch name (`main`,
//! `feature/foo`) or an already fully-qualified ref (`refs/heads/main`,
//! `refs/remotes/origin/main`). Before either is handed to any `git`
//! subcommand, [`normalize_ref`] turns it into a fully-qualified ref:
//! - a short name is resolved by checking `refs/heads/<name>` then
//!   `refs/remotes/<name>`, in that order (DESIGN.md 3.2);
//! - an already fully-qualified ref is passed through as-is (still
//!   syntax-validated).
//!
//! This removes the ambiguity/argv-injection risk of a raw branch name that
//! happens to start with `-` (which a naive `git diff <name> ...` would
//! otherwise parse as an option) and matches DESIGN.md's "内部表現" note in
//! 3.2.

use std::path::Path;

use super::process::run_git;

/// Normalizes `raw` (short or fully-qualified) into a fully-qualified
/// `refs/heads/<name>` or `refs/remotes/<name>` ref, validating it against
/// `git check-ref-format --allow-onelevel` (DESIGN.md 8 H-3) along the way.
///
/// A fully-qualified `refs/heads/...` / `refs/remotes/...` input is returned
/// as-is once validated (DESIGN.md 8: "full ref ならそのまま"). A short name
/// is resolved by existence-checking `refs/heads/<raw>` then
/// `refs/remotes/<raw>`, in that order, and only the winning candidate is
/// returned; if neither exists, this errors rather than silently passing the
/// short name through to `git`.
pub fn normalize_ref(repo: &Path, raw: &str) -> Result<String, String> {
    if raw.is_empty() {
        return Err("ref must not be empty".to_string());
    }
    // Defense-in-depth beyond `check-ref-format` (DESIGN.md 8 H-3): a value
    // starting with `-` risks being parsed as a command-line option by some
    // downstream `git` invocation. `check-ref-format` alone does not reject
    // this for every shape (e.g. `refs/heads/-foo` is syntactically legal),
    // so reject it unconditionally here, up front.
    if raw.starts_with('-') {
        return Err(format!("ref must not start with '-': '{raw}'"));
    }

    if raw.starts_with("refs/heads/") || raw.starts_with("refs/remotes/") {
        validate_ref_format(repo, raw)?;
        return Ok(raw.to_string());
    }

    validate_ref_format(repo, raw)?;

    let heads_candidate = format!("refs/heads/{raw}");
    if ref_exists(repo, &heads_candidate)? {
        return Ok(heads_candidate);
    }
    let remotes_candidate = format!("refs/remotes/{raw}");
    if ref_exists(repo, &remotes_candidate)? {
        return Ok(remotes_candidate);
    }

    Err(format!(
        "branch not found: '{raw}' (checked refs/heads/{raw} and refs/remotes/{raw})"
    ))
}

/// `git check-ref-format --allow-onelevel <candidate>` (DESIGN.md 8 H-3):
/// pure syntax validation, rejects illegal characters, `..`, a trailing
/// `.lock`/`/`, etc. `--allow-onelevel` permits a bare one-component name
/// (e.g. `main`) in addition to a full `a/b` path.
fn validate_ref_format(repo: &Path, candidate: &str) -> Result<(), String> {
    let out = run_git(repo, &["check-ref-format", "--allow-onelevel", candidate])?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("invalid ref name: '{candidate}'"))
    }
}

/// Whether `full_ref` (already fully-qualified) exists in `repo`.
fn ref_exists(repo: &Path, full_ref: &str) -> Result<bool, String> {
    let out = run_git(repo, &["show-ref", "--verify", "--quiet", full_ref])?;
    Ok(out.status.success())
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

    #[test]
    fn normalizes_short_local_branch_to_refs_heads() {
        let dir = init_repo();
        let repo = dir.path();
        git(repo, &["branch", "feature"]);

        assert_eq!(normalize_ref(repo, "main").unwrap(), "refs/heads/main");
        assert_eq!(normalize_ref(repo, "feature").unwrap(), "refs/heads/feature");
    }

    #[test]
    fn normalizes_short_remote_tracking_branch_to_refs_remotes_when_no_local_match() {
        let dir = init_repo();
        let repo = dir.path();
        let head_sha = {
            let out = run_git(repo, &["rev-parse", "HEAD"]).unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        git(repo, &["update-ref", "refs/remotes/origin/release", &head_sha]);

        assert_eq!(
            normalize_ref(repo, "origin/release").unwrap(),
            "refs/remotes/origin/release"
        );
    }

    #[test]
    fn prefers_local_branch_over_remote_tracking_branch_of_the_same_short_name() {
        let dir = init_repo();
        let repo = dir.path();
        let head_sha = {
            let out = run_git(repo, &["rev-parse", "HEAD"]).unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        // A local branch and a same-named remote-tracking ref both exist;
        // refs/heads/ must win (DESIGN.md 3.2 resolution order).
        git(repo, &["branch", "shared"]);
        git(repo, &["update-ref", "refs/remotes/origin/shared", &head_sha]);

        assert_eq!(normalize_ref(repo, "shared").unwrap(), "refs/heads/shared");
    }

    #[test]
    fn passes_through_already_fully_qualified_refs() {
        let dir = init_repo();
        let repo = dir.path();
        assert_eq!(
            normalize_ref(repo, "refs/heads/main").unwrap(),
            "refs/heads/main"
        );
    }

    #[test]
    fn rejects_nonexistent_short_branch() {
        let dir = init_repo();
        let repo = dir.path();
        let err = normalize_ref(repo, "does-not-exist").unwrap_err();
        assert!(err.contains("not found"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_ref_starting_with_dash() {
        let dir = init_repo();
        let repo = dir.path();
        let err = normalize_ref(repo, "-foo").unwrap_err();
        assert!(err.contains("must not start with '-'"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_illegal_ref_syntax() {
        let dir = init_repo();
        let repo = dir.path();
        for bad in ["a..b", "foo.lock", "foo bar", "foo~1", "foo^HEAD"] {
            let err = normalize_ref(repo, bad).unwrap_err();
            assert!(
                err.contains("invalid ref name") || err.contains("not found"),
                "expected '{bad}' to be rejected, got: {err}"
            );
        }
    }

    #[test]
    fn rejects_empty_ref() {
        let dir = init_repo();
        let repo = dir.path();
        let err = normalize_ref(repo, "").unwrap_err();
        assert!(err.contains("empty"), "unexpected error: {err}");
    }
}
