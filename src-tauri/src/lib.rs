mod agents;
mod git;
mod history;
mod hooks_server;
mod mcp;
mod pty;
mod tray;
mod trust;
mod usage;
mod voice;
mod workspace;

use std::sync::Mutex;

use tauri::Manager;

/// Shared backend state, managed by Tauri.
pub struct AppState {
    pub panes: pty::Panes,
    /// Pane that receives voice transcripts.
    pub active_pane: Mutex<Option<String>>,
    pub hooks: hooks_server::HookServer,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .on_window_event(tray::on_window_event)
        .setup(|app| {
            let hooks = hooks_server::start(app.handle().clone());
            app.manage(AppState {
                panes: pty::Panes::default(),
                active_pane: Mutex::new(None),
                hooks,
            });
            voice::init(app.handle())?;
            tray::init(app.handle())?;
            history::init(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            pty::spawn_pane,
            pty::write_to_pane,
            pty::resize_pane,
            pty::kill_pane,
            pty::list_panes,
            pty::set_active_pane,
            agents::detect_agents,
            agents::uninstall_hooks,
            hooks_server::permissions::answer_permission,
            mcp::mcp_register,
            mcp::mcp_respond,
            mcp::mcp_set_paused,
            git::git_status,
            git::git_diff,
            git::git_diff_trees,
            git::git_commit,
            git::git_merge_worktree,
            git::git_remove_worktree,
            git::git_pane_dir,
            git::git_checkpoint,
            git::git_list_checkpoints,
            git::git_restore_checkpoint,
            git::git_compare,
            voice::voice_start,
            voice::voice_stop,
            voice::voice_send,
            voice::voice_get_config,
            voice::voice_set_config,
            voice::voice_set_api_key,
            voice::voice_list_models,
            voice::voice_download_model,
            workspace::list_open_projects,
            workspace::inspect_folder,
            tray::set_tray_status,
            tray::set_close_to_tray,
            usage::usage_summary,
            usage::pane_usage,
            history::history_search,
            history::history_read,
            history::history_get_enabled,
            history::history_set_enabled,
            history::history_clear,
            history::journal::journal_append,
            history::journal::journal_read,
            history::journal::journal_git_commits,
            history::journal::write_text_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
