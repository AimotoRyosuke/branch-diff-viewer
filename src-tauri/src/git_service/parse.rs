//! Parsing of NUL-separated (`-z`) `git diff --name-status` /
//! `git diff --numstat` output, and reconciliation between the two.
//!
//! With `-z`, git terminates every path with a NUL byte instead of quoting
//! special characters. For `--name-status`, a rename/copy record is the
//! status code (e.g. `R100`) followed by two NUL-terminated paths (old, new)
//! instead of one, and the status code itself tells us unambiguously which
//! shape to expect.
//!
//! `--numstat` has no separate status code, but empirically (verified
//! against real `git diff -z --numstat` output, git 2.50) a rename/copy
//! record serializes as `"<add>\t<del>\t"` immediately followed by a NUL
//! (i.e. an *empty* third field), then the old path, then the new path, each
//! its own NUL-terminated token — e.g. `"1\t1\t\0old.txt\0new.txt\0"`. A
//! plain record instead carries the path directly in that third field, e.g.
//! `"1\t1\tfile.txt\0"`. The empty-third-field is therefore a self-describing
//! signal: numstat can be parsed without any shape hint borrowed from
//! name-status.
//!
//! That self-describing property matters for Hide-whitespace (`-w`): a real
//! git quirk (also verified empirically) is that `--name-status -w` does
//! *not* drop whitespace-only-changed files (name-status only compares blob
//! ids, never running the line-level algorithm `-w` affects), while
//! `--numstat -w` correctly drops them. So the two streams can legitimately
//! disagree on which paths are present when `-w` is requested, and
//! reconciliation is done by path (keyed on the final/new path) rather than
//! by position (DESIGN.md 3.5 / 4.3, task step 4).

use std::collections::HashMap;

use super::types::{DiffFile, DiffFileStatus};

/// One entry parsed from `--name-status -z`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameStatusEntry {
    pub status_code: String,
    /// One path normally; two (`[old, new]`) for renames/copies.
    pub paths: Vec<String>,
}

/// One entry parsed from `--numstat -z`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumstatEntry {
    pub additions: Option<i64>,
    pub deletions: Option<i64>,
    /// One path normally; two (`[old, new]`) for renames/copies.
    pub paths: Vec<String>,
}

/// Splits a NUL-terminated byte stream into UTF-8 (lossy) tokens, dropping
/// the trailing empty token produced by the final terminator.
fn split_nul(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect()
}

pub fn parse_name_status(bytes: &[u8]) -> Result<Vec<NameStatusEntry>, String> {
    let tokens = split_nul(bytes);
    let mut entries = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let status = tokens[i].clone();
        let is_rename_or_copy = status.starts_with('R') || status.starts_with('C');
        if is_rename_or_copy {
            if i + 2 >= tokens.len() {
                return Err(format!(
                    "malformed --name-status output: expected old/new path after status '{status}'"
                ));
            }
            entries.push(NameStatusEntry {
                status_code: status,
                paths: vec![tokens[i + 1].clone(), tokens[i + 2].clone()],
            });
            i += 3;
        } else {
            if i + 1 >= tokens.len() {
                return Err(format!(
                    "malformed --name-status output: expected path after status '{status}'"
                ));
            }
            entries.push(NameStatusEntry {
                status_code: status,
                paths: vec![tokens[i + 1].clone()],
            });
            i += 2;
        }
    }
    Ok(entries)
}

