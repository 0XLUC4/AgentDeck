//! AgentDeck theme: the index.css tokens ([`Palette`]) as a GPUI global, also mapped onto
//! gpui-component's theme so its widgets (buttons, inputs, lists, dialogs) match.

use agentdeck_core::logic::settings::Appearance;
use agentdeck_core::logic::theme::Palette;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::*;

/// Current palette, read with [`palette`].
#[derive(Clone, Copy)]
pub struct AppTheme(pub Palette);

impl Global for AppTheme {}

/// Base UI font size at 100% zoom; gpui-component's Root sets the window rem to the theme font size.
pub const BASE_FONT_SIZE: f32 = 13.0;

/// Interface zoom (1.0 = 100%), kept so [`apply`] does not reset it.
struct UiZoom(f32);

impl Global for UiZoom {}

/// A size in px at 100% zoom, as rems so it follows the interface zoom.
pub fn ui(px_at_100: f32) -> Rems {
    rems(px_at_100 / BASE_FONT_SIZE)
}

/// Sets the interface zoom in percent: scales the theme font size, hence the rem of every window.
pub fn set_zoom(percent: u32, cx: &mut App) {
    let zoom = percent as f32 / 100.0;
    cx.set_global(UiZoom(zoom));
    Theme::update(cx, |t| t.font_size = px(BASE_FONT_SIZE * zoom)); // also refreshes every window
}

/// 0xRRGGBB -> Hsla.
pub fn hsla(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// 0xRRGGBB with an alpha 0..1.
pub fn hsla_a(hex: u32, alpha: f32) -> Hsla {
    let mut c = hsla(hex);
    c.a = alpha;
    c
}

pub fn palette(cx: &App) -> Palette {
    cx.try_global::<AppTheme>()
        .map_or_else(Palette::dark, |t| t.0)
}

/// Applies the appearance settings: our palette plus gpui-component's colors.
pub fn apply(appearance: &Appearance, window: Option<&mut Window>, cx: &mut App) {
    let system_light = window
        .as_ref()
        .is_some_and(|w| matches!(w.appearance(), WindowAppearance::Light | WindowAppearance::VibrantLight));
    let p = Palette::for_appearance(appearance, system_light);
    cx.set_global(AppTheme(p));
    let mode = if p.base > 0x808080 { ThemeMode::Light } else { ThemeMode::Dark };
    Theme::change(mode, window, cx);
    let zoom = cx.try_global::<UiZoom>().map_or(1.0, |z| z.0);
    Theme::update(cx, |t| {
        t.font_family = agentdeck_core::logic::theme::FONT_SANS.into();
        t.mono_font_family = agentdeck_core::logic::theme::FONT_MONO.into();
        t.font_size = px(BASE_FONT_SIZE * zoom);
        t.radius = px(6.0); // inputs and buttons: soft corners, not pills
        t.radius_lg = px(8.0);
        let c = &mut t.colors;
        c.background = hsla(p.base);
        c.foreground = hsla(p.fg);
        c.border = hsla(p.line);
        c.input = hsla(p.line_strong);
        c.ring = hsla(p.accent);
        c.caret = hsla(p.fg);
        c.selection = hsla(p.selection);
        c.muted = hsla(p.raised);
        c.muted_foreground = hsla(p.muted);
        c.accent = hsla(p.hover);
        c.accent_foreground = hsla(p.fg);
        c.primary = hsla(p.accent);
        c.primary_hover = hsla(p.accent_hover);
        c.primary_active = hsla(p.accent_hover);
        c.primary_foreground = hsla(p.on_accent);
        c.secondary = hsla(p.raised);
        c.secondary_hover = hsla(p.hover);
        c.secondary_active = hsla(p.hover);
        c.secondary_foreground = hsla(p.fg);
        c.popover = hsla(p.panel);
        c.popover_foreground = hsla(p.fg);
        c.list = hsla(p.panel);
        c.list_hover = hsla(p.hover);
        c.list_active = hsla(p.raised);
        c.list_active_border = hsla(p.line_strong);
        c.title_bar = hsla(p.gutter);
        c.title_bar_border = hsla(p.gutter);
        c.tab_bar = hsla(p.gutter);
        c.tab = hsla(p.gutter);
        c.tab_active = hsla(p.base);
        c.tab_foreground = hsla(p.subtle);
        c.tab_active_foreground = hsla(p.fg);
        c.sidebar = hsla(p.gutter);
        c.sidebar_border = hsla(p.line);
        c.sidebar_foreground = hsla(p.fg);
        c.sidebar_accent = hsla(p.hover);
        c.sidebar_accent_foreground = hsla(p.fg);
        c.danger = hsla(p.bad);
        // Text on a danger background (title bar close button hover): white, not red on red.
        c.danger_foreground = white();
        c.success = hsla(p.ok);
        c.warning = hsla(p.wait);
        c.info = hsla(p.busy);
        c.scrollbar_thumb = hsla(p.line_strong);
        c.window_border = hsla(p.line);
    });
}
