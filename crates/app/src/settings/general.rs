//! Settings > General: terminal font, startup panels, auto grid, launch at startup, terminal
//! history (HistorySettings.tsx) and close to tray.

use agentdeck_core::history;
use agentdeck_core::logic::deck::{FONT_SIZE_MAX, FONT_SIZE_MIN};
use agentdeck_core::logic::settings::{DEFAULT_FONT_FAMILY, Settings};
use agentdeck_core::logic::theme::FONT_MONO;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::*;

use super::SettingsDialog;
use super::controls::{button, confirm, error_text, field, row, section, toggle};
use crate::system::{autostart_enabled, set_autostart};
use crate::theme::{hsla, ui};

pub(super) struct General {
    font_size: Entity<SliderState>,
    font_family: Entity<InputState>,
    /// OS registration (source of truth), read once at open.
    autostart: bool,
    autostart_error: Option<String>,
    /// Stored by the backend (history marker file).
    history: bool,
}

impl General {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsDialog>, s: &Settings, subs: &mut Vec<Subscription>) -> Self {
        let size = s.font_size;
        let font_size = cx.new(|_| {
            SliderState::new()
                .min(FONT_SIZE_MIN)
                .max(FONT_SIZE_MAX)
                .step(1.0)
                .default_value(size)
        });
        subs.push(cx.subscribe(&font_size, |this, _, event: &SliderEvent, cx| {
            let SliderEvent::Change(value) = event else {
                return;
            };
            let size = value.start().round();
            if size != this.settings(cx).font_size {
                this.set(cx, |s| s.font_size = size);
            }
        }));
        let font_family = super::input(window, cx, DEFAULT_FONT_FAMILY, &s.font_family);
        subs.push(cx.subscribe(&font_family, |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let family = state.read(cx).value().to_string();
                this.set(cx, |s| s.font_family = family);
            }
        }));
        let (autostart, autostart_error) = match autostart_enabled() {
            Ok(on) => (on, None),
            Err(e) => (false, Some(e)),
        };
        Self {
            font_size,
            font_family,
            autostart,
            autostart_error,
            history: history::history_get_enabled(),
        }
    }
}

impl SettingsDialog {
    pub(super) fn render_general(&mut self, cx: &mut Context<Self>) -> Div {
        let p = crate::theme::palette(cx);
        let s = self.settings(cx).clone();
        let g = &self.general;
        let flag = |id: &'static str, on: bool, edit: fn(&mut Settings, bool), cx: &mut Context<Self>| {
            toggle(id, on, &p).on_click(cx.listener(move |this, _, _, cx| this.set(cx, |s| edit(s, !on))))
        };
        let mut rows = vec![
            row(
                "Terminal font size",
                Some("For new agents"),
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(ui(144.0)).child(Slider::new(&g.font_size)))
                    .child(
                        div()
                            .w(ui(40.0))
                            .text_right()
                            .font_family(FONT_MONO)
                            .text_xs()
                            .text_color(hsla(p.muted))
                            .child(format!("{} px", s.font_size)),
                    ),
                &p,
            ),
            row("Terminal font", Some("For new agents"), field(&g.font_family, Some(256.0), true), &p),
            row(
                "Confirm \"Stop all\" and \"Restart all\"",
                None,
                flag("set-confirm", s.confirm_bulk, |s, v| s.confirm_bulk = v, cx),
                &p,
            ),
            row(
                "Show workspaces on startup",
                None,
                flag("set-sidebar", s.sidebar_open, |s, v| s.sidebar_open = v, cx),
                &p,
            ),
            row(
                "Show activity panel on startup",
                None,
                flag("set-activity", s.activity_open, |s, v| s.activity_open = v, cx),
                &p,
            ),
            row(
                "Auto-arrange panes in a grid",
                Some("When a pane is added or removed"),
                flag("set-auto-grid", s.auto_grid, |s, v| s.auto_grid = v, cx),
                &p,
            ),
            row(
                "Launch AgentDeck when Windows starts",
                None,
                toggle("set-autostart", g.autostart, &p).on_click(cx.listener(|this, _, window, cx| {
                    let next = !this.general.autostart;
                    this.run(window, cx, move || set_autostart(next), move |this, result, _, cx| {
                        if result.is_ok() {
                            this.general.autostart = next;
                        }
                        this.report(result, cx);
                    });
                })),
                &p,
            ),
        ];
        if let Some(e) = &g.autostart_error {
            rows.push(error_text(&format!("Launch at startup unavailable: {e}"), &p).into_any_element());
        }
        rows.push(row(
            "Save terminal history",
            Some("Plain text on this PC, kept for 30 days (Ctrl+Shift+F to search)"),
            toggle("set-history", g.history, &p).on_click(cx.listener(|this, _, window, cx| {
                let next = !this.general.history;
                this.general.history = next;
                this.run(window, cx, move || history::history_set_enabled(next), |this, result, _, cx| {
                    this.report(result, cx)
                });
            })),
            &p,
        ));
        rows.push(row(
            "Clear history",
            Some("Deletes all saved terminal output"),
            button("set-history-clear", Some(IconName::Trash), "Clear", false, &p).on_click(cx.listener(
                |this, _, window, cx| {
                    if confirm("Delete all terminal history?") {
                        this.run(window, cx, history::history_clear, |this, result, _, cx| this.report(result, cx));
                    }
                },
            )),
            &p,
        ));
        rows.push(row(
            "Close to system tray",
            Some("Closing the window hides it; Quit from the tray icon stops everything"),
            flag("set-close-tray", s.close_to_tray, |s, v| s.close_to_tray = v, cx),
            &p,
        ));
        section("General", rows, &p)
    }
}
