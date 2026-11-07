//! Terminal history search (Ctrl+Shift+F), daily journal (tasks + commits, Markdown export),
//! usage and cost (today / 7d / 30d, per agent / workspace / day) and the usage chip, budget watch.
//! Original: src/components/{History,Journal,Usage}, src/lib/{budget,dialogs}.ts.
//! Backend: agentdeck_core::{history,usage}.
//!
//! Dialogs open through the actions `SearchHistory` (Ctrl+Shift+F), `OpenUsage` and `OpenJournal`
//! (title bar buttons, command palette). The open one lives in a global and is drawn by [`overlay`],
//! which the root view renders as its last child. The pane header embeds [`UsageChip`]; the settings
//! dialog may embed [`HistorySettings`].

mod budget;
mod journal;
mod search;
mod settings;
mod usage;

use agentdeck_core::agents::AgentKind;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;

// Public interface for the root view, pane header, palette and settings dialog.
#[allow(unused_imports)]
pub use journal::JournalDialog;
#[allow(unused_imports)]
pub use search::HistoryDialog;
#[allow(unused_imports)]
pub use settings::{HistorySaved, HistorySettings};
#[allow(unused_imports)]
pub use usage::{UsageChip, UsageDialog, usage_details, usage_label};

use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

actions!(agentdeck, [SearchHistory, OpenUsage, OpenJournal, CloseHistoryDialog]);

const MODAL_CONTEXT: &str = "HistoryModal";

/// Registers this module's key bindings and action handlers, and starts the journal recorder and
/// budget watch once the main `DeckView` exists. Called once from `main`.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-f", SearchHistory, None),
        KeyBinding::new("escape", CloseHistoryDialog, Some(MODAL_CONTEXT)),
    ]);
    cx.on_action(|_: &SearchHistory, cx| with_window(DialogId::History, cx));
    cx.on_action(|_: &OpenUsage, cx| with_window(DialogId::Usage, cx));
    cx.on_action(|_: &OpenJournal, cx| with_window(DialogId::Journal, cx));
    cx.on_action(|_: &CloseHistoryDialog, cx| close_dialog(cx));
    cx.set_global(OpenDialog(None));
    cx.observe_new::<crate::layout::DeckView>(|view, _, cx| {
        if cx.has_global::<MainDeck>() {
            return;
        }
        let deck = view.deck().clone();
        cx.set_global(MainDeck(deck.clone()));
        journal::start_recorder(&deck, cx);
        budget::start_watch(deck, cx);
    })
    .detach();
}

/// The main window's deck, captured when its `DeckView` is created.
struct MainDeck(Entity<Deck>);
impl Global for MainDeck {}

/// Title bar dialogs; at most one is open (original `lib/dialogs.ts`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DialogId {
    Usage,
    History,
    Journal,
}

struct OpenDialog(Option<AnyView>);
impl Global for OpenDialog {}

/// Opens on the active window, outside of the current dispatch (actions arrive mid-update).
fn with_window(id: DialogId, cx: &mut App) {
    let Some(window) = cx.active_window() else { return };
    cx.defer(move |cx| {
        if let Err(e) = window.update(cx, |_, window, cx| open_dialog(id, window, cx)) {
            eprintln!("history: cannot open dialog: {e}");
        }
    });
}

/// Opens one of the dialogs (replacing the open one) with fresh state.
pub fn open_dialog(id: DialogId, window: &mut Window, cx: &mut App) {
    let Some(deck) = cx.try_global::<MainDeck>().map(|m| m.0.clone()) else { return };
    let view: AnyView = match id {
        DialogId::Usage => cx.new(|cx| UsageDialog::new(deck, window, cx)).into(),
        DialogId::History => cx.new(|cx| HistoryDialog::new(deck, window, cx)).into(),
        DialogId::Journal => cx.new(|cx| JournalDialog::new(deck, window, cx)).into(),
    };
    cx.set_global(OpenDialog(Some(view)));
    window.refresh();
}

