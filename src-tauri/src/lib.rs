mod git_service;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            git_service::commands::validate_repo,
            git_service::commands::get_diff_summary,
            git_service::commands::get_file_diff,
            git_service::branches::list_branches,
            git_service::recent::get_recent_projects,
            git_service::recent::add_recent_project,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
