//! AgentDeck: multi-agent terminal dashboard, pure Rust + GPUI.
//! The backend lives in `agentdeck_core`; this crate is the UI. See PORT.md for module ownership.

// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// The shell is being filled in module by module; stubs are not all wired yet.
#![allow(dead_code)]

mod automations;
mod browser;
mod git;
mod history;
mod layout;
mod orchestrator;
mod palette;
mod panes;
mod settings;
mod skills;
mod state;
mod system;
mod terminal;
mod theme;
mod voice;

use gpui_kit::component::TitleBar;
use gpui_kit::*;

fn main() {
    // GPUI's DirectComposition surface is drawn above child windows: the browser's native webview
    // would stay hidden behind it (black panel). Without it GPUI draws in the window itself.
    // shortcut: gives up transparent window backgrounds; revisit if GPUI lets children sit above.
    #[cfg(windows)]
    if std::env::var_os("GPUI_DISABLE_DIRECT_COMPOSITION").is_none() {
        // SAFETY: first statement of main, no other thread exists yet.
        unsafe { std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "1") };
    }
    if let Some(code) = system::run_cli() {
        std::process::exit(code);
    }
    let Some(_instance) = system::single_instance() else {
        eprintln!("AgentDeck is already running");
        return;
    };
    // AGENTDECK_DATA_DIR: run against another data folder (tests, side by side with the Tauri app).
    let data_dir = match std::env::var_os("AGENTDECK_DATA_DIR")
        .map(|dir| Ok(std::path::PathBuf::from(dir)))
        .unwrap_or_else(agentdeck_core::AppHandle::default_data_dir)
    {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("AgentDeck cannot start: {e}");
            return;
        }
    };
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let backend = agentdeck_core::start(data_dir.clone());
            let deck = cx.new(|cx| state::Deck::new(backend, data_dir, cx));
            let appearance = deck.read(cx).settings.appearance.clone();
            theme::apply(&appearance, None, cx);
            theme::set_zoom(deck.read(cx).settings.ui_zoom, cx);
            layout::init(cx);
            for init in [
                automations::init,
                browser::init,
                git::init,
                history::init,
                orchestrator::init,
                palette::init,
                settings::init,
                skills::init,
                system::init,
                voice::init,
            ] {
                init(cx);
            }
            // Restores the saved sessions; spawns run async, after the window below exists.
            panes::start(&deck, cx);
            cx.on_app_quit(|_| async { agentdeck_core::shutdown() })
                .detach();
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("AgentDeck".into()),
                    ..TitleBar::title_bar_options()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1400.0), px(900.0)),
                    cx,
                ))),
                window_min_size: Some(size(px(800.0), px(500.0))),
                ..TitleBar::window_options()
            };
            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| layout::DeckView::new(deck.clone(), window, cx))
            });
            if let Err(e) = opened {
                eprintln!("AgentDeck cannot open its window: {e}");
                cx.quit();
            }
        });
}