pub fn close_dialog(cx: &mut App) {
    cx.set_global(OpenDialog(None));
    cx.refresh_windows();
}

/// The open dialog of this module, drawn over the whole window (it positions itself).
pub fn overlay(cx: &App) -> Option<AnyElement> {
    let view = cx.try_global::<OpenDialog>()?.0.clone()?;
    Some(view.into_any_element())
}

/// Ghost icon button with a tooltip (original `IconButton`).
fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .icon(Icon::from(icon))
        .ghost()
        .small()
        .tooltip(label)
        .on_click(on_click)
}

/// Modal frame drawn over the whole window: title, extra header buttons, close button, scrolling body.
/// Esc and the close button close it. `width_rem` is the CSS `min(<n>rem, 94vw)`.
fn modal(
    title: impl Into<SharedString>,
    width_rem: f32,
    actions: Vec<AnyElement>,
    focus: &FocusHandle,
    body: impl IntoElement,
    window: &Window,
    cx: &App,
) -> impl IntoElement {
    let p = palette(cx);
    let viewport = window.viewport_size();
    let width = ui(width_rem * 16.0).to_pixels(window.rem_size()).min(viewport.width * 0.94);
    let dialog = div()
        .key_context(MODAL_CONTEXT)
        .track_focus(focus)
        .w(width)
        .max_h(viewport.height * 0.86)
        .flex()
        .flex_col()
        .rounded_xl()
        .border_1()
        .border_color(hsla(p.line))
        .bg(hsla(p.panel))
        .text_color(hsla(p.fg))
        .text_size(ui(13.0))
        .child(
            div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(hsla(p.line))
                .px_5()
                .py_3()
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_size(ui(15.0))
                        .font_weight(FontWeight::MEDIUM)
                        .child(title.into()),
                )
                .children(actions)
                .child(icon_button("modal-close", IconName::X, "Close", |_, _, cx| close_dialog(cx))),
        )
        .child(
            div()
                .id("modal-body")
                .overflow_y_scroll()
                .min_h_0()
                .flex_1()
                .px_5()
                .py_4()
                .child(body),
        );
    deferred(
        anchored().position(point(px(0.0), px(0.0))).child(
            div()
                .id("modal-backdrop")
                .occlude()
                .w(viewport.width)
                .h(viewport.height)
                .flex()
                .items_center()
                .justify_center()
                .bg(hsla_a(p.gutter, 0.7))
                .child(dialog),
        ),
    )
    .with_priority(2)
}

/// Segmented control: one pressed button (original `Segmented`).
fn segmented<T: Copy + PartialEq + 'static>(
    options: &[(T, &'static str)],
    value: T,
    on_change: impl Fn(T, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let p = palette(cx);
    let on_change = std::rc::Rc::new(on_change);
    div()
        .flex()
        .gap_1()
        .rounded_lg()
        .bg(hsla(p.base))
        .p_1()
        .children(options.iter().enumerate().map(|(i, &(id, label))| {
            let on_change = on_change.clone();
            let item = div()
                .id(("segment", i))
                .h_7()
                .px_3()
                .flex()
                .items_center()
                .rounded_md()
                .text_xs()
                .cursor_pointer()
                .on_click(move |_, _, cx| on_change(id, cx))
                .child(label);
            if id == value {
                item.bg(hsla(p.raised)).font_weight(FontWeight::MEDIUM).text_color(hsla(p.fg))
            } else {
                item.text_color(hsla(p.muted)).hover(|s| s.text_color(hsla(p.fg)))
            }
        }))
}

/// Tooltip builder with plain (multi-line) text.
fn text_tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}

/// The agent logo (owned by `panes`).
fn agent_mark(kind: AgentKind, size: f32, cx: &App) -> Icon {
    crate::panes::agent_logo(kind, size, None, &palette(cx))
}

/// Parses an agent id ("claude", "opencode"...) as stored in run metadata and usage keys.
fn agent_kind(id: &str) -> Option<AgentKind> {
    serde_json::from_value(serde_json::Value::String(id.to_owned())).ok()
}
