//! Pane frame effects (PanePanel.tsx, WorkingOutline.tsx, VoiceGlow.tsx, index.css): the rounded pane
//! box with its amber "waiting" border, the working outline with a light running around it, the voice
//! glow on the pane you dictate to, and the "Process exited" bar.

use std::time::Duration;

use agentdeck_core::hooks_server::{PaneStatus, StepState};
use agentdeck_core::logic::deck::Pane;
use agentdeck_core::logic::text::now_ms;
use agentdeck_core::logic::theme::{FONT_MONO, Palette, RADIUS_XL};
use agentdeck_core::Event;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::runtime;
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

/// How long the "delivered" flash stays on the pane that received a dictation.
const SENT_FLASH_MS: i64 = 900;

/// Voice activity shown as the glow. `voice-state` / `voice-level` events fill `state` and `level`
/// (see runtime); the voice module sets `target` (pane the next dictation goes to, None = active
/// pane) and `sent_to` / `sent_at` when a transcript lands.
#[derive(Default, Clone)]
pub struct VoiceGlow {
    pub target: Option<String>,
    /// "idle" | "listening" | "downloading" | "transcribing" | "error".
    pub state: String,
    /// Smoothed mic level 0..1.
    pub level: f32,
    pub sent_to: Option<String>,
    pub sent_at: i64,
}

impl Global for VoiceGlow {}

impl VoiceGlow {
    /// Mic state and level from the bus events; target and "sent" flash from the voice module.
    pub fn current(cx: &App) -> VoiceGlow {
        let mut glow = cx.try_global::<VoiceGlow>().cloned().unwrap_or_default();
        let activity = crate::voice::voice_activity(cx);
        glow.target = crate::voice::voice_target(cx);
        glow.sent_to = activity.sent_to;
        glow.sent_at = activity.sent_at;
        glow
    }

    /// Pane the next dictation goes to.
    pub fn target_pane(cx: &App, deck: &Deck) -> Option<String> {
        Self::current(cx).target.or_else(|| deck.state.active_pane_id.clone())
    }
}

pub(super) fn on_voice_event(event: &Event, cx: &mut App) {
    let glow = cx.default_global::<VoiceGlow>();
    if event.name == "voice-state" {
        glow.state = event.payload.as_str().unwrap_or("idle").to_owned();
        if glow.state != "listening" {
            glow.level = 0.0;
        }
    } else if let Some(level) = event.payload.as_f64() {
        glow.level += (level as f32 - glow.level) * 0.4;
    }
}

/// A "sent" flash is still showing somewhere: the clock keeps repainting until it ends.
pub(super) fn voice_flash_pending(now: i64, cx: &App) -> bool {
    let v = crate::voice::voice_activity(cx);
    v.sent_to.is_some() && now - v.sent_at < SENT_FLASH_MS + 1000
}

/// Border of the pane box: amber while the agent waits for the user, otherwise none.
pub fn pane_border(pane: &Pane, p: &Palette) -> Hsla {
    if pane.status == PaneStatus::Waiting {
        hsla(p.wait)
    } else {
        hsla_a(p.base, 0.0)
    }
}

/// Border of the pane box with the active pane marked: amber while waiting (it keeps priority),
/// a thin accent line on the active pane when its session shows several panes, otherwise none.
pub fn pane_frame_border(pane: &Pane, deck: &Deck, p: &Palette) -> Hsla {
    let state = &deck.state;
    let active = state.active_pane_id.as_deref() == Some(pane.id.as_str());
    // With a single pane there is nothing to tell apart: the border would be noise.
    let several = state.tabs.iter().any(|tab| tab.pane_ids.len() > 1 && tab.pane_ids.contains(&pane.id));
    if active && several && pane.status != PaneStatus::Waiting {
        hsla_a(p.accent, 0.45)
    } else {
        pane_border(pane, p)
    }
}

