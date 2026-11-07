//! Settings rows: terminal history on/off (stored by the backend) and "Clear history".
//! Original: `components/History/HistorySettings.tsx`. The settings dialog embeds this view and may
//! subscribe to its [`HistorySaved`] events to show its "Saved" / error state.

use agentdeck_core::history::{history_clear, history_get_enabled, history_set_enabled};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;

use crate::state::Deck;
use crate::theme::{hsla, palette, ui};

/// Outcome of a change made by [`HistorySettings`] (the original `save(promise)`).
pub struct HistorySaved(pub Result<(), String>);

pub struct HistorySettings {
    deck: Entity<Deck>,
    enabled: Option<bool>,
    _task: Task<()>,
}

impl EventEmitter<HistorySaved> for HistorySettings {}

impl HistorySettings {
    pub fn new(deck: Entity<Deck>, cx: &mut Context<Self>) -> Self {
        let task = cx.spawn(async move |this, cx| {
            let enabled = cx.background_spawn(async { history_get_enabled() }).await;
            let _ = this.update(cx, |this, cx| {
                this.enabled = Some(enabled);
                cx.notify();
            });
        });
        Self { deck, enabled: None, _task: task }
    }

    /// Runs a backend change off the UI thread, then reports it (toast on failure).
    fn save(&mut self, job: impl FnOnce() -> Result<(), String> + Send + 'static, cx: &mut Context<Self>) {
        self._task = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { job() }).await;
            let _ = this.update(cx, |this, cx| {
                if let Err(e) = &result {
                    let message = format!("History setting not saved: {e}");
                    this.deck.update(cx, |deck, cx| deck.toast(message, cx));
                }
                cx.emit(HistorySaved(result));
            });
        });
    }

    fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.enabled = Some(enabled);
        cx.notify();
        self.save(move || history_set_enabled(enabled), cx);
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let answer = rfd::AsyncMessageDialog::new()
                .set_title("AgentDeck")
                .set_description("Delete all terminal history?")
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                .await;
            if answer == rfd::MessageDialogResult::Yes {
                let _ = this.update(cx, |this, cx| this.save(history_clear, cx));
            }
        })
        .detach();
    }
}

fn row(label: &'static str, hint: &'static str, control: impl IntoElement, cx: &App) -> Div {
    let p = palette(cx);
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_6()
        .py_3()
        .child(
            div()
                .flex()
                .min_w_0()
                .flex_col()
                .gap(ui(2.0))
                .child(div().text_color(hsla(p.fg)).child(label))
                .child(div().text_xs().text_color(hsla(p.subtle)).child(hint)),
        )
        .child(div().flex().flex_shrink_0().items_center().gap_2().child(control))
}

impl Render for HistorySettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let this = cx.entity().downgrade();
        let toggle = Switch::new("set-history").checked(self.enabled.unwrap_or(true)).on_click({
            let this = this.clone();
            move |checked, _, cx| {
                let _ = this.update(cx, |this, cx| this.set_enabled(*checked, cx));
            }
        });
        let clear = div()
            .id("history-clear")
            .flex()
            .h_8()
            .flex_shrink_0()
            .items_center()
            .gap(ui(6.0))
            .rounded_md()
            .border_1()
            .border_color(hsla(p.line))
            .px_3()
            .text_color(hsla(p.muted))
            .cursor_pointer()
            .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
            .on_click(move |_, _, cx| {
                let _ = this.update(cx, |this, cx| this.clear(cx));
            })
            .child(Icon::from(IconName::Trash))
            .child("Clear");
        div()
            .flex()
            .flex_col()
            .child(row(
                "Save terminal history",
                "Plain text on this PC, kept for 30 days (Ctrl+Shift+F to search)",
                toggle,
                cx,
            ))
            .child(row("Clear history", "Deletes all saved terminal output", clear, cx))
    }
}
