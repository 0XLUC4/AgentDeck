//! Main window handle, show / hide / toggle (tray), close to tray and the clean exit.

use agentdeck_core::AppState;
use gpui_kit::*;

use crate::state::Deck;

/// The main window and its deck, set once the first `DeckView` exists.
struct Main {
    window: AnyWindowHandle,
    deck: Entity<Deck>,
    /// Win32 HWND: GPUI cannot hide a window on Windows, so close to tray uses ShowWindow.
    hwnd: Option<isize>,
}

impl Global for Main {}

pub(super) fn is_wired(cx: &App) -> bool {
    cx.has_global::<Main>()
}

/// Remembers the main window and routes its close button through close to tray / quit.
pub(super) fn wire(window: &mut Window, deck: Entity<Deck>, cx: &mut App) {
    cx.set_global(Main {
        window: window.window_handle(),
        deck,
        hwnd: hwnd(window),
    });
    window.on_window_should_close(cx, |window, cx| {
        let to_tray = main_deck(cx).is_some_and(|d| d.read(cx).settings.close_to_tray);
        if !to_tray {
            quit(cx);
        } else if !set_visible(cx, false) {
            window.minimize_window();
        }
        false
    });
}

pub fn main_deck(cx: &App) -> Option<Entity<Deck>> {
    cx.try_global::<Main>().map(|m| m.deck.clone())
}

pub fn main_window(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Main>().map(|m| m.window)
}

/// True when the main window is the active (focused) window.
pub fn is_main_focused(cx: &mut App) -> bool {
    main_window(cx)
        .and_then(|w| w.update(cx, |_, window, _| window.is_window_active()).ok())
        .unwrap_or(false)
}

/// Restores, shows and focuses the main window.
pub fn show_main(cx: &mut App) {
    set_visible(cx, true);
    if let Some(w) = main_window(cx) {
        let _ = w.update(cx, |_, window, _| window.activate_window());
    }
}

pub fn hide_main(cx: &mut App) {
    if !set_visible(cx, false)
        && let Some(w) = main_window(cx)
    {
        let _ = w.update(cx, |_, window, _| window.minimize_window());
    }
}

/// Tray left click: hide when visible and focused, otherwise bring it back.
pub(super) fn toggle_main(cx: &mut App) {
    if is_visible(cx) && is_main_focused(cx) {
        hide_main(cx);
    } else {
        show_main(cx);
    }
}

/// Kills every agent process, then exits (the pill window would otherwise keep the app alive).
pub fn quit(cx: &mut App) {
    stop_agents(cx);
    cx.quit();
}

/// Update restart: same cleanup, then exit at once. The graceful GPUI quit can take longer than the
/// new exe waits for the single-instance port (closing pseudoconsoles, MCP servers), and then the
/// new exe thinks AgentDeck still runs and exits: nothing restarts.
pub fn quit_for_update(cx: &mut App) {
    stop_agents(cx);
    agentdeck_core::shutdown();
    std::process::exit(0);
}

fn stop_agents(cx: &mut App) {
    if let Some(deck) = main_deck(cx) {
        deck.update(cx, |d, cx| d.save_now(cx)); // a change inside the autosave debounce would be lost
        let backend = deck.read(cx).backend.clone();
        if let Some(state) = backend.try_state::<AppState>() {
            let ids = agentdeck_core::pty::list_panes(&state).unwrap_or_default();
            for id in ids {
                if let Err(e) = agentdeck_core::pty::kill_pane(&state, id) {
                    eprintln!("quit: kill pane: {e}");
                }
            }
        }
    }
}

#[cfg(windows)]
fn hwnd(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

#[cfg(not(windows))]
fn hwnd(_: &Window) -> Option<isize> {
    None
}

/// Removes the Windows 11 DWM frame (1px border + rounded corners) from a transparent pop-up.
#[cfg(windows)]
pub fn strip_frame(window: &Window) {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
        DwmSetWindowAttribute,
    };
    let Some(hwnd) = hwnd(window) else { return };
    let set = |attr, value: u32| {
        // SAFETY: hwnd is live (we are inside its build callback); value outlives the call.
        unsafe { DwmSetWindowAttribute(hwnd as _, attr as _, (&raw const value).cast(), 4) }
    };
    // Failures are cosmetic (older Windows): the pill still works.
    set(DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE);
    set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND as u32);
}

#[cfg(not(windows))]
pub fn strip_frame(_: &Window) {}

/// Shows or hides the main window at the OS level. False when unsupported (no HWND).
#[cfg(windows)]
fn set_visible(cx: &App, visible: bool) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOW, ShowWindow};
    let Some(hwnd) = cx.try_global::<Main>().and_then(|m| m.hwnd) else {
        return false;
    };
    // SAFETY: hwnd is the live main window handle; ShowWindow tolerates a stale handle.
    unsafe { ShowWindow(hwnd as _, if visible { SW_SHOW } else { SW_HIDE }) };
    true
}

#[cfg(not(windows))]
fn set_visible(_: &App, _: bool) -> bool {
    false
}

#[cfg(windows)]
fn is_visible(cx: &App) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible};
    let Some(hwnd) = cx.try_global::<Main>().and_then(|m| m.hwnd) else {
        return true;
    };
    // SAFETY: read-only queries on a window handle.
    unsafe { IsWindowVisible(hwnd as _) != 0 && IsIconic(hwnd as _) == 0 }
}

#[cfg(not(windows))]
fn is_visible(_: &App) -> bool {
    true
}
