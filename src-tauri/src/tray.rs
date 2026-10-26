//! System tray icon, "close to tray" behaviour and a clean exit that kills every PTY.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, Window, WindowEvent};

use crate::AppState;

const TRAY_ID: &str = "main";
const MAIN_WINDOW: &str = "main";

/// When set, closing the main window hides it instead of quitting. Off by default.
#[derive(Default)]
pub struct CloseToTray(AtomicBool);

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    app.manage(CloseToTray::default());
    let show = MenuItem::with_id(app, "show", "Afficher AgentDeck", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &separator, &quit_item])?;

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("AgentDeck")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn toggle_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    if visible && window.is_focused().unwrap_or(false) {
        let _ = window.hide();
    } else {
        show_main(app);
    }
}

/// Kills every agent process, then exits. The hidden voice window would otherwise keep the app alive.
pub fn quit(app: &AppHandle) {
    let state = app.state::<AppState>();
    let ids: Vec<String> = match state.panes.0.lock() {
        Ok(panes) => panes.keys().cloned().collect(),
        Err(_) => Vec::new(),
    };
    for id in ids {
        let _ = crate::pty::kill_pane(state.clone(), id);
    }
    app.exit(0);
}

/// Main window close: hide when "close to tray" is on, otherwise quit the whole app.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        let app = window.app_handle();
        if app.state::<CloseToTray>().0.load(Ordering::Relaxed) {
            let _ = window.hide();
        } else {
            quit(app);
        }
    }
}

#[tauri::command]
pub fn set_close_to_tray(state: State<'_, CloseToTray>, enabled: bool) {
    state.0.store(enabled, Ordering::Relaxed);
}

/// Tooltip shows how many agents wait for an answer.
#[tauri::command]
pub fn set_tray_status(app: AppHandle, waiting: u32) -> Result<(), String> {
    let tooltip = if waiting > 0 {
        format!("AgentDeck — {waiting} en attente")
    } else {
        "AgentDeck".to_string()
    };
    app.tray_by_id(TRAY_ID)
        .ok_or("tray not found")?
        .set_tooltip(Some(tooltip))
        .map_err(|e| e.to_string())
}
