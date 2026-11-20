//! Settings > Appearance: light / dark / system theme, color themes installed from the Plugins page,
//! interface zoom, accent color and background tint.

use agentdeck_core::logic::settings::{
    Appearance, Settings, ThemeMode, UI_ZOOM_DEFAULT, UI_ZOOM_MAX, UI_ZOOM_MIN, step_ui_zoom,
};
use agentdeck_core::logic::theme::{ACCENT_PRESETS, BACKGROUND_PRESETS, Palette, on_accent, parse_hex};
use agentdeck_core::plugins::CATALOG;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::SettingsDialog;
use super::controls::{hint_text, icon, icon_button, row, section, segmented, to_hex};
use crate::theme::{self, hsla, ui};
use agentdeck_core::logic::theme::{RADIUS_MD, RADIUS_SM};

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

    /// Color theme picker: AgentDeck's own theme, then every installed theme plugin variant.
    fn color_themes(&self, current: &str, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings(cx);
        let installed: Vec<_> = CATALOG
            .iter()
            .filter(|plugin| plugin.is_theme() && settings.plugin_on(plugin))
            .flat_map(|plugin| plugin.themes.iter().map(|v| (v.id, v.title, v.palette())))
            .collect();
        let default = if p.is_light() && current.is_empty() { Palette::light() } else { Palette::dark() };
        let options = std::iter::once(("", "AgentDeck", default)).chain(installed.iter().copied());
        let chips = options.enumerate().map(|(i, (id, title, preview))| {
            let selected = current == id;
            div()
                .id(("color-theme", i))
                .flex()
                .items_center()
                .gap_2()
                .h(ui(30.0))
                .pl(ui(6.0))
                .pr_3()
                .rounded(ui(RADIUS_MD))
                .border_1()
                .border_color(hsla(if selected { p.accent } else { p.line }))
                .when(selected, |d| d.bg(hsla(p.raised)))
                .cursor_pointer()
                .hover(|s| s.bg(hsla(p.hover)))
                .text_xs()
                .text_color(hsla(if selected { p.fg } else { p.muted }))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_appearance(window, cx, |a| a.color_theme = id.to_owned())
                }))
                .child(theme_swatch(&preview))
                .child(title)
        });
        let hint = if installed.is_empty() {
            "Install color themes from the Plugins page (Themes)."
        } else {
            "Installed from the Plugins page. A color theme replaces the light/dark mode and the background."
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .py_3()
            .child(div().text_color(hsla(p.fg)).child("Color theme"))
            .child(hint_text(hint, p))
            .child(div().flex().flex_wrap().gap_1p5().children(chips))
            .into_any_element()
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
                // Picking a mode goes back to AgentDeck's own theme.
                let _ = this.update(cx, |this, cx| {
                    this.set_appearance(window, cx, |a| {
                        a.theme = mode;
                        a.color_theme.clear();
                    })
                });
            }
        });
        let rows = vec![
            row("Theme", Some("System follows Windows' light/dark setting. Terminals follow the theme."), modes, &p),
            self.color_themes(&appearance.color_theme, &p, cx),
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

/// A theme in miniature: its background with the accent and two code colors.
fn theme_swatch(preview: &Palette) -> Div {
    let dot = |color: u32| div().size(ui(6.0)).rounded_full().bg(hsla(color));
    div()
        .flex()
        .items_center()
        .gap(ui(2.0))
        .px(ui(4.0))
        .h(ui(18.0))
        .rounded(ui(RADIUS_SM))
        .border_1()
        .border_color(hsla(preview.line_strong))
        .bg(hsla(preview.base))
        .child(dot(preview.accent))
        .child(dot(preview.syntax.keyword))
        .child(dot(preview.syntax.string))
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
