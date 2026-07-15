//! Persisted UI settings.
//!
//! Stored as `<app_config_dir>/settings.json`, read and written directly
//! with `std::fs` — same flat-JSON approach as `recent.rs` (DESIGN.md 8.1).
//! Never sent anywhere off-device.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::Manager;

const SETTINGS_FILE_NAME: &str = "settings.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiSettings {
    pub hide_whitespace: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        // DESIGN.md 3.5: hide whitespace defaults to ON.
        Self { hide_whitespace: true }
    }
}

#[tauri::command]
pub fn get_ui_settings(app: tauri::AppHandle) -> Result<UiSettings, String> {
    get_ui_settings_impl(&config_dir(&app)?)
}

#[tauri::command]
pub fn set_ui_settings(app: tauri::AppHandle, settings: UiSettings) -> Result<(), String> {
    set_ui_settings_impl(&config_dir(&app)?, &settings)
}

fn config_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|e| format!("failed to resolve app config directory: {e}"))
}

fn get_ui_settings_impl(config_dir: &Path) -> Result<UiSettings, String> {
    let file = config_dir.join(SETTINGS_FILE_NAME);
    match std::fs::read(&file) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(UiSettings::default()),
        Err(e) => Err(format!("failed to read settings file: {e}")),
    }
}

fn set_ui_settings_impl(config_dir: &Path, settings: &UiSettings) -> Result<(), String> {
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("failed to create app config directory: {e}"))?;
    let json = serde_json::to_vec_pretty(settings)
        .map_err(|e| format!("failed to serialize settings: {e}"))?;
    std::fs::write(config_dir.join(SETTINGS_FILE_NAME), json)
        .map_err(|e| format!("failed to write settings file: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn get_ui_settings_defaults_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let settings = get_ui_settings_impl(dir.path()).unwrap();
        assert!(settings.hide_whitespace);
    }

    #[test]
    fn set_then_get_round_trips() {
        let dir = TempDir::new().unwrap();
        set_ui_settings_impl(dir.path(), &UiSettings { hide_whitespace: false }).unwrap();
        let settings = get_ui_settings_impl(dir.path()).unwrap();
        assert!(!settings.hide_whitespace);
    }

    #[test]
    fn get_ui_settings_defaults_when_file_is_corrupt() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(SETTINGS_FILE_NAME), b"not json").unwrap();
        let settings = get_ui_settings_impl(dir.path()).unwrap();
        assert!(settings.hide_whitespace);
    }

    #[test]
    fn unknown_fields_fall_back_to_defaults_per_field() {
        // A settings file written by a future version with extra keys should
        // still load the keys this version knows about.
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join(SETTINGS_FILE_NAME),
            br#"{"hideWhitespace": false, "futureOption": 42}"#,
        )
        .unwrap();
        let settings = get_ui_settings_impl(dir.path()).unwrap();
        assert!(!settings.hide_whitespace);
    }
}
