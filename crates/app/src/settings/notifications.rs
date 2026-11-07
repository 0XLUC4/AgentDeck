//! Settings > Notifications: system notifications, events, stuck / remind timers, spoken summaries
//! (SpeechRows.tsx) and phone push (ntfy or any webhook).

use agentdeck_core::logic::settings::{NotifySettings, Settings};
use agentdeck_core::logic::theme::FONT_MONO;
use agentdeck_core::notify::push_notify;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::controls::{Opt, button, field, icon, row, section, toggle};
use super::{SettingsDialog, speech};
use crate::theme::{hsla, ui};

type VoiceSelect = SelectState<Vec<Opt>>;

pub(super) struct Notifications {
    stuck: Entity<InputState>,
    remind: Entity<InputState>,
    phone: Entity<InputState>,
    voice: Entity<VoiceSelect>,
    rate: Entity<SliderState>,
}

fn voice_options(voices: &[(String, String)]) -> Vec<Opt> {
    let mut list = vec![Opt::new("", "Automatic")];
    list.extend(voices.iter().map(|(name, lang)| Opt::new(name.clone(), format!("{name} ({lang})"))));
    list
}

impl Notifications {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsDialog>, s: &Settings, subs: &mut Vec<Subscription>) -> Self {
        let stuck = super::number_input(window, cx, subs, s.stuck_minutes, (0, 1440), |s| s.stuck_minutes, |s, v| {
            s.stuck_minutes = v
        });
        let remind = super::number_input(window, cx, subs, s.remind_minutes, (0, 1440), |s| s.remind_minutes, |s, v| {
            s.remind_minutes = v
        });

        let phone = super::input(window, cx, "https://ntfy.sh/\u{2026}", &s.phone_url);
        subs.push(cx.subscribe(&phone, |this, _, event: &InputEvent, cx| match event {
            InputEvent::Change => cx.notify(), // enables "Test"
            InputEvent::Blur => this.commit_phone(cx),
            _ => {}
        }));

        let current = SharedString::from(s.speech.voice.clone());
        let voice = cx.new(|cx| {
            let mut state = SelectState::new(voice_options(&[]), None, window, cx);
            state.set_selected_value(&current, window, cx);
            state
        });
        subs.push(cx.subscribe(&voice, |this, _, event: &SelectEvent<Vec<Opt>>, cx| {
            let SelectEvent::Confirm(value) = event;
            let voice = value.as_ref().map(|v| v.to_string()).unwrap_or_default();
            this.set(cx, |s| s.speech.voice = voice);
        }));
        // The voice list takes about a second (PowerShell): fill it in when ready.
        let select = voice.clone();
        cx.spawn_in(window, async move |_, cx| {
            let voices = cx.background_executor().spawn(async { speech::list_voices() }).await;
            let voices = match voices {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("settings: speech voices unavailable: {e}");
                    return;
                }
            };
            let _ = cx.update(|window, cx| {
                select.update(cx, |state, cx| {
                    state.set_items(voice_options(&voices), window, cx);
                    state.set_selected_value(&current, window, cx);
                });
            });
        })
        .detach();

        let speed = s.speech.rate as f32;
        let rate = cx.new(|_| SliderState::new().min(0.5).max(2.0).step(0.1).default_value(speed));
        subs.push(cx.subscribe(&rate, |this, _, event: &SliderEvent, cx| {
            let SliderEvent::Change(value) = event else {
                return;
            };
            let rate = (f64::from(value.start()) * 10.0).round() / 10.0;
            if rate != this.settings(cx).speech.rate {
                this.set(cx, |s| s.speech.rate = rate);
            }
        }));
        Self {
            stuck,
            remind,
            phone,
            voice,
            rate,
        }
    }
}

