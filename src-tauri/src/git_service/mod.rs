//! Read-only git CLI wrapper backing the Tauri IPC commands.
//!
//! This module never invokes a git subcommand that mutates the repository
//! (no `add`, `add -N`, `commit`, etc.) — see DESIGN.md 1 / 8.

pub mod commands;
mod parse;
mod process;
mod types;
