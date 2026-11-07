//! Floating dictation pill: live mic bars, model download progress, transcribing dots, errors and
//! the last transcript. One entity for the whole run, shown in a borderless pop-up window.

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
/// The pill keeps the end of long text (newest words), like the original rtl truncation.
const TAIL_CHARS: usize = 56;

pub struct VoicePill {
    pub deck: Entity<Deck>,
    state: String,
    error: String,
    transcript: String,
    partial: String,
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
            transcript: String::new(),
            partial: String::new(),
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
                    self.transcript.clear();
                    self.partial.clear();
                    self.history = [0.0; BAR_COUNT];
                    self.shown = [0.0; BAR_COUNT];
                }
            }
            "voice-error" => self.error = text(),
            "voice-transcript" => self.transcript = text(),
            "voice-partial" => self.partial = text(),
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
            "listening" if !self.partial.is_empty() => self.partial.clone(),
            "listening" => "Listening".into(),
            "downloading" => match self.percent {
                Some(p) => format!("Downloading model {p}%"),
                None => "Downloading model…".into(),
            },
            "transcribing" => "Transcribing…".into(),
            "error" if !self.error.is_empty() => self.error.clone(),
            "error" => "Voice error".into(),
            _ if !self.transcript.is_empty() => self.transcript.clone(),
            _ => "Ready".into(),
        }
    }

    fn waveform(&mut self) -> Div {
        let mut bars = div().flex().h(px(28.0)).items_center().gap(px(2.0));
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

/// Last `TAIL_CHARS` characters with a leading ellipsis.
fn tail(text: &str) -> String {
    let count = text.chars().count();
    if count <= TAIL_CHARS {
        return text.to_owned();
    }
    let rest: String = text.chars().skip(count - TAIL_CHARS + 1).collect();
    format!("…{rest}")
}

impl Render for VoicePill {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let state = self.state.clone();
        let label = self.label();
        let animating = state == "listening" || state == "transcribing";
        if animating {
            window.request_animation_frame();
        }
        let tall = state == "listening" && !self.partial.is_empty();
        let text = |s: String, alpha: f32| div().truncate().text_sm().text_color(white().opacity(alpha)).child(s);

        let mut pill = div()
            .id("voice-pill")
            .flex()
            .items_center()
            .gap_3()
            .max_w(px(440.0))
            .px_5()
            .bg(black().opacity(0.9))
            .border_1()
            .border_color(white().opacity(0.1))
            .shadow_lg()
            .cursor_default()
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move());
        pill = if tall { pill.min_h(px(48.0)).rounded(px(24.0)).py(px(6.0)) } else { pill.h(px(48.0)).rounded_full() };

        match state.as_str() {
            "listening" => {
                let mut column = div().flex().flex_col().items_center().min_w_0();
                if !self.partial.is_empty() {
                    column = column.child(text(tail(&self.partial), 0.85).max_w(px(400.0)).line_height(px(20.0)));
                }
                pill = pill.child(column.child(self.waveform()));
            }
            "transcribing" => {
                if !self.partial.is_empty() {
                    pill = pill.child(text(tail(&self.partial), 0.6).min_w_0());
                }
                pill = pill.child(self.dots());
            }
            "error" => pill = pill.child(div().truncate().text_sm().text_color(hsla(p.bad)).child(label)),
            "downloading" => pill = pill.child(text(label, 0.7)),
            _ => {
                if !self.transcript.is_empty() {
                    pill = pill.child(
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
                    );
                }
                pill = pill.child(text(tail(&label), 0.7));
            }
        }
        div().size_full().flex().items_center().justify_center().child(pill)
    }
}

#[cfg(test)]
mod tests {
    use super::{TAIL_CHARS, tail};

    #[test]
    fn tail_keeps_the_end() {
        assert_eq!(tail("short"), "short");
        let long = "a".repeat(100) + "END";
        let t = tail(&long);
        assert_eq!(t.chars().count(), TAIL_CHARS);
        assert!(t.starts_with('…') && t.ends_with("END"));
    }
}