/// Parses `--numstat -z` output. Self-describing: a record is a rename/copy
/// iff the numeric-fields token's third field is empty (see module docs).
pub fn parse_numstat(bytes: &[u8]) -> Result<Vec<NumstatEntry>, String> {
    let tokens = split_nul(bytes);
    let mut entries = Vec::new();
    let mut ti = 0;
    while ti < tokens.len() {
        let first = &tokens[ti];
        // splitn(3, ..) is safe even if the path itself contains a literal tab.
        let mut parts = first.splitn(3, '\t');
        let add_s = parts
            .next()
            .ok_or_else(|| "numstat record missing additions field".to_string())?;
        let del_s = parts
            .next()
            .ok_or_else(|| "numstat record missing deletions field".to_string())?;
        let third = parts
            .next()
            .ok_or_else(|| "numstat record missing path field".to_string())?
            .to_string();
        ti += 1;

        let paths = if third.is_empty() {
            // Rename/copy: old and new path follow as their own tokens.
            let old_path = tokens
                .get(ti)
                .ok_or("numstat record is a rename/copy but is missing the old path")?;
            let new_path = tokens
                .get(ti + 1)
                .ok_or("numstat record is a rename/copy but is missing the new path")?;
            let paths = vec![old_path.clone(), new_path.clone()];
            ti += 2;
            paths
        } else {
            vec![third]
        };

        let additions = parse_numstat_count(add_s)?;
        let deletions = parse_numstat_count(del_s)?;
        entries.push(NumstatEntry {
            additions,
            deletions,
            paths,
        });
    }
    Ok(entries)
}

fn parse_numstat_count(field: &str) -> Result<Option<i64>, String> {
    if field == "-" {
        Ok(None)
    } else {
        field
            .parse::<i64>()
            .map(Some)
            .map_err(|_| format!("invalid numstat count: '{field}'"))
    }
}

fn map_status(code: &str, paths: &[String]) -> Result<(DiffFileStatus, Option<String>, String), String> {
    let first_char = code.chars().next().ok_or("empty status code in name-status output")?;
    match first_char {
        'A' => Ok((DiffFileStatus::Added, None, paths[0].clone())),
        'M' => Ok((DiffFileStatus::Modified, None, paths[0].clone())),
        'D' => Ok((DiffFileStatus::Deleted, None, paths[0].clone())),
        // Copy detection is not enabled (`-M` only, no `-C`), so 'C' should not
        // normally appear; mapped defensively to the same shape as rename
        // (both carry an old + new path) since the enum has no dedicated
        // "copied" variant (task step 3 / DESIGN.md 5 M-1,M-6).
        'R' | 'C' => Ok((DiffFileStatus::Renamed, Some(paths[0].clone()), paths[1].clone())),
        'T' => Ok((DiffFileStatus::Typechange, None, paths[0].clone())),
        'U' => Ok((DiffFileStatus::Unmerged, None, paths[0].clone())),
        _ => Ok((DiffFileStatus::Other, None, paths[0].clone())),
    }
}

