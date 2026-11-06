mod agents;
mod assets;
mod browser;
mod git;
mod history;
mod hooks_server;
mod mcp;
mod notes_graph;
mod notify;
mod pty;
mod skills;
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

/// Same as `additionalBrowserArgs` in tauri.conf.json: windows sharing one WebView2 environment need identical
/// arguments. They keep WebView2 rendering while the window is covered by another app (otherwise the GPU canvas is
/// dropped and terminals stay black for a while on return).
const WEBVIEW_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion --disable-backgrounding-occluded-windows --disable-renderer-backgrounding --disable-background-timer-throttling";

/// Opens a detached pane window (`pane-<id>`). From Rust because the JS API can't set the browser arguments.
#[tauri::command(async)]
fn open_pane_window(
    app: tauri::AppHandle,
    pane_id: String,
    title: String,
    geometry: Option<(f64, f64, f64, f64)>,
) -> Result<(), String> {
    if !pane_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("invalid pane id".into());
    }
    let url = format!("pane.html?pane={pane_id}");
    let mut builder = tauri::WebviewWindowBuilder::new(
        &app,
        format!("pane-{pane_id}"),
        tauri::WebviewUrl::App(url.into()),
    )
    .title(title)
    .min_inner_size(360.0, 200.0)
    .decorations(false)
    .shadow(true)
    .additional_browser_args(WEBVIEW_ARGS);
    builder = match geometry {
        Some((x, y, width, height)) => builder.position(x, y).inner_size(width, height),
        None => builder.inner_size(900.0, 600.0).center(),
    };
    builder.build().map(|_| ()).map_err(|e| e.to_string())
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
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                pty::drop_webview_sinks(window.label());
            }
            tray::on_window_event(window, event);
        })
        // A reload kills the page's output channels; it re-subscribes when it boots.
        .on_page_load(|webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Started {
                pty::drop_webview_sinks(webview.label());
            }
        })
        .setup(|app| {
            let hooks = hooks_server::start(app.handle().clone());
            mcp::hub::init(app.handle(), hooks.port);
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
            open_pane_window,
            pty::spawn_pane,
            pty::attach_pane_output,
            pty::detach_pane_output,
            pty::write_to_pane,
            pty::resize_pane,
            pty::kill_pane,
            pty::list_panes,
            pty::set_active_pane,
            agents::detect_agents,
            agents::uninstall_hooks,
            hooks_server::permissions::answer_permission,
            browser::browser_open,
            browser::browser_bounds,
            browser::browser_go,
            browser::browser_url,
            browser::browser_close,
            mcp::mcp_register,
            mcp::mcp_respond,
            mcp::mcp_set_paused,
            mcp::hub::hub_list,
            mcp::hub::hub_save,
            mcp::hub::hub_test,
            mcp::hub::hub_claude_servers,
            git::git_status,
            git::git_diff,
            git::git_diff_trees,
            git::git_commit,
            git::git_merge_worktree,
            git::git_create_pr,
            git::git_remove_worktree,
            git::git_pane_dir,
            git::git_checkpoint,
            git::git_list_checkpoints,
            git::git_restore_checkpoint,
            git::git_compare,
            git::git_upstream,
            git::git_pull,
            git::git_push,
            git::gh_list_repos,
            git::git_clone,
            voice::voice_start,
            voice::voice_stop,
            voice::voice_get_config,
            voice::voice_set_config,
            voice::voice_set_api_key,
            voice::voice_list_models,
            voice::voice_download_model,
            workspace::list_open_projects,
            workspace::inspect_folder,
            notify::push_notify,
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
            skills::skills_list,
            skills::skills_install,
            skills::skills_remove,
            skills::save_paste_image,
            skills::notes_list,
            skills::agent_file_read,
            skills::agent_file_write,
            assets::assets_list,
            assets::asset_source,
            notes_graph::notes_graph,
            notes_graph::note_read,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_, event| {
            if let tauri::RunEvent::Exit = event {
                mcp::hub::stop_all();
            }
        });
}