/// Outline on the pane you are talking to: its inner glow follows your voice, it pulses while
/// transcribing and flashes once the words land.
fn voice_glow(pane_id: &str, glow: &VoiceGlow, target: bool, now: i64, p: &Palette) -> Option<AnyElement> {
    let live = target && matches!(glow.state.as_str(), "listening" | "transcribing");
    let flash = glow.sent_to.as_deref() == Some(pane_id) && now - glow.sent_at < SENT_FLASH_MS;
    if !live && !flash {
        return None;
    }
    let level = if glow.state == "listening" { glow.level } else { 0.0 };
    let base = div()
        .absolute()
        .inset_0()
        .rounded(ui(RADIUS_XL - 1.0))
        .border(px(1.5))
        .border_color(hsla_a(p.accent, 0.7))
        .shadow(vec![BoxShadow {
            color: hsla_a(p.accent, 0.38 * (0.2 + 0.8 * level)),
            offset: point(px(0.0), px(0.0)),
            blur_radius: px(36.0),
            spread_radius: px(0.0),
            inset: true,
        }]);
    if flash && !live {
        let accent = hsla(p.accent);
        return Some(
            base.border_color(accent)
                .with_animation(
                    SharedString::from(format!("voice-sent-{pane_id}-{}", glow.sent_at)),
                    Animation::new(Duration::from_millis(700)).with_easing(ease_out_quint()),
                    |el, t| el.opacity(1.0 - t),
                )
                .into_any_element(),
        );
    }
    if glow.state == "transcribing" {
        return Some(
            base.with_animation(
                SharedString::from(format!("voice-pulse-{pane_id}")),
                Animation::new(Duration::from_millis(900)).repeat(),
                |el, t| {
                    let half = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
                    el.opacity(1.0 - 0.55 * ease_in_out(half))
                },
            )
            .into_any_element(),
        );
    }
    Some(base.into_any_element())
}

/// Overlays drawn above the pane content (absolute, inset 0): working outline and voice glow.
/// A plain shell has no task, so it never gets the outline.
pub fn pane_overlays(pane: &Pane, deck: &Deck, cx: &App) -> Vec<AnyElement> {
    let p = palette(cx);
    let now = now_ms();
    let mut overlays = Vec::new();
    let glow = VoiceGlow::current(cx);
    let target = glow.target.clone().or_else(|| deck.state.active_pane_id.clone()).as_deref() == Some(pane.id.as_str());
    overlays.extend(voice_glow(&pane.id, &glow, target, now, &p));
    if target {
        overlays.extend(crate::voice::pill_overlay(cx));
    }
    overlays
}

