//! Settings > General: terminal font, startup panels, auto grid, launch at startup, terminal
//! history (HistorySettings.tsx) and close to tray.

use agentdeck_core::history;
use agentdeck_core::logic::deck::{FONT_SIZE_MAX, FONT_SIZE_MIN};
use agentdeck_core::logic::settings::{DEFAULT_FONT_FAMILY, RemoteSettings, Settings};
use agentdeck_core::logic::theme::{FONT_MONO, Palette};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::*;

use super::SettingsDialog;
use super::controls::{button, confirm, error_text, field, hint_text, row, section, toggle};
use crate::system::{autostart_enabled, set_autostart};
use crate::theme::{hsla, ui};

pub(super) struct General {
    font_size: Entity<SliderState>,
    font_family: Entity<InputState>,
    sleep: Entity<InputState>,
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
        let sleep = super::number_input(window, cx, subs, s.sleep_minutes, (0, 1440), |s| s.sleep_minutes, |s, v| {
            s.sleep_minutes = v
        });
        let (autostart, autostart_error) = match autostart_enabled() {
            Ok(on) => (on, None),
            Err(e) => (false, Some(e)),
        };
        Self {
            font_size,
            font_family,
            sleep,
            autostart,
            autostart_error,
            history: history::history_get_enabled(),
        }
    }
}

/// Port of the phone page unless the settings file names another.
const PHONE_PORT: u16 = 47810;

/// The link as a QR code: dark modules on white, with the quiet zone scanners need.
fn qr(text: &str) -> AnyElement {
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else {
        return div().into_any_element();
    };
    let width = code.width();
    let dark: Vec<bool> = code.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect();
    div()
        .flex_shrink_0()
        .size(ui(176.0))
        .p(ui(10.0))
        .rounded(ui(8.0))
        .bg(white())
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| {
                    let cell = bounds.size.width / width as f32;
                    for (i, _) in dark.iter().enumerate().filter(|(_, dark)| **dark) {
                        let (x, y) = ((i % width) as f32, (i / width) as f32);
                        // Half a pixel wider: neighbours overlap instead of leaving hairlines.
                        let module = Bounds::new(bounds.origin + point(cell * x, cell * y), size(cell + px(0.5), cell + px(0.5)));
                        window.paint_quad(fill(module, black()));
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

impl SettingsDialog {
    /// Turns the phone page on or off; the first time, makes its secret link.
    fn toggle_phone(&mut self, cx: &mut Context<Self>) {
        self.set(cx, |s| {
            s.remote.enabled = !s.remote.enabled;
            if s.remote.token.is_empty() {
                s.remote.token = agentdeck_core::logic::deck::new_id();
            }
            if s.remote.port == 0 {
                s.remote.port = PHONE_PORT;
            }
        });
        let applied = self.deck.update(cx, |deck, _| crate::system::apply_phone(deck));
        if applied.is_err() {
            // The port is taken: off again, with the reason shown.
            self.set(cx, |s| s.remote.enabled = false);
            self.report(applied, cx);
        }
    }

    /// The link to open on the phone: QR code, copy button and what it gives access to.
    fn phone_link(&self, remote: &RemoteSettings, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let host = agentdeck_core::remote::lan_ip().map_or_else(|| "this-computer".to_owned(), |ip| ip.to_string());
        let url = format!("http://{host}:{}/#{}", remote.port, remote.token);
        let copy = url.clone();
        div()
            .flex()
            .gap_4()
            .py_2()
            .child(qr(&url))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_2()
                    .min_w_0()
                    .child(div().font_family(FONT_MONO).text_xs().text_color(hsla(p.muted)).child(url))
                    .child(button("set-phone-copy", Some(IconName::Copy), "Copy link", false, p).on_click(cx.listener(
                        move |_, _, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone())),
                    )))
                    .child(hint_text(
                        "Scan it with a phone on the same Wi-Fi. The page is plain HTTP: anyone on that network who gets \
                         this link can prompt your agents and answer their permissions, so keep it for a network you \
                         trust. Windows may ask to let AgentDeck through the firewall.",
                        p,
                    )),
            )
            .into_any_element()
    }

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
                "Show projects on startup",
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
        rows.push(row(
            "Put idle Claude agents to sleep",
            Some("After this many minutes without activity (0 = never). Frees their memory; a key wakes them on the same conversation"),
            field(&g.sleep, Some(96.0), false),
            &p,
        ));
        rows.push(row(
            "Phone access",
            Some("A page on your local network: agent status, prompts, allow or deny permissions"),
            toggle("set-phone", s.remote.enabled, &p).on_click(cx.listener(|this, _, _, cx| this.toggle_phone(cx))),
            &p,
        ));
        if s.remote.enabled {
            rows.push(self.phone_link(&s.remote, &p, cx));
        }
        section("General", rows, &p)
    }
}
