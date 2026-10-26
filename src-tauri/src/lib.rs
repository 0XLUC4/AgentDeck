mod agents;
mod hooks_server;
mod pty;
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
        .setup(|app| {
            let hooks = hooks_server::start(app.handle().clone());
            app.manage(AppState {
                panes: pty::Panes::default(),
                active_pane: Mutex::new(None),
                hooks,
            });
            voice::init(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            pty::spawn_pane,
            pty::write_to_pane,
            pty::resize_pane,
            pty::kill_pane,
            pty::set_active_pane,
            agents::detect_agents,
            agents::uninstall_hooks,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