/// Reconciles `--name-status` and `--numstat` records (from the same diff
/// invocation) into the final `DiffFile` list, matched by the current
/// ("new") path rather than by position — see module docs for why: with
/// Hide-whitespace (`-w`) requested, `--numstat` may legitimately omit
/// whitespace-only-changed files that `--name-status` still lists.
///
/// When `allow_numstat_gaps` is `false`, every name-status entry must have a
/// matching numstat entry (used when `-w` was not requested, where any gap
/// indicates a real parsing/reconciliation bug rather than expected
/// filtering). When `true`, name-status entries with no numstat match are
/// silently dropped (they were whitespace-only changes hidden by `-w`).
pub fn merge_entries(
    name_status: Vec<NameStatusEntry>,
    numstat: Vec<NumstatEntry>,
    allow_numstat_gaps: bool,
) -> Result<Vec<DiffFile>, String> {
    let mut by_new_path: HashMap<&str, &NumstatEntry> = HashMap::with_capacity(numstat.len());
    for nu in &numstat {
        let new_path = nu.paths.last().expect("numstat entry always has >= 1 path");
        by_new_path.insert(new_path.as_str(), nu);
    }

    let mut files = Vec::with_capacity(name_status.len());
    for ns in name_status {
        let new_path = ns.paths.last().expect("name-status entry always has >= 1 path");
        let nu = match by_new_path.get(new_path.as_str()) {
            Some(nu) => *nu,
            None if allow_numstat_gaps => continue,
            None => {
                return Err(format!(
                    "numstat has no matching record for name-status path '{new_path}'"
                ))
            }
        };
        if ns.paths != nu.paths {
            return Err(format!(
                "name-status/numstat path mismatch for '{new_path}': {:?} vs {:?}",
                ns.paths, nu.paths
            ));
        }
        let (status, old_path, path) = map_status(&ns.status_code, &ns.paths)?;
        let is_binary = nu.additions.is_none() || nu.deletions.is_none();
        files.push(DiffFile {
            path,
            old_path,
            status,
            additions: nu.additions,
            deletions: nu.deletions,
            is_binary,
            is_untracked: None,
        });
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_name_status_records() {
        let raw = b"A\0new.txt\0M\0changed.txt\0D\0removed.txt\0";
        let entries = parse_name_status(raw).unwrap();
        assert_eq!(
            entries,
            vec![
                NameStatusEntry { status_code: "A".into(), paths: vec!["new.txt".into()] },
                NameStatusEntry { status_code: "M".into(), paths: vec!["changed.txt".into()] },
                NameStatusEntry { status_code: "D".into(), paths: vec!["removed.txt".into()] },
            ]
        );
    }

    #[test]
    fn parses_rename_name_status_record() {
        let raw = b"R100\0old.txt\0new.txt\0";
        let entries = parse_name_status(raw).unwrap();
        assert_eq!(
            entries,
            vec![NameStatusEntry {
                status_code: "R100".into(),
                paths: vec!["old.txt".into(), "new.txt".into()]
            }]
        );
    }

    #[test]
    fn parses_numstat_with_binary_and_rename_records() {
        // record 0: plain, record 1: rename (empty third field signals the
        // old/new pair follows as separate tokens), record 2: binary.
        let raw = b"3\t1\ta.txt\05\t2\t\0old.txt\0new.txt\0-\t-\timg.png\0";
        let entries = parse_numstat(raw).unwrap();
        assert_eq!(
            entries,
            vec![
                NumstatEntry { additions: Some(3), deletions: Some(1), paths: vec!["a.txt".into()] },
                NumstatEntry {
                    additions: Some(5),
                    deletions: Some(2),
                    paths: vec!["old.txt".into(), "new.txt".into()]
                },
                NumstatEntry { additions: None, deletions: None, paths: vec!["img.png".into()] },
            ]
        );
    }

    #[test]
    fn merge_entries_rejects_mismatched_paths_when_gaps_disallowed() {
        let ns = vec![NameStatusEntry { status_code: "M".into(), paths: vec!["a.txt".into()] }];
        let nu = vec![NumstatEntry { additions: Some(1), deletions: Some(0), paths: vec!["b.txt".into()] }];
        let err = merge_entries(ns, nu, false).unwrap_err();
        assert!(err.contains("no matching record"), "unexpected error: {err}");
    }

    #[test]
    fn merge_entries_drops_whitespace_only_files_when_gaps_allowed() {
        // name-status still lists ws.txt (its -w blind spot); numstat correctly
        // omitted it because the only difference was whitespace.
        let ns = vec![NameStatusEntry { status_code: "M".into(), paths: vec!["ws.txt".into()] }];
        let nu = vec![];
        let files = merge_entries(ns, nu, true).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn merge_entries_maps_status_and_binary_flag() {
        let ns = vec![
            NameStatusEntry { status_code: "A".into(), paths: vec!["a.txt".into()] },
            NameStatusEntry {
                status_code: "R095".into(),
                paths: vec!["old.txt".into(), "new.txt".into()],
            },
        ];
        let nu = vec![
            NumstatEntry { additions: None, deletions: None, paths: vec!["a.txt".into()] },
            NumstatEntry {
                additions: Some(4),
                deletions: Some(1),
                paths: vec!["old.txt".into(), "new.txt".into()],
            },
        ];
        let files = merge_entries(ns, nu, false).unwrap();
        assert_eq!(files[0].status, DiffFileStatus::Added);
        assert!(files[0].is_binary);
        assert_eq!(files[1].status, DiffFileStatus::Renamed);
        assert_eq!(files[1].old_path.as_deref(), Some("old.txt"));
        assert_eq!(files[1].path, "new.txt");
        assert!(!files[1].is_binary);
    }
}
