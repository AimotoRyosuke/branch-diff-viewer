mod git_service;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            git_service::commands::validate_repo,
            git_service::commands::get_diff_summary,
            git_service::commands::get_file_diff
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
