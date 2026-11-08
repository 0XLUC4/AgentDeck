//! Dictation pill: live mic bars, model download progress, transcribing dots, errors and a check
//! once the words land. One entity for the whole run, shown at the bottom of the target pane.

use std::time::Instant;

use gpui_kit::assets::IconName;
use gpui_kit::*;

use crate::state::{Deck, DeckEvent};
use crate::theme::{hsla, palette};

// ~30 levels/s from the backend: 28 bars show the last ~0.9 s of real mic signal, newest on the right.
const BAR_COUNT: usize = 28;
const MIN_HEIGHT: f32 = 0.1;
/// Below this the mic is just room noise: draw flat so silence reads as silence.
const NOISE_GATE: f32 = 0.2;
/// Per-frame easing toward the target height, so bars glide instead of jumping.
const EASE: f32 = 0.35;

pub struct VoicePill {
    pub deck: Entity<Deck>,
    state: String,
    error: String,
    /// A transcript landed since the last recording started: show the check.
    delivered: bool,
    percent: Option<u64>,
    history: [f32; BAR_COUNT],
    shown: [f32; BAR_COUNT],
    started: Instant,
}

impl VoicePill {
    pub fn new(deck: Entity<Deck>, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&deck, Self::on_event).detach();
        Self {
            deck,
            state: "idle".into(),
            error: String::new(),
            delivered: false,
            percent: None,
            history: [0.0; BAR_COUNT],
            shown: [0.0; BAR_COUNT],
            started: Instant::now(),
        }
    }

    fn on_event(&mut self, _: Entity<Deck>, event: &DeckEvent, cx: &mut Context<Self>) {
        let DeckEvent::Backend(event) = event;
        let text = || event.payload.as_str().unwrap_or("").to_owned();
        match event.name.as_str() {
            "voice-state" => {
                self.state = text();
                if self.state == "listening" {
                    self.delivered = false;
                    self.history = [0.0; BAR_COUNT];
                    self.shown = [0.0; BAR_COUNT];
                }
            }
            "voice-error" => self.error = text(),
            "voice-transcript" => self.delivered = !text().is_empty(),
            "voice-download" => {
                let p = &event.payload;
                self.percent = match (p["received"].as_u64(), p["total"].as_u64()) {
                    (Some(received), Some(total)) if total > 0 => Some(received * 100 / total),
                    _ => None,
                };
            }
            "voice-level" => {
                // Scroll left: values and their eased heights move together, so motion stays smooth.
                let level = event.payload.as_f64().unwrap_or(0.0) as f32;
                self.history.rotate_left(1);
                self.shown.rotate_left(1);
                self.history[BAR_COUNT - 1] =
                    if level < NOISE_GATE { 0.0 } else { (level - NOISE_GATE) / (1.0 - NOISE_GATE) };
                return; // the animation frame loop redraws
            }
            _ => return,
        }
        cx.notify();
    }

    fn label(&self) -> String {
        match self.state.as_str() {
            "downloading" => match self.percent {
                Some(p) => format!("Downloading model {p}%"),
                None => "Downloading model…".into(),
            },
            "error" if !self.error.is_empty() => self.error.clone(),
            "error" => "Voice error".into(),
            _ => "Ready".into(),
        }
    }

    fn waveform(&mut self) -> Div {
        let mut bars = div().flex().h(px(24.0)).items_center().gap(px(2.0));
        for i in 0..BAR_COUNT {
            self.shown[i] += (self.history[i] - self.shown[i]) * EASE;
            let height = MIN_HEIGHT + self.shown[i] * (1.0 - MIN_HEIGHT);
            // Fade in from the left over the first 30%, like the original mask gradient.
            let fade = (i as f32 / (BAR_COUNT as f32 * 0.3)).min(1.0);
            bars = bars.child(div().w(px(3.0)).h(relative(height)).rounded_full().bg(white().opacity(fade)));
        }
        bars
    }

    fn dots(&self) -> Div {
        let t = self.started.elapsed().as_secs_f32();
        div().flex().h(px(24.0)).items_center().gap(px(6.0)).children([0.0f32, 0.15, 0.3].map(|delay| {
            // Tailwind animate-pulse: opacity 1 -> 0.5 -> 1 over 2 s.
            let phase = ((t - delay) / 2.0).rem_euclid(1.0);
            let opacity = 0.75 + 0.25 * (phase * std::f32::consts::TAU).cos();
            div().size(px(6.0)).rounded_full().bg(white().opacity(0.8 * opacity))
        }))
    }
}

impl Render for VoicePill {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let state = self.state.clone();
        if state == "listening" || state == "transcribing" {
            window.request_animation_frame();
        }
        let text = |s: String, alpha: f32| div().truncate().text_sm().text_color(white().opacity(alpha)).child(s);

        let pill = div()
            .id("voice-pill")
            .flex()
            .items_center()
            .gap_3()
            .h(px(40.0))
            .max_w(px(440.0))
            .px_5()
            .rounded_full()
            .bg(black().opacity(0.9))
            .border_1()
            .border_color(white().opacity(0.1))
            .shadow_lg();

        match state.as_str() {
            "listening" => pill.child(self.waveform()),
            "transcribing" => pill.child(self.dots()),
            "error" => pill.child(div().truncate().text_sm().text_color(hsla(p.bad)).child(self.label())),
            "downloading" => pill.child(text(self.label(), 0.7)),
            _ if self.delivered => pill.child(
                div()
                    .size(px(24.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(hsla(p.ok))
                    .text_color(white())
                    .child(IconName::Check),
            ),
            _ => pill.child(text(self.label(), 0.7)),
        }
    }
}
