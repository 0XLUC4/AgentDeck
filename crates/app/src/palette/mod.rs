//! Command palette (Ctrl+K / Ctrl+Shift+P), broadcast a prompt to several agents (Ctrl+Shift+B),
//! per-pane prompt queue and session template picker.
//! Original: src/components/{CommandPalette,Broadcast,Queue,Templates}, src/lib/recentPrompts.ts,
//! src/store/{queue,templates}.ts. Logic: agentdeck_core::logic::{queue,templates}.
//!
//! Every dialog opens through gpui-component's dialog layer (`WindowExt::open_dialog`), so the
//! layout does not have to render anything for this module.

mod broadcast;
mod command;
pub mod fuzzy;
mod queue;
mod templates;

use std::time::Duration;

use agentdeck_core::AppState;
use agentdeck_core::logic::queue::{SUBMIT_DELAY_MS, prompt_text};
use agentdeck_core::persist::{JsonStore, LOCAL_FILE};
use agentdeck_core::{AppHandle, pty};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::{Dialog, DialogEntrance};
use gpui_kit::*;

use crate::layout::DeckView;
use crate::state::Deck;
use crate::theme::{hsla, palette, ui};

pub use broadcast::{Broadcast, open_broadcast, open_race};
pub use command::CommandPalette;
#[allow(unused_imports)] // public API for the panes and settings modules
pub use queue::{QUEUE_HOVER_GROUP, QueueDialog, open_queue, queue_button, send_next};
#[allow(unused_imports)]
pub use templates::{
    TemplatePicker, delete_template, new_session_from_template, open_template_picker, open_template_picker_for,
    rename_template, save_session_as_template,
};

actions!(agentdeck, [OpenPalette, OpenPaletteAlt, OpenBroadcast]);

/// localStorage key of the web build, kept in local.json.
const RECENT_PROMPTS_KEY: &str = "recent-prompts";

/// Module state: the deck, which dialogs are open (weak: a closed dialog drops its view),
/// recently run palette items and the queue runner's last seen statuses.
#[derive(Default)]
struct PaletteGlobal {
    deck: Option<Entity<Deck>>,
    palette: Option<WeakEntity<CommandPalette>>,
    broadcast: Option<WeakEntity<Broadcast>>,
    picker: Option<WeakEntity<TemplatePicker>>,
    /// Recently run palette item ids, most recent first. In memory: resets with the app.
    recent_items: Vec<String>,
    queue_runner: queue::Runner,
}

impl Global for PaletteGlobal {}

/// Registers this module's key bindings and globals. Called once from `main`.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-k", OpenPalette, None),
        KeyBinding::new("ctrl-shift-p", OpenPaletteAlt, None),
        KeyBinding::new("ctrl-shift-b", OpenBroadcast, None),
    ]);
    command::bind_keys(cx);
    templates::bind_keys(cx);
    cx.set_global(PaletteGlobal::default());
    cx.on_action(|_: &OpenPalette, cx| with_window(cx, command::toggle));
    cx.on_action(|_: &OpenPaletteAlt, cx| with_window(cx, command::toggle));
    cx.on_action(|_: &OpenBroadcast, cx| with_window(cx, open_broadcast));
    // The main window's root view carries the deck: attach to it once it exists.
    cx.observe_new::<DeckView>(|view, _, cx| attach(view.deck().clone(), cx))
        .detach();
}

fn attach(deck: Entity<Deck>, cx: &mut App) {
    if cx.global::<PaletteGlobal>().deck.is_some() {
        return;
    }
    cx.global_mut::<PaletteGlobal>().deck = Some(deck.clone());
    load_recent_prompts(&deck, cx);
    queue::start_runner(&deck, cx);
}

fn global_deck(cx: &App) -> Option<Entity<Deck>> {
    cx.try_global::<PaletteGlobal>().and_then(|g| g.deck.clone())
}

/// Runs `f` on the active window, outside of the current dispatch (actions arrive mid-update).
fn with_window(cx: &mut App, f: impl FnOnce(Entity<Deck>, &mut Window, &mut App) + 'static) {
    let (Some(deck), Some(window)) = (global_deck(cx), cx.active_window()) else {
        return;
    };
    cx.defer(move |cx| {
        if let Err(e) = window.update(cx, |_, window, cx| f(deck, window, cx)) {
            eprintln!("palette: window gone: {e}");
        }
    });
}

