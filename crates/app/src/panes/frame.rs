//! Pane frame effects (PanePanel.tsx, WorkingOutline.tsx, VoiceGlow.tsx, index.css): the rounded pane
//! box with its amber "waiting" border, the working outline with a light running around it, the voice
//! glow on the pane you dictate to, and the "Process exited" bar.

use std::time::Duration;

use agentdeck_core::agents::AgentKind;
use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::Pane;
use agentdeck_core::logic::text::now_ms;
use agentdeck_core::logic::theme::{Palette, RADIUS_XL};
use agentdeck_core::Event;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::runtime::{self, is_busy};
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

/// One loop of the running light (index.css: four segments share one 3.2 s cycle).
const SWEEP_CYCLE: Duration = Duration::from_millis(3200);
/// Inner radius of the outline (index.css: 13px inside the 14px pane).
const OUTLINE_RADIUS: f32 = 13.0;
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

/// Length of a rounded rectangle's outline (radius clamped like `perimeter_point`).
fn perimeter_len(w: f32, h: f32, r: f32) -> f32 {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    2.0 * (w + h) - 8.0 * r + std::f32::consts::TAU * r
}

/// Point at distance `s` clockwise along a rounded rectangle, starting where the top edge leaves
/// the top-left corner. `w`/`h` are the size, `r` the corner radius (clamped to fit).
fn perimeter_point(s: f32, w: f32, h: f32, r: f32) -> (f32, f32) {
    use std::f32::consts::FRAC_PI_2;
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let (sw, sh, arc) = (w - 2.0 * r, h - 2.0 * r, FRAC_PI_2 * r);
    let total = 2.0 * (sw + sh) + 4.0 * arc;
    let mut s = s.rem_euclid(total.max(f32::EPSILON));
    // Corner centers clockwise from top-right, and the angle each arc starts at.
    let corners = [(w - r, r, -FRAC_PI_2), (w - r, h - r, 0.0), (r, h - r, FRAC_PI_2), (r, r, 2.0 * FRAC_PI_2)];
    let edges = [((r, 0.0), (1.0, 0.0), sw), ((w, r), (0.0, 1.0), sh), ((w - r, h), (-1.0, 0.0), sw), ((0.0, h - r), (0.0, -1.0), sh)];
    for k in 0..4 {
        let ((x, y), (dx, dy), len) = edges[k];
        if s <= len {
            return (x + dx * s, y + dy * s);
        }
        s -= len;
        let (cx, cy, start) = corners[k];
        if s <= arc || k == 3 {
            let a = start + (s / r.max(f32::EPSILON)).min(FRAC_PI_2);
            return (cx + r * a.cos(), cy + r * a.sin());
        }
        s -= arc;
    }
    (r, 0.0)
}

