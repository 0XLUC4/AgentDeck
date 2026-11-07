//! Settings dialog (Ctrl+,): General, Appearance, Notifications, Voice, Agents, MCP hub,
//! Templates, About. Every change applies and saves immediately; the header shows "Saved" or the error.
//! Original: src/components/Settings/*, src/components/Templates/TemplatesSection.tsx,
//! src/components/History/HistorySettings.tsx. Model: agentdeck_core::logic::settings.

mod about;
mod agents;
mod appearance;
mod controls;
mod general;
mod mcp;
mod notifications;
mod speech;
mod templates;
mod voice;

use std::time::Duration;

use agentdeck_core::logic::settings::{Settings, step_ui_zoom};
use agentdeck_core::persist;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{Root, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::state::{Deck, DeckEvent};
use crate::theme::{hsla, palette, ui};
use controls::{clamp_number, error_text, icon, icon_button};

actions!(agentdeck, [OpenSettings, UiZoomIn, UiZoomOut, UiZoomReset]);

/// Registers this module's key bindings and the global `OpenSettings` / UI zoom handlers. Called once from `main`.
pub fn init(cx: &mut App) {
    // Windows reports Ctrl+Shift+= as "ctrl-+" (shift folded into the key), other platforms as "ctrl-shift-=".
    cx.bind_keys([
        KeyBinding::new("ctrl-,", OpenSettings, None),
        KeyBinding::new("ctrl-shift-=", UiZoomIn, None),
        KeyBinding::new("ctrl-+", UiZoomIn, None),
        KeyBinding::new("ctrl-shift--", UiZoomOut, None),
        KeyBinding::new("ctrl-_", UiZoomOut, None),
        KeyBinding::new("ctrl-shift-0", UiZoomReset, None),
        KeyBinding::new("ctrl-)", UiZoomReset, None),
    ]);
    cx.on_action(|_: &UiZoomIn, cx: &mut App| zoom_ui(1, cx));
    cx.on_action(|_: &UiZoomOut, cx: &mut App| zoom_ui(-1, cx));
    cx.on_action(|_: &UiZoomReset, cx: &mut App| zoom_ui(0, cx));
    cx.on_action(|_: &OpenSettings, cx: &mut App| {
        let Some(handle) = cx.active_window() else {
            return;
        };
        // The window is borrowed while it dispatches the action: open on the next tick.
        cx.defer(move |cx| {
            if let Err(e) = handle.update(cx, |_, window, cx| open(window, cx)) {
                eprintln!("settings: cannot open: {e}");
            }
        });
    });
}

/// Steps the interface zoom (0 resets to 100%), saves it and rescales every window.
fn zoom_ui(delta: i32, cx: &mut App) {
    let Some(handle) = crate::system::main_window(cx) else {
        return;
    };
    // The window may be dispatching this action: update it on the next tick.
    cx.defer(move |cx| {
        let deck = handle.update(cx, |_, window, cx| main_deck(window, cx)).ok().flatten();
        let Some(deck) = deck else {
            return;
        };
        let zoom = deck.update(cx, |deck, cx| {
            deck.update_settings(|s| s.ui_zoom = step_ui_zoom(s.ui_zoom, delta), cx);
            deck.settings.ui_zoom
        });
        crate::theme::set_zoom(zoom, cx);
    });
}

/// Opens the dialog over the main window (no-op while another dialog is open).
pub fn open(window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let Some(deck) = main_deck(window, cx) else {
        return;
    };
    let view = cx.new(|cx| SettingsDialog::new(deck, window, cx));
    window.open_dialog(cx, move |dialog, window, _| {
        let viewport = window.viewport_size();
        let width = (f32::from(viewport.width) * 0.94).min(896.0);
        let height = (f32::from(viewport.height) - 48.0).min(768.0);
        dialog
            .w(px(width))
            .h(px(height))
            .p_0()
            .overflow_hidden()
            .centered()
            .close_button(false)
            .child(view.clone())
    });
}

/// The deck of the main window (its root content is `layout::DeckView`).
fn main_deck(window: &Window, cx: &App) -> Option<Entity<Deck>> {
    let root = window.root::<Root>().flatten()?;
    let deck_view = root.read(cx).view().clone().downcast::<crate::layout::DeckView>().ok()?;
    Some(deck_view.read(cx).deck().clone())
}

#[derive(Clone, Copy, PartialEq)]
enum SectionId {
    General,
    Appearance,
    Notifications,
    Voice,
    Agents,
    Mcp,
    Templates,
    About,
}

const SECTIONS: [(SectionId, &str, IconName); 8] = [
    (SectionId::General, "General", IconName::SlidersHorizontal),
    (SectionId::Appearance, "Appearance", IconName::Palette),
    (SectionId::Notifications, "Notifications", IconName::Bell),
    (SectionId::Voice, "Voice", IconName::Mic),
    (SectionId::Agents, "Agents", IconName::Bot),
    (SectionId::Mcp, "MCP hub", IconName::Network),
    (SectionId::Templates, "Templates", IconName::LayoutTemplate),
    (SectionId::About, "About", IconName::Info),
];

pub struct SettingsDialog {
    pub deck: Entity<Deck>,
    section: SectionId,
    saved: bool,
    error: Option<String>,
    saved_timer: Option<Task<()>>,
    general: general::General,
    notify: notifications::Notifications,
    agents: agents::Agents,
    appearance: appearance::AppearanceUi,
    voice: voice::VoiceUi,
    hub: mcp::Hub,
    templates: templates::Templates,
    about: about::About,
    _subs: Vec<Subscription>,
}

impl SettingsDialog {
    pub fn new(deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let s = deck.read(cx).settings.clone();
        let backend = deck.read(cx).backend.clone();
        let data_dir = deck.read(cx).data_dir.clone();
        let mut subs = vec![
            cx.observe(&deck, |_, _, cx| cx.notify()),
            cx.subscribe(&deck, |this, _, DeckEvent::Backend(event), cx| {
                this.on_backend(event, cx)
            }),
        ];
        let dialog = cx.entity().downgrade();
        subs.push(cx.intercept_keystrokes(move |event, window, cx| {
            let _ = dialog.update(cx, |this, cx| this.intercept_key(&event.keystroke, window, cx));
        }));
        let general = general::General::new(window, cx, &s, &mut subs);
        let notify = notifications::Notifications::new(window, cx, &s, &mut subs);
        let agents = agents::Agents::new(window, cx, &s, &mut subs);
        let appearance = appearance::AppearanceUi::new(window, cx, &s, &mut subs);
        let voice = voice::VoiceUi::new(backend, window, cx, &mut subs);
        let hub = mcp::Hub::new(window, cx, &mut subs);
        let templates = templates::Templates::new(window, cx, &mut subs);
        let about = about::About::new(data_dir, cx, &mut subs);
        Self {
            deck,
            section: SectionId::General,
            saved: false,
            error: None,
            saved_timer: None,
            general,
            notify,
            agents,
            appearance,
            voice,
            hub,
            templates,
            about,
            _subs: subs,
        }
    }

    fn settings<'a>(&self, cx: &'a App) -> &'a Settings {
        &self.deck.read(cx).settings
    }

    /// Applies a settings change, writes settings.json and reports it in the header.
    fn set(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut Settings)) {
        let result = self.deck.update(cx, |deck, cx| {
            edit(&mut deck.settings);
            cx.notify();
            persist::save_settings(&deck.data_dir, &deck.settings)
        });
        self.report(result, cx);
    }

    /// "Saved" for 1.5 s, or "Could not save: ..." (the `Save` contract of the original).
    fn report(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        match result {
            Ok(()) => {
                self.error = None;
                self.saved = true;
                self.saved_timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(1500)).await;
                    let _ = this.update(cx, |this, cx| {
                        this.saved = false;
                        cx.notify();
                    });
                }));
            }
            Err(e) => {
                self.saved = false;
                self.error = Some(format!("Could not save: {e}"));
            }
        }
        cx.notify();
    }

    /// Runs blocking backend work off the UI thread, then `done` on the dialog.
    fn run<T: Send + 'static>(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
        done: impl FnOnce(&mut Self, Result<T, String>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move { work() }).await;
            let _ = this.update_in(cx, |this, window, cx| done(this, result, window, cx));
        })
        .detach();
    }

    /// Runs before key bindings, so Esc in these modes does not close the dialog.
    fn intercept_key(&mut self, keystroke: &Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        if self.voice.recording {
            self.on_shortcut_key(keystroke, window, cx);
        } else if keystroke.key == "escape" && self.templates.renaming.take().is_some() {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_backend(&mut self, event: &agentdeck_core::Event, cx: &mut Context<Self>) {
        if event.name == "voice-download" {
            self.voice.on_download_progress(&event.payload);
            cx.notify();
        }
    }

    fn nav(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        div()
            .flex()
            .flex_col()
            .w(ui(192.0))
            .flex_shrink_0()
            .gap(ui(2.0))
            .p_3()
            .border_r_1()
            .border_color(hsla(p.line))
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .pb_3()
                    .text_size(ui(15.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(hsla(p.fg))
                    .child("Settings"),
            )
            .children(SECTIONS.iter().map(|&(id, label, name)| {
                let current = self.section == id;
                let color = if current { p.fg } else { p.muted };
                div()
                    .id(label)
                    .flex()
                    .items_center()
                    .gap(ui(10.0))
                    .h_8()
                    .px_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(hsla(color))
                    .when(current, |d| d.bg(hsla(p.raised)))
                    .when(!current, |d| d.hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.section = id;
                        cx.notify();
                    }))
                    .child(icon(name, 15.0, color))
                    .child(label)
            }))
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let body = match self.section {
            SectionId::General => self.render_general(cx),
            SectionId::Appearance => self.render_appearance(cx),
            SectionId::Notifications => self.render_notifications(cx),
            SectionId::Voice => self.render_voice(cx),
            SectionId::Agents => self.render_agents(cx),
            SectionId::Mcp => self.render_mcp(cx),
            SectionId::Templates => self.render_templates(cx),
            SectionId::About => self.render_about(cx),
        };
        let header = div()
            .flex()
            .flex_shrink_0()
            .h_12()
            .items_center()
            .justify_end()
            .gap_3()
            .pr_3()
            .pl_6()
            .when(self.saved, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .text_color(hsla(p.subtle))
                        .child(icon(IconName::Check, 12.0, p.subtle))
                        .child("Saved"),
                )
            })
            .child(
                icon_button("settings-close", IconName::X, false, &p)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            );
        div()
            .flex()
            .size_full()
            .bg(hsla(p.panel))
            .text_color(hsla(p.fg))
            .text_size(ui(13.0))
            .child(self.nav(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(header)
                    .child(
                        div()
                            .id("settings-body")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .gap_4()
                            .px_6()
                            .pb_6()
                            .overflow_y_scroll()
                            .children(self.error.as_deref().map(|e| error_text(e, &p)))
                            .child(body),
                    ),
            )
    }
}

/// Single-line text field.
fn input(window: &mut Window, cx: &mut Context<SettingsDialog>, placeholder: &str, value: &str) -> Entity<InputState> {
    let (placeholder, value) = (placeholder.to_owned(), value.to_owned());
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value))
}

/// Number field saved on blur; empty or 0 means off, other values are clamped to `min..=max`.
fn number_input(
    window: &mut Window,
    cx: &mut Context<SettingsDialog>,
    subs: &mut Vec<Subscription>,
    value: u32,
    (min, max): (u32, u32),
    get: fn(&Settings) -> u32,
    set: fn(&mut Settings, u32),
) -> Entity<InputState> {
    let shown = if value > 0 { value.to_string() } else { String::new() };
    let state = input(window, cx, "Off", &shown);
    subs.push(cx.subscribe_in(&state, window, move |this, state, event: &InputEvent, window, cx| {
        if !matches!(event, InputEvent::Blur) {
            return;
        }
        let next = clamp_number(&state.read(cx).value(), min, max);
        let text = if next > 0 { next.to_string() } else { String::new() };
        state.update(cx, |s, cx| s.set_value(text, window, cx));
        if next != get(this.settings(cx)) {
            this.set(cx, |s| set(s, next));
        }
    }));
    state
}
