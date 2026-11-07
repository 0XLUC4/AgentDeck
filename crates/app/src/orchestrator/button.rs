//! Pane header badge for an orchestrator pane: crown + journal of its MCP tool calls + global pause.
//! Original: src/components/Orchestrator/OrchestratorButton.tsx.

use agentdeck_core::logic::deck::Pane;
use agentdeck_core::logic::orchestrator::ActivityEntry;
use chrono::{Local, TimeZone};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable, WindowExt};
use gpui_kit::*;

use super::tools::set_paused;
use crate::state::Deck;
use crate::theme::{hsla, palette, ui};

/// `new Date(at).toLocaleTimeString("en-US")`: "3:04:05 PM".
fn time(at: i64) -> String {
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|t| t.format("%-I:%M:%S %p").to_string())
        .unwrap_or_default()
}

/// Amber crown marking an orchestrator pane.
pub fn orchestrator_crown(size: f32, cx: &App) -> Icon {
    Icon::new(IconName::Crown)
        .size(ui(size))
        .text_color(hsla(palette(cx).wait))
}

/// "Pause orchestrator" / "Resume orchestrator" button (applies to every orchestrator).
pub fn pause_toggle(deck: &Entity<Deck>, cx: &App) -> Stateful<Div> {
    let p = palette(cx);
    let paused = deck.read(cx).orchestrator.paused;
    let deck = deck.clone();
    div()
        .id("orchestrator-pause")
        .flex()
        .items_center()
        .gap(ui(6.0))
        .h_8()
        .px_3()
        .rounded(ui(6.0))
        .border_1()
        .border_color(hsla(if paused { p.line_strong } else { p.line }))
        .text_color(hsla(if paused { p.fg } else { p.muted }))
        .cursor_pointer()
        .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
        .child(Icon::new(if paused { IconName::Play } else { IconName::Pause }).size(ui(14.0)))
        .child(if paused { "Resume orchestrator" } else { "Pause orchestrator" })
        .on_click(move |_, _, cx| set_paused(&deck, !paused, cx))
}

/// Crown + activity log button; `None` for ordinary panes.
pub fn orchestrator_badge(deck: &Entity<Deck>, pane: &Pane, cx: &App) -> Option<AnyElement> {
    if !pane.orchestrator {
        return None;
    }
    let d = deck.read(cx);
    let count = d.orchestrator.log.get(&pane.id).map_or(0, Vec::len);
    let paused = if d.orchestrator.paused { " \u{b7} paused" } else { "" };
    let (deck, pane_id) = (deck.clone(), pane.id.clone());
    Some(
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .child(orchestrator_crown(13.0, cx))
            .child(
                Button::new(SharedString::from(format!("orchestrator-log-{}", pane.id)))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::ScrollText))
                    .tooltip(format!("Orchestrator actions ({count}){paused}"))
                    .on_click(move |_, window, cx| open_activity_log(deck.clone(), pane_id.clone(), window, cx)),
            )
            .into_any_element(),
    )
}

/// Opens the "Orchestrator actions" dialog for one orchestrator pane.
pub fn open_activity_log(deck: Entity<Deck>, pane_id: String, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| OrchestratorLog::new(deck, pane_id, cx));
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .w(ui(640.0).to_pixels(window.rem_size()))
            .p_0()
            .close_button(false)
            .child(view.clone())
    });
}

/// Content of the activity dialog: newest call first, then the pause bar.
pub struct OrchestratorLog {
    deck: Entity<Deck>,
    pane_id: String,
    _observe: Subscription,
}

impl OrchestratorLog {
    pub fn new(deck: Entity<Deck>, pane_id: String, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&deck, |_, _, cx| cx.notify());
        Self {
            deck,
            pane_id,
            _observe: observe,
        }
    }

    fn entry(e: &ActivityEntry, cx: &App) -> Div {
        let p = palette(cx);
        let mono = agentdeck_core::logic::theme::FONT_MONO;
        div()
            .flex()
            .items_start()
            .gap_3()
            .px_3()
            .py(ui(6.0))
            .border_b_1()
            .border_color(hsla(p.line))
            .child(
                div()
                    .flex_shrink_0()
                    .font_family(mono)
                    .text_xs()
                    .text_color(hsla(p.subtle))
                    .child(time(e.at)),
            )
            .child(
                div()
                    .w_24()
                    .flex_shrink_0()
                    .font_family(mono)
                    .text_xs()
                    .text_color(hsla(p.fg))
                    .child(e.tool.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_color(hsla(p.muted)).child(e.summary.clone()))
                    .children(
                        e.error
                            .clone()
                            .map(|err| div().text_xs().text_color(hsla(p.bad)).child(err)),
                    ),
            )
    }
}

impl Render for OrchestratorLog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let d = self.deck.read(cx);
        let paused = d.orchestrator.paused;
        let entries: Vec<Div> = d
            .orchestrator
            .log
            .get(&self.pane_id)
            .map(|list| list.iter().map(|e| Self::entry(e, cx)).collect())
            .unwrap_or_default();
        let body = if entries.is_empty() {
            div()
                .py_3()
                .flex()
                .justify_center()
                .text_color(hsla(p.muted))
                .child("No actions yet.")
        } else {
            div().child(
                div()
                    .id("orchestrator-log")
                    .flex()
                    .flex_col()
                    .max_h(ui(384.0))
                    .overflow_y_scroll()
                    .rounded(ui(6.0))
                    .border_1()
                    .border_color(hsla(p.line))
                    .children(entries),
            )
        };
        div()
            .flex()
            .flex_col()
            .text_size(ui(13.0))
            .text_color(hsla(p.fg))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt_4()
                    .pr_3()
                    .pl_5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(orchestrator_crown(16.0, cx))
                            .child(div().text_size(ui(15.0)).font_weight(FontWeight::MEDIUM).child("Orchestrator actions")),
                    )
                    .child(
                        Button::new("orchestrator-log-close")
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::X))
                            .tooltip("Close (Esc)")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
            .child(div().p_5().child(body))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(hsla(p.line))
                    .px_5()
                    .py_3()
                    .child(div().flex_1().text_xs().text_color(hsla(p.subtle)).child(if paused {
                        "Paused: launch, send and interrupt are blocked"
                    } else {
                        "Cannot answer permissions or close agents"
                    }))
                    .child(pause_toggle(&self.deck, cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn time_is_us_style() {
        let t = super::time(0);
        assert!(t.ends_with(" AM") || t.ends_with(" PM"), "{t}");
    }
}