/// Opens `view` as a modal in the shared AgentDeck dialog style (`rounded-xl border bg-panel`).
/// `top`: offset from the window top, None to center.
fn open_modal<V: Render>(
    view: Entity<V>,
    width: f32,
    top: Option<Pixels>,
    focus: FocusHandle,
    window: &mut Window,
    cx: &mut App,
) {
    let first = std::cell::Cell::new(true);
    window.open_dialog(cx, move |dialog: Dialog, window, cx| {
        if first.replace(false) {
            focus.focus(window, cx);
        }
        let p = palette(cx);
        let dialog = dialog
            .w(ui(width).to_pixels(window.rem_size()))
            .p_0()
            .gap_0()
            .close_button(false)
            .entrance(DialogEntrance::None)
            .bg(hsla(p.panel))
            .border_1()
            .border_color(hsla(p.line))
            .rounded(ui(agentdeck_core::logic::theme::RADIUS_XL))
            .text_color(hsla(p.fg))
            .child(view.clone());
        match top {
            Some(top) => dialog.margin_top(top),
            None => dialog.centered(),
        }
    });
}

/// Dispatches an action owned by another module by name (`agentdeck::Name`). Toasts when no
/// module registered it yet, so the palette never fails silently.
fn dispatch_named(deck: &Entity<Deck>, name: &str, label: &str, window: &mut Window, cx: &mut App) {
    match cx.build_action(&format!("agentdeck::{name}"), None) {
        Ok(action) => window.dispatch_action(action, cx),
        Err(_) => deck.update(cx, |d, cx| d.toast(format!("{label}: not available yet."), cx)),
    }
}

/// Pane the pane-level actions target: the focused one if it is in the visible session, else
/// that session's first (Wall/jump.ts `currentPane`).
fn current_pane(deck: &Deck) -> Option<(String, String)> {
    let tab = deck.state.active_tab()?;
    let pane = match &deck.state.active_pane_id {
        Some(id) if tab.pane_ids.contains(id) => id.clone(),
        _ => tab.pane_ids.first()?.clone(),
    };
    Some((tab.id.clone(), pane))
}

/// Types the prompt, then presses Enter in a separate write (Broadcast/send.ts `sendPrompt`).
async fn send_prompt(
    backend: AppHandle,
    executor: BackgroundExecutor,
    pane_id: String,
    text: String,
) -> Result<(), String> {
    let state = backend
        .try_state::<AppState>()
        .ok_or("the backend is not ready")?;
    pty::write_to_pane(&state, pane_id.clone(), prompt_text(&text))?;
    executor.timer(Duration::from_millis(SUBMIT_DELAY_MS)).await;
    pty::write_to_pane(&state, pane_id, "\r".into())
}

/// Remembers a prompt sent to an agent (newest first, saved in local.json like the web build's
/// localStorage). Also for the terminal and voice modules.
pub fn remember_prompt(deck: &Entity<Deck>, prompt: &str, cx: &mut App) {
    let changed = deck.update(cx, |d, _| {
        agentdeck_core::logic::queue::remember_prompt(&mut d.recent_prompts, prompt)
    });
    if !changed {
        return;
    }
    let (dir, list) = {
        let d = deck.read(cx);
        (d.data_dir.clone(), d.recent_prompts.clone())
    };
    cx.background_executor()
        .spawn(async move {
            // Storage failure: the list just won't survive a restart.
            let saved = JsonStore::load(&dir, LOCAL_FILE).and_then(|mut store| {
                store.set(RECENT_PROMPTS_KEY, serde_json::json!(list));
                store.save()
            });
            if let Err(e) = saved {
                eprintln!("recent prompts not saved: {e}");
            }
        })
        .detach();
}

fn load_recent_prompts(deck: &Entity<Deck>, cx: &mut App) {
    let dir = deck.read(cx).data_dir.clone();
    let list: Vec<String> = match JsonStore::load(&dir, LOCAL_FILE) {
        Ok(store) => match store.get(RECENT_PROMPTS_KEY) {
            serde_json::Value::Array(items) => items
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .take(agentdeck_core::logic::queue::MAX_RECENT_PROMPTS)
                .collect(),
            _ => Vec::new(),
        },
        Err(e) => {
            eprintln!("recent prompts not loaded: {e}");
            Vec::new()
        }
    };
    deck.update(cx, |d, _| {
        if d.recent_prompts.is_empty() {
            d.recent_prompts = list;
        }
    });
}

