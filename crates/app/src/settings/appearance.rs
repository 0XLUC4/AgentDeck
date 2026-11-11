//! Settings > Appearance: light / dark / system theme, interface zoom, accent color and background tint.

use agentdeck_core::logic::settings::{
    Appearance, Settings, ThemeMode, UI_ZOOM_DEFAULT, UI_ZOOM_MAX, UI_ZOOM_MIN, step_ui_zoom,
};
use agentdeck_core::logic::theme::{ACCENT_PRESETS, BACKGROUND_PRESETS, Palette, on_accent, parse_hex};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::SettingsDialog;
use super::controls::{icon, icon_button, row, section, segmented, to_hex};
use crate::theme::{self, hsla, ui};

const MODES: [(ThemeMode, &str); 3] = [
    (ThemeMode::Dark, "Dark"),
    (ThemeMode::Light, "Light"),
    (ThemeMode::System, "System"),
];

#[derive(Clone, Copy)]
enum Target {
    Accent,
    Background,
}

pub(super) struct AppearanceUi {
    accent: Entity<ColorPickerState>,
    background: Entity<ColorPickerState>,
}

fn picker(
    window: &mut Window,
    cx: &mut Context<SettingsDialog>,
    subs: &mut Vec<Subscription>,
    value: &str,
    target: Target,
) -> Entity<ColorPickerState> {
    let start = parse_hex(value).unwrap_or(0x4f8ff7);
    let state = cx.new(|cx| ColorPickerState::new(window, cx).default_value(theme::hsla(start)));
    subs.push(cx.subscribe_in(&state, window, move |this, _, event: &ColorPickerEvent, window, cx| {
        let ColorPickerEvent::Change(Some(color)) = event else {
            return;
        };
        let hex = to_hex(*color);
        this.set_appearance(window, cx, |a| match target {
            Target::Accent => a.accent = hex,
            Target::Background => a.background = hex,
        });
    }));
    state
}

impl AppearanceUi {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsDialog>, s: &Settings, subs: &mut Vec<Subscription>) -> Self {
        Self {
            accent: picker(window, cx, subs, &s.appearance.accent, Target::Accent),
            background: picker(window, cx, subs, &s.appearance.background, Target::Background),
        }
    }
}

impl SettingsDialog {
    /// Saves the appearance and restyles the window at once.
    fn set_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>, edit: impl FnOnce(&mut Appearance)) {
        self.set(cx, |s| edit(&mut s.appearance));
        let appearance = self.settings(cx).appearance.clone();
        theme::apply(&appearance, Some(window), cx);
        window.refresh();
    }

    fn swatches(
        &self,
        target: Target,
        presets: &[(&'static str, &'static str)],
        value: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        // Both rows start with "Neutral": the row name keeps their element ids apart.
        let (id_prefix, neutral, picker) = match target {
            Target::Accent => ("accent-swatch", p.fg, &self.appearance.accent),
            Target::Background => ("background-swatch", p.raised, &self.appearance.background),
        };
        let custom = !value.is_empty() && !presets.iter().any(|(_, v)| *v == value);
        let custom_fill = if custom { parse_hex(value).unwrap_or(p.raised) } else { p.raised };
        let pipette = if custom { parse_hex(on_accent(value)).unwrap_or(p.fg) } else { p.muted };
        div()
            .flex()
            .items_center()
            .gap(ui(4.0))
            .children(presets.iter().enumerate().map(|(i, &(_, preset))| {
                let fill = parse_hex(preset).unwrap_or(neutral);
                ring(value == preset, p)
                    .id((id_prefix, i))
                    .cursor_pointer()
                    .child(dot(fill))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_appearance(window, cx, |a| match target {
                            Target::Accent => a.accent = preset.to_owned(),
                            Target::Background => a.background = preset.to_owned(),
                        })
                    }))
            }))
            .child(
                ring(custom, p).child(
                    dot(custom_fill)
                        .relative()
                        .child(icon(IconName::Pipette, 12.0, pipette))
                        // The picker's own trigger is a rounded square: an invisible icon trigger covers the dot instead.
                        .child(
                            div().absolute().inset_0().child(
                                ColorPicker::new(picker)
                                    .icon(Icon::new(IconName::Circle).size(ui(DOT)).text_color(transparent_black())),
                            ),
                        ),
                ),
            )
    }

    /// Interface zoom: saves it and rescales every window.
    fn set_ui_zoom(&mut self, zoom: u32, cx: &mut Context<Self>) {
        self.set(cx, |s| s.ui_zoom = zoom);
        theme::set_zoom(zoom, cx);
    }

    fn zoom_control(&self, zoom: u32, p: &Palette, cx: &mut Context<Self>) -> Div {
        let step = |id: &'static str, name: IconName, delta: i32, enabled: bool| {
            icon_button(id, name, false, p)
                .when(!enabled, |d| d.opacity(0.4))
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| this.set_ui_zoom(step_ui_zoom(zoom, delta), cx)))
                })
        };
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(step("ui-zoom-out", IconName::Minus, -1, zoom > UI_ZOOM_MIN))
            .child(div().w(ui(44.0)).text_center().text_color(hsla(p.fg)).child(format!("{zoom}%")))
            .child(step("ui-zoom-in", IconName::Plus, 1, zoom < UI_ZOOM_MAX))
            .child(step("ui-zoom-reset", IconName::RotateCcw, 0, zoom != UI_ZOOM_DEFAULT))
    }

    pub(super) fn render_appearance(&mut self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let appearance = self.settings(cx).appearance.clone();
        let modes = segmented("set-theme", &MODES, appearance.theme, &p, {
            let this = cx.entity().downgrade();
            move |mode, window, cx| {
                let _ = this.update(cx, |this, cx| this.set_appearance(window, cx, |a| a.theme = mode));
            }
        });
        let rows = vec![
            row("Theme", Some("System follows Windows' light/dark setting. Terminals follow the theme."), modes, &p),
            row(
                "Interface zoom",
                Some("Scales the whole interface. Ctrl+Shift+= / Ctrl+Shift+- / Ctrl+Shift+0"),
                self.zoom_control(self.settings(cx).ui_zoom, &p, cx),
                &p,
            ),
            row(
                "Accent color",
                Some("Buttons, focus rings, toggles and highlights"),
                self.swatches(Target::Accent, &ACCENT_PRESETS, &appearance.accent, &p, cx),
                &p,
            ),
            row(
                "Background",
                Some("Tints windows and panels in the theme's own light or dark shade"),
                self.swatches(Target::Background, &BACKGROUND_PRESETS, &appearance.background, &p, cx),
                &p,
            ),
        ];
        section("Appearance", rows, &p)
    }
}

/// Size of a color dot; its selection ring adds a 2px gap and a 2px solid border around it.
const DOT: f32 = 20.0;

/// Round frame drawing the selection ring as a solid border (transparent when not selected), no shadow.
fn ring(selected: bool, p: &Palette) -> Div {
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(ui(DOT + 8.0))
        .rounded_full()
        .border_2()
        .border_color(if selected { hsla(p.fg) } else { transparent_black() })
}

/// Plain filled circle, no border.
fn dot(fill: u32) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(ui(DOT))
        .rounded_full()
        .bg(hsla(fill))
}