/// Soft line around a working pane with one light running all the way around it, corners
/// included: clockwise from the top left, one 3.2 s linear cycle, fading in and out like the
/// original gradient segments.
pub fn working_outline(pane_id: &str, p: &Palette) -> AnyElement {
    let line = hsla_a(p.accent, 0.3);
    let accent = hsla(p.accent);
    div()
        .absolute()
        .inset_0()
        .rounded(ui(RADIUS_XL - 1.0))
        .border_1()
        .border_color(line)
        .with_animation(
            SharedString::from(format!("pane-outline-{pane_id}")),
            Animation::new(SWEEP_CYCLE).repeat(),
            move |el, t| {
                el.child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, (), window, _| {
                            let thick = 1.5_f32;
                            let inset = thick / 2.0;
                            let w = f32::from(bounds.size.width) - thick;
                            let h = f32::from(bounds.size.height) - thick;
                            let r = f32::from(ui(OUTLINE_RADIUS).to_pixels(window.rem_size())) - inset;
                            if w <= 0.0 || h <= 0.0 {
                                return;
                            }
                            let total = perimeter_len(w, h, r);
                            let length = (0.3 * total).min(420.0);
                            let head = t * total;
                            const STEPS: usize = 48;
                            let at = |s: f32| {
                                let (x, y) = perimeter_point(s, w, h, r);
                                point(bounds.origin.x + px(x + inset), bounds.origin.y + px(y + inset))
                            };
                            for i in 0..STEPS {
                                let a = head - length + length * i as f32 / STEPS as f32;
                                let b = head - length + length * (i + 1) as f32 / STEPS as f32;
                                // Bright in the middle, clear at both ends.
                                let mid = (i as f32 + 0.5) / STEPS as f32;
                                let alpha = 0.9 * (1.0 - (2.0 * mid - 1.0).abs());
                                let mut path = PathBuilder::stroke(px(thick));
                                path.move_to(at(a));
                                path.line_to(at(b));
                                if let Ok(path) = path.build() {
                                    window.paint_path(path, hsla_a_of(accent, alpha));
                                }
                            }
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
            },
        )
        .into_any_element()
}

fn hsla_a_of(mut color: Hsla, alpha: f32) -> Hsla {
    color.a = alpha;
    color
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
    let working = pane.agent != AgentKind::Shell && pane.status == PaneStatus::Working && !pane.exited;
    if working && is_busy(&pane.id, now, cx) {
        overlays.push(working_outline(&pane.id, &p));
    }
    let glow = VoiceGlow::current(cx);
    let target = glow.target.clone().or_else(|| deck.state.active_pane_id.clone()).as_deref() == Some(pane.id.as_str());
    overlays.extend(voice_glow(&pane.id, &glow, target, now, &p));
    if target {
        overlays.extend(crate::voice::pill_overlay(cx));
    }
    overlays
}

/// "Process exited" bar under the terminal, with a Restart button.
pub fn exit_bar(pane_id: &str, p: &Palette) -> Div {
    let id = pane_id.to_owned();
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_3()
        .border_t_1()
        .border_color(hsla(p.line))
        .bg(hsla(p.panel))
        .px_3()
        .py(ui(6.0))
        .text_xs()
        .text_color(hsla(p.muted))
        .child(div().size(ui(6.0)).rounded_full().bg(hsla(p.subtle)))
        .child("Process exited. Press R in the terminal to restart.")
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
                .on_click(move |_, _, cx| runtime::restart_pane(&id, cx))
                .child(Icon::new(IconName::RotateCcw).size(ui(13.0)))
                .child("Restart"),
        )
}

/// The whole pane box: rounded 14 px, amber border while waiting, header on top, then the body
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
        .border_color(pane_border(pane, &p))
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
        .child(div().relative().flex_1().min_h_0().child(body))
        .when(pane.exited, |el| el.child(exit_bar(&pane.id, &p)))
        .children(pane_overlays(pane, deck, cx))
}

#[cfg(test)]
mod tests {
    use super::{perimeter_len, perimeter_point};

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn light_goes_all_the_way_around() {
        let (w, h, r) = (100.0, 60.0, 10.0);
        let total = perimeter_len(w, h, r);
        // Starts after the top-left corner, wraps back to the same point.
        assert!(close(perimeter_point(0.0, w, h, r), (10.0, 0.0)));
        assert!(close(perimeter_point(total, w, h, r), (10.0, 0.0)));
        // End of the top edge, then halfway round the top-right corner (on the arc, not the box corner).
        assert!(close(perimeter_point(80.0, w, h, r), (90.0, 0.0)));
        let quarter = std::f32::consts::FRAC_PI_2 * r;
        let (x, y) = perimeter_point(80.0 + quarter / 2.0, w, h, r);
        assert!(((x - 90.0).powi(2) + (y - 10.0).powi(2)).sqrt() - r < 1e-3 && x > 90.0 && y < 10.0);
        // Right edge starts after the corner.
        assert!(close(perimeter_point(80.0 + quarter, w, h, r), (100.0, 10.0)));
        // Just before wrapping: on the top-left arc.
        let (x, y) = perimeter_point(total - 1.0, w, h, r);
        assert!(x < 10.0 && y < 10.0);
    }
}