impl SettingsDialog {
    fn set_notify(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut NotifySettings)) {
        self.set(cx, |s| edit(&mut s.notify));
    }

    fn commit_phone(&mut self, cx: &mut Context<Self>) {
        let url = self.notify.phone.read(cx).value().trim().to_owned();
        if url != self.settings(cx).phone_url {
            self.set(cx, |s| s.phone_url = url);
        }
    }

    pub(super) fn render_notifications(&mut self, cx: &mut Context<Self>) -> Div {
        let p = crate::theme::palette(cx);
        let s = self.settings(cx).clone();
        let n = &self.notify;
        let flag = |id: &'static str, on: bool, edit: fn(&mut NotifySettings, bool), cx: &mut Context<Self>| {
            toggle(id, on, &p).on_click(cx.listener(move |this, _, _, cx| this.set_notify(cx, |n| edit(n, !on))))
        };
        let events: [(&'static str, bool, fn(&mut NotifySettings, bool)); 3] = [
            ("Done", s.notify.done, |n, v| n.done = v),
            ("Waiting", s.notify.waiting, |n, v| n.waiting = v),
            ("Error", s.notify.error, |n, v| n.error = v),
        ];
        let checkboxes = div().flex().gap_4().children(events.into_iter().map(|(label, on, edit)| {
            div()
                .id(label)
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .text_color(hsla(p.muted))
                .hover(|s| s.text_color(hsla(p.fg)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_notify(cx, |n| edit(n, !on))))
                .child(
                    div()
                        .size_4()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(ui(3.0))
                        .border_1()
                        .border_color(hsla(if on { p.accent } else { p.line_strong }))
                        .when(on, |d| d.bg(hsla(p.accent)).child(icon(IconName::Check, 12.0, p.on_accent))),
                )
                .child(label)
        }));
        let phone_empty = n.phone.read(cx).value().trim().is_empty();
        let mut test = button("set-phone-test", None, "Test", phone_empty, &p);
        if !phone_empty {
            test = test.on_click(cx.listener(|this, _, window, cx| {
                this.commit_phone(cx);
                let url = this.notify.phone.read(cx).value().trim().to_owned();
                this.run(
                    window,
                    cx,
                    move || push_notify(url, "AgentDeck".into(), "Test notification".into()),
                    |this, result, _, cx| this.report(result, cx),
                );
            }));
        }

        let mut rows = vec![
            row("System notifications", None, flag("set-notify", s.notify.enabled, |n, v| n.enabled = v, cx), &p),
            row("Sound", None, flag("set-sound", s.notify.sound, |n, v| n.sound = v, cx), &p),
            row("Events", None, checkboxes, &p),
            row(
                "Only when I'm not looking",
                Some("Window in the background, or the agent is not the active pane"),
                flag("set-unseen", s.notify.only_unseen, |n, v| n.only_unseen = v, cx),
                &p,
            ),
            row(
                "Alert when an agent looks stuck",
                Some("Working with no progress for this many minutes (a hung command, a dev server in the foreground)"),
                field(&n.stuck, Some(96.0), false),
                &p,
            ),
            row(
                "Remind me when an agent keeps waiting",
                Some("Every this many minutes, also on your phone"),
                field(&n.remind, Some(96.0), false),
                &p,
            ),
            row(
                "Read summary aloud",
                Some("When a task finishes while you are away; any key stops it"),
                toggle("set-tts", s.speech.enabled, &p).on_click(cx.listener(move |this, _, _, cx| {
                    this.set(cx, |s| s.speech.enabled = !s.speech.enabled)
                })),
                &p,
            ),
        ];
        if s.speech.enabled {
            let (voice, rate) = (s.speech.voice.clone(), s.speech.rate);
            rows.push(row(
                "Voice",
                Some("Default: the dictation language, otherwise the system language"),
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(ui(256.0)).child(Select::new(&n.voice)))
                    .child(button("set-tts-test", Some(IconName::Volume2), "Test", false, &p).on_click(
                        cx.listener(move |this, _, _, cx| {
                            if let Err(e) = speech::speak("Claude Code finished: voice test.", &voice, rate) {
                                this.report(Err(e), cx);
                            }
                        }),
                    )),
                &p,
            ));
            rows.push(row(
                "Speed",
                None,
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(ui(144.0)).child(Slider::new(&n.rate)))
                    .child(
                        div()
                            .w(ui(40.0))
                            .text_right()
                            .font_family(FONT_MONO)
                            .text_xs()
                            .text_color(hsla(p.muted))
                            .child(format!("{:.1}\u{d7}", s.speech.rate)),
                    ),
                &p,
            ));
        }
        rows.push(row(
            "Phone notifications",
            Some(
                "An ntfy topic URL (e.g. https://ntfy.sh/your-secret-topic) or any webhook. Gets the same events, \
                 even without system notifications. Pick a topic nobody can guess.",
            ),
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(field(&n.phone, Some(224.0), false))
                .child(test),
            &p,
        ));
        section("Notifications", rows, &p)
    }
}