/// Small shared pieces of the dialogs (header with close button, buttons, agent mark).
mod ui {
    use agentdeck_core::agents::AgentKind;
    use agentdeck_core::logic::theme::Palette;
    use gpui_kit::assets::IconName;
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::prelude::FluentBuilder as _;
    use gpui_kit::*;

    use crate::theme::{hsla, ui};

    /// Dialog header: title, subtitle and an X button (IconButton "Close (Esc)").
    pub fn header(title: &str, subtitle: impl Into<SharedString>, p: &Palette) -> Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .pt_4()
            .pr_3()
            .pl_5()
            .child(
                div()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(ui(15.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title.to_owned()),
                    )
                    .child(div().text_xs().text_color(hsla(p.subtle)).truncate().child(subtitle.into())),
            )
            .child(icon_button("dialog-close", IconName::X, p).on_click(|_, window, cx| window.close_dialog(cx)))
    }

    /// Square icon button (IconButton.tsx).
    pub fn icon_button(id: impl Into<ElementId>, icon: IconName, p: &Palette) -> Stateful<Div> {
        let hover = hsla(p.hover);
        let fg = hsla(p.fg);
        div()
            .id(id)
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size(ui(28.0))
            .rounded(ui(6.0))
            .text_color(hsla(p.muted))
            .text_size(ui(15.0))
            .cursor_pointer()
            .hover(move |s| s.bg(hover).text_color(fg))
            .child(icon)
    }

    /// `chip` class of Broadcast.tsx: small bordered button.
    pub fn chip(id: impl Into<ElementId>, p: &Palette) -> Stateful<Div> {
        let hover = hsla(p.hover);
        let fg = hsla(p.fg);
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_1p5()
            .h(ui(28.0))
            .px_2p5()
            .rounded(ui(6.0))
            .border_1()
            .border_color(hsla(p.line))
            .text_xs()
            .text_color(hsla(p.muted))
            .cursor_pointer()
            .hover(move |s| s.bg(hover).text_color(fg))
    }

    /// Accent button (`bg-accent text-on-accent`), dimmed when disabled.
    pub fn primary_button(
        id: impl Into<ElementId>,
        icon: IconName,
        label: impl Into<SharedString>,
        disabled: bool,
        p: &Palette,
    ) -> Stateful<Div> {
        let hover = hsla(p.accent_hover);
        div()
            .id(id)
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .h(ui(32.0))
            .px_3()
            .rounded(ui(6.0))
            .bg(hsla(p.accent))
            .text_color(hsla(p.on_accent))
            .font_weight(FontWeight::MEDIUM)
            .text_size(ui(14.0))
            .child(icon)
            .child(label.into())
            .when(disabled, |s| s.opacity(0.4))
            .when(!disabled, |s| s.cursor_pointer().hover(move |s| s.bg(hover)))
    }

    /// Bordered secondary button (`border border-line text-muted`), dimmed when disabled.
    pub fn secondary_button(
        id: impl Into<ElementId>,
        icon: IconName,
        label: impl Into<SharedString>,
        disabled: bool,
        p: &Palette,
    ) -> Stateful<Div> {
        let hover = hsla(p.hover);
        let fg = hsla(p.fg);
        div()
            .id(id)
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_1p5()
            .h(ui(32.0))
            .px_3()
            .rounded(ui(6.0))
            .border_1()
            .border_color(hsla(p.line))
            .text_color(hsla(p.muted))
            .text_size(ui(14.0))
            .child(icon)
            .child(label.into())
            .when(disabled, |s| s.opacity(0.4))
            .when(!disabled, |s| s.cursor_pointer().hover(move |s| s.bg(hover).text_color(fg)))
    }

    /// Agent brand mark (panes module's AgentLogo).
    pub fn agent_mark(kind: AgentKind, size: f32, p: &Palette) -> impl IntoElement {
        crate::panes::agent_logo(kind, size, None, p)
    }

    /// `<kbd>` hint (`font-sans text-xs text-subtle`).
    pub fn kbd(keys: &'static str, p: &Palette) -> Div {
        div().flex_shrink_0().text_xs().text_color(hsla(p.subtle)).child(keys)
    }
}
