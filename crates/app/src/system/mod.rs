//! OS integration: tray icon (Show AgentDeck / Quit, waiting count tooltip, close to tray), desktop
//! notifications + sound + phone push, taskbar flash, stuck / remind / rate-limit watchers, updater,
//! autostart, global voice shortcut (forwards to `agentdeck_core::voice::on_shortcut`), single instance.
//! Original: src-tauri/src/{tray,notify}.rs, src/lib/{notify,tray,updater}.ts, src/store/runtime.ts
//! (listenAgentStatus, watchAgents, scheduleRateLimitRetry), src/components/Permission/permissions.ts
//! (permission notification), src/components/Update/UpdateButton.tsx, src/components/Settings/AppControls.tsx.

mod autostart;
mod cli;
mod controls;
mod hotkey;
mod instance;
mod notify;
mod tray;
mod updater;
mod watch;
mod window;

// Public API for the title bar (layout), Settings (settings) and pane chrome (panes); unused until they wire it.
#[allow(unused_imports)]
pub use {
    autostart::{autostart_enabled, set_autostart},
    controls::AppControls,
    hotkey::set_voice_shortcut,
    notify::notify,
    updater::{Update, UpdateButton, UpdatePhase, Updater, updater},
    window::{hide_main, main_deck, main_window},
};
pub use cli::run as run_cli;
pub use instance::single_instance;
pub use window::{is_main_focused, quit, show_main};

use gpui_kit::*;

use crate::layout::DeckView;

/// Wires everything to the first `DeckView` (the main window) once it exists.
/// Starts or stops the phone page according to the settings (startup, Settings > General).
pub fn apply_phone(deck: &crate::state::Deck) -> Result<(), String> {
    let remote = &deck.settings.remote;
    agentdeck_core::remote::set_phone(&deck.backend, remote.enabled.then(|| (remote.port, remote.token.clone())))
}

pub fn init(cx: &mut App) {
    updater::init(cx); // before any view so UpdateButton / AppControls can observe it
    cx.observe_new::<DeckView>(|view, window, cx| {
        if window::is_wired(cx) {
            return; // detached pane windows may host views too; only the first one is the main window
        }
        let Some(window) = window else {
            return;
        };
        let deck = view.deck().clone();
        window::wire(window, deck.clone(), cx);
        instance::listen(cx);
        if let Err(e) = apply_phone(deck.read(cx)) {
            eprintln!("{e}");
        }
        tray::init(cx);
        hotkey::init(&deck, cx);
        watch::init(deck, cx);
    })
    .detach();
}
