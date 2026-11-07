//! System tray icon: "Show AgentDeck" / "Quit" menu, left click toggles the window, tooltip shows
//! how many agents wait for an answer.

use gpui_kit::*;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const ICON_PNG: &[u8] = include_bytes!("tray.png");

/// Keeps the tray icon alive (dropping it removes it from the tray).
struct Tray(TrayIcon);

impl Global for Tray {}

enum TrayAction {
    Show,
    Quit,
    Toggle,
}

pub(super) fn init(cx: &mut App) {
    match build() {
        Ok(tray) => cx.set_global(Tray(tray)),
        Err(e) => {
            eprintln!("tray icon unavailable: {e}");
            return;
        }
    }
    let (tx, rx) = smol::channel::unbounded::<TrayAction>();
    let menu_tx = tx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let action = match event.id.as_ref() {
            "show" => TrayAction::Show,
            "quit" => TrayAction::Quit,
            _ => return,
        };
        let _ = menu_tx.send_blocking(action);
    }));
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            let _ = tx.send_blocking(TrayAction::Toggle);
        }
    }));
    cx.spawn(async move |cx| {
        while let Ok(action) = rx.recv().await {
            cx.update(|cx| match action {
                TrayAction::Show => super::show_main(cx),
                TrayAction::Quit => super::quit(cx),
                TrayAction::Toggle => super::window::toggle_main(cx),
            });
        }
    })
    .detach();
}

fn build() -> Result<TrayIcon, String> {
    let show = MenuItem::with_id("show", "Show AgentDeck", true, None);
    let quit = MenuItem::with_id("quit", "Quit", true, None);
    let menu = Menu::with_items(&[&show, &PredefinedMenuItem::separator(), &quit]).map_err(|e| e.to_string())?;
    let image = image::load_from_memory(ICON_PNG).map_err(|e| e.to_string())?.to_rgba8();
    let (width, height) = image.dimensions();
    let icon = Icon::from_rgba(image.into_raw(), width, height).map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_id("main")
        .with_tooltip("AgentDeck")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(icon)
        .build()
        .map_err(|e| e.to_string())
}

/// Tooltip text for the waiting count.
pub(super) fn tooltip(waiting: usize) -> String {
    if waiting > 0 {
        format!("AgentDeck - {waiting} waiting")
    } else {
        "AgentDeck".to_owned()
    }
}

pub(super) fn set_waiting(cx: &App, waiting: usize) {
    if let Some(Tray(tray)) = cx.try_global::<Tray>()
        && let Err(e) = tray.set_tooltip(Some(tooltip(waiting)))
    {
        eprintln!("tray status failed: {e}");
    }
}
