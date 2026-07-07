//! Thin wrapper around `std::process::Command` for invoking the system `git`.
//!
//! Follows DESIGN.md 4.0 / 8: arguments are always passed as an argv array
//! (never through a shell), stdin is closed, and the environment disables
//! locks / terminal prompts. This module never writes to the repository.

use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Runs `git -C <repo> <args...>` with the security-hardening options
/// required by DESIGN.md 4.0 / 8 (no shell, no stdin, locks/prompts
/// disabled). `args` is everything after `-C <repo>` (global opts,
/// subcommand, subcommand opts, pathspec separator, etc).
pub fn run_git<S: AsRef<std::ffi::OsStr>>(repo: &Path, args: &[S]) -> Result<Output, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo);
    cmd.args(args);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    // Index-lock/side-effect avoidance and no interactive prompts (DESIGN.md 4.0/8).
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    cmd.env("GIT_TERMINAL_PROMPT", "0");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Prevent a console window flash on Windows (DESIGN.md 4.0/8.1).
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.output()
        .map_err(|e| format!("failed to execute git: {e}"))
}

/// Runs `git --version` (no repo context required).
pub fn git_version() -> Result<Output, String> {
    let mut cmd = Command::new("git");
    cmd.arg("--version");
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.output()
        .map_err(|e| format!("failed to execute git --version: {e}"))
}

/// `stdout` trimmed and lossily decoded as UTF-8 (DESIGN.md 7: never fail on
/// non-UTF-8 output).
pub fn stdout_trimmed(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn stderr_trimmed(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}