/// "Process exited" bar under the terminal, with a Restart button.
pub fn exit_bar(pane_id: &str, asleep: bool, p: &Palette) -> Div {
    let id = pane_id.to_owned();
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_3()
        .border_t_1()
        .border_color(hsla(p.line))
        .rounded_b(ui(RADIUS_XL - 1.0))
        .bg(hsla(p.panel))
        .px_3()
        .py(ui(6.0))
        .text_xs()
        .text_color(hsla(p.muted))
        .child(div().size(ui(6.0)).rounded_full().bg(hsla(p.subtle)))
        .child(if asleep {
            "Asleep to free memory. Press a key in the terminal to wake it on the same conversation."
        } else {
            "Process exited. Press R in the terminal to restart."
        })
        .child(
            div()
                .id(SharedString::from(format!("restart-exited-{pane_id}")))
                .ml_auto()
                .flex()
                .items_center()
                .gap(ui(6.0))
                .rounded(ui(RADIUS_XL - 8.0))
                .border_1()
                .border_color(hsla(p.line_strong))
                .px_2()
                .py(ui(2.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(hsla(p.fg))
                .cursor_pointer()
                .hover(|s| s.bg(hsla(p.hover)))
                .on_click(move |_, _, cx| {
                    if asleep {
                        runtime::wake_pane(&id, cx)
                    } else {
                        runtime::restart_pane(&id, cx)
                    }
                })
                .child(Icon::new(IconName::RotateCcw).size(ui(13.0)))
                .child(if asleep { "Wake" } else { "Restart" }),
        )
}

/// Plan of the pane, under its header: "2/5", a progress bar and the running step. A click unfolds
/// the whole plan: a green check for a finished step, an accent ring for the running one (red
/// when the agent stopped on it), a gray ring for what is left. Subagents show the same way
/// when the agent keeps no task list.
pub fn steps_bar(pane: &Pane, cx: &App) -> Option<Div> {
    let steps = runtime::pane_steps(&pane.id, cx);
    if steps.is_empty() {
        return None;
    }
    let p = palette(cx);
    let open = runtime::steps_open(&pane.id, cx);
    let stopped = pane.exited || pane.status == PaneStatus::Error;
    let done = steps.iter().filter(|s| s.state == StepState::Done).count();
    let total = steps.len();
    let current = steps.iter().find(|s| s.state == StepState::Active).map(|s| s.label.clone());
    let id = pane.id.clone();
    let bar = div()
        .id(SharedString::from(format!("pane-steps-{}", pane.id)))
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .h(ui(24.0))
        .px(ui(10.0))
        .border_b_1()
        .border_color(hsla(p.line))
        .text_xs()
        .text_color(hsla(p.muted))
        .cursor_pointer()
        .hover(|s| s.bg(hsla(p.hover)))
        .tooltip(|window, cx| Tooltip::new("Show or hide the plan").build(window, cx))
        .on_click(move |_, _, cx| runtime::toggle_steps(&id, cx))
        .child(div().flex_shrink_0().font_family(FONT_MONO).text_color(hsla(p.fg)).child(format!("{done}/{total}")))
        .child(
            div().w(ui(96.0)).h(ui(4.0)).flex_shrink_0().overflow_hidden().rounded_full().bg(hsla(p.raised)).child(
                div()
                    .h_full()
                    .rounded_full()
                    .bg(hsla(if stopped { p.bad } else { p.ok }))
                    .w(relative(done as f32 / total as f32)),
            ),
        )
        .children(current.map(|label| div().min_w_0().flex_1().truncate().child(label)))
        .child(
            div().ml_auto().flex_shrink_0().text_color(hsla(p.subtle)).child(
                Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(ui(12.0)),
            ),
        );
    let ring = |color: u32| div().size(ui(12.0)).flex_shrink_0().rounded_full().border(px(1.5)).border_color(hsla(color));
    let list = open.then(|| {
        div()
            .id(SharedString::from(format!("pane-steps-list-{}", pane.id)))
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(ui(6.0))
            .max_h(ui(180.0))
            .overflow_y_scroll()
            .px(ui(10.0))
            .py_2()
            .border_b_1()
            .border_color(hsla(p.line))
            .bg(hsla(p.base))
            .text_xs()
            .children(steps.iter().map(|step| {
                let (mark, note, color) = match step.state {
                    StepState::Done => {
                        let check = div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(ui(12.0))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(hsla(p.ok))
                            .text_color(hsla(p.base))
                            .child(Icon::new(IconName::Check).size(ui(9.0)));
                        (check, "Done", p.ok)
                    }
                    StepState::Active if stopped => (ring(p.bad), "Stopped", p.bad),
                    StepState::Active => (ring(p.busy), "Running", p.busy),
                    StepState::Pending => (ring(p.subtle), "To do", p.subtle),
                };
                let label_color = if step.state == StepState::Pending { p.muted } else { p.fg };
                let note = if step.agent { format!("{note} \u{b7} agent") } else { note.to_owned() };
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(mark)
                    .child(div().min_w_0().flex_1().truncate().text_color(hsla(label_color)).child(step.label.clone()))
                    .child(div().flex_shrink_0().text_color(hsla(color)).child(note))
            }))
    });
    Some(div().flex().flex_col().flex_shrink_0().child(bar).children(list))
}

/// The whole pane box: rounded 14 px, amber border while waiting (accent when active), header on top, then the body
/// (terminal), the exit bar when the process is gone, and the overlays. Mouse down focuses the pane.
pub fn pane_frame(pane: &Pane, deck: &Deck, header: impl IntoElement, body: impl IntoElement, cx: &App) -> Stateful<Div> {
    let p = palette(cx);
    let waiting = pane.status == PaneStatus::Waiting;
    let id = pane.id.clone();
    div()
        .id(SharedString::from(format!("pane-frame-{}", pane.id)))
        .group(crate::palette::QUEUE_HOVER_GROUP)
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .min_w_0()
        .min_h_0()
        .rounded(ui(RADIUS_XL))
        .overflow_hidden()
        .bg(hsla(p.base))
        .border_1()
        .border_color(pane_frame_border(pane, deck, &p))
        .when(waiting, |el| {
            el.shadow(vec![BoxShadow {
                color: hsla(p.wait),
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(0.0),
                spread_radius: px(1.0),
                inset: false,
            }])
        })
        .on_mouse_down(MouseButton::Left, move |_, _, cx| runtime::focus_pane(&id, cx))
        .child(div().flex_shrink_0().h(ui(30.0)).border_b_1().border_color(hsla(p.line)).child(header))
        .children(steps_bar(pane, cx))
        .child(div().relative().flex_1().min_h_0().child(body))
        .when(pane.exited, |el| el.child(exit_bar(&pane.id, runtime::is_asleep(&pane.id, cx), &p)))
        .children(pane_overlays(pane, deck, cx))
}
