//! Recently opened project paths (DESIGN.md 3.1 / 8.1).
//!
//! Persisted as a small JSON array in `<app_config_dir>/recent.json`, read
//! and written directly with `std::fs` — no `tauri-plugin-store` (DESIGN.md
//! 8.1: the store plugin is only needed "if used", and a flat JSON file is
//! simpler here). Never sent anywhere off-device.

use std::path::{Path, PathBuf};

use tauri::Manager;

const RECENT_FILE_NAME: &str = "recent.json";
const RECENT_LIMIT: usize = 10;

#[tauri::command]
pub fn get_recent_projects(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    get_recent_projects_impl(&config_dir(&app)?)
}

/// Adds `path` to the front of the recent-projects list (deduplicating and
/// capping at [`RECENT_LIMIT`]) and returns the updated list.
#[tauri::command]
pub fn add_recent_project(app: tauri::AppHandle, path: String) -> Result<Vec<String>, String> {
    add_recent_project_impl(&config_dir(&app)?, &path)
}

fn config_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|e| format!("failed to resolve app config directory: {e}"))
}

fn get_recent_projects_impl(config_dir: &Path) -> Result<Vec<String>, String> {
    let file = config_dir.join(RECENT_FILE_NAME);
    match std::fs::read(&file) {
        Ok(bytes) => Ok(serde_json::from_slice::<Vec<String>>(&bytes).unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("failed to read recent projects file: {e}")),
    }
}

fn add_recent_project_impl(config_dir: &Path, project_path: &str) -> Result<Vec<String>, String> {
    if project_path.is_empty() {
        return Err("project path must not be empty".to_string());
    }
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("failed to create app config directory: {e}"))?;

    let mut list = get_recent_projects_impl(config_dir)?;
    list.retain(|p| p != project_path);
    list.insert(0, project_path.to_string());
    list.truncate(RECENT_LIMIT);

    let json = serde_json::to_vec_pretty(&list)
        .map_err(|e| format!("failed to serialize recent projects: {e}"))?;
    std::fs::write(config_dir.join(RECENT_FILE_NAME), json)
        .map_err(|e| format!("failed to write recent projects file: {e}"))?;
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn get_recent_projects_is_empty_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let list = get_recent_projects_impl(dir.path()).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn add_recent_project_prepends_newest_first() {
        let dir = TempDir::new().unwrap();
        add_recent_project_impl(dir.path(), "/repo/a").unwrap();
        add_recent_project_impl(dir.path(), "/repo/b").unwrap();
        let list = get_recent_projects_impl(dir.path()).unwrap();
        assert_eq!(list, vec!["/repo/b", "/repo/a"]);
    }

    #[test]
    fn add_recent_project_dedupes_by_moving_existing_entry_to_front() {
        let dir = TempDir::new().unwrap();
        add_recent_project_impl(dir.path(), "/repo/a").unwrap();
        add_recent_project_impl(dir.path(), "/repo/b").unwrap();
        add_recent_project_impl(dir.path(), "/repo/a").unwrap();
        let list = get_recent_projects_impl(dir.path()).unwrap();
        assert_eq!(list, vec!["/repo/a", "/repo/b"]);
    }

    #[test]
    fn add_recent_project_caps_at_ten_dropping_the_oldest() {
        let dir = TempDir::new().unwrap();
        for i in 0..12 {
            add_recent_project_impl(dir.path(), &format!("/repo/{i}")).unwrap();
        }
        let list = get_recent_projects_impl(dir.path()).unwrap();
        assert_eq!(list.len(), RECENT_LIMIT);
        assert_eq!(list[0], "/repo/11");
        assert_eq!(list[RECENT_LIMIT - 1], "/repo/2");
        assert!(!list.contains(&"/repo/0".to_string()));
        assert!(!list.contains(&"/repo/1".to_string()));
    }

    #[test]
    fn add_recent_project_rejects_empty_path() {
        let dir = TempDir::new().unwrap();
        let err = add_recent_project_impl(dir.path(), "").unwrap_err();
        assert!(err.contains("empty"), "unexpected error: {err}");
    }
}
