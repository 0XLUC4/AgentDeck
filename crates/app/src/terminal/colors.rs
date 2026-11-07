//! ANSI colors: 16 named, the 256-color cube and truecolor. The 16 named colors, background,
//! foreground and cursor come from the terminal theme, light or dark like the app palette.

use agentdeck_core::logic::theme::{TerminalTheme, mix};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use gpui_kit::App;

/// Terminal theme matching the current app palette.
pub fn theme(cx: &App) -> &'static TerminalTheme {
    TerminalTheme::for_palette(&crate::theme::palette(cx))
}

/// Color of a 0..=255 palette index (xterm 256-color table).
pub fn indexed(index: u8, theme: &TerminalTheme) -> u32 {
    let i = index as u32;
    match i {
        0..=15 => theme.ansi[i as usize],
        16..=231 => {
            let level = |v: u32| if v == 0 { 0 } else { 55 + v * 40 };
            let n = i - 16;
            (level(n / 36) << 16) | (level(n / 6 % 6) << 8) | level(n % 6)
        }
        _ => {
            let gray = 8 + (i - 232) * 10;
            (gray << 16) | (gray << 8) | gray
        }
    }
}

fn rgb(c: Rgb) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
}

/// Two thirds of the way from the background, like xterm's dim (SGR 2) on either theme.
pub fn dim(color: u32, theme: &TerminalTheme) -> u32 {
    mix(color, theme.background, 2.0 / 3.0)
}

/// Answer to an OSC color query: 0..=255 palette, 256 foreground, 257 background, 258 cursor.
pub fn query(index: usize, theme: &TerminalTheme) -> Rgb {
    let hex = match index {
        0..=255 => indexed(index as u8, theme),
        257 => theme.background,
        258 => theme.cursor,
        _ => theme.foreground,
    };
    Rgb { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8 }
}

/// Resolves a cell color. `bold` brightens the 8 base colors (xterm.js drawBoldTextInBrightColors).
/// Colors set by the program (OSC 4/10/11) override the defaults.
pub fn resolve(color: Color, overrides: &Colors, bold: bool, theme: &TerminalTheme) -> u32 {
    match color {
        Color::Spec(c) => rgb(c),
        Color::Indexed(i) => {
            let i = if bold && i < 8 { i + 8 } else { i };
            overrides[i as usize].map_or_else(|| indexed(i, theme), rgb)
        }
        Color::Named(named) => {
            if let Some(c) = overrides[named] {
                return rgb(c);
            }
            let index = named as usize;
            match named {
                NamedColor::Foreground | NamedColor::BrightForeground => theme.foreground,
                NamedColor::Background => theme.background,
                NamedColor::Cursor => theme.cursor,
                NamedColor::DimForeground => dim(theme.foreground, theme),
                _ if index < 8 && bold => theme.ansi[index + 8],
                _ if index < 16 => theme.ansi[index],
                // DimBlack..DimWhite follow DimForeground (index 259..266).
                _ => dim(theme.ansi[(index - NamedColor::DimBlack as usize) % 8], theme),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentdeck_core::logic::theme::{TERMINAL_DARK, TERMINAL_LIGHT};
    use core::prelude::v1::test;

    #[test]
    fn palette_256() {
        let dark = &TERMINAL_DARK;
        assert_eq!(indexed(1, dark), 0xcc0000);
        assert_eq!(indexed(16, dark), 0x000000);
        assert_eq!(indexed(21, dark), 0x0000ff);
        assert_eq!(indexed(196, dark), 0xff0000);
        assert_eq!(indexed(231, dark), 0xffffff);
        assert_eq!(indexed(232, dark), 0x080808);
        assert_eq!(indexed(255, dark), 0xeeeeee);
        assert_eq!(indexed(1, &TERMINAL_LIGHT), 0xcd3131);
    }

    #[test]
    fn answers_color_queries() {
        assert_eq!(query(1, &TERMINAL_DARK), Rgb { r: 0xcc, g: 0, b: 0 });
        for theme in [&TERMINAL_DARK, &TERMINAL_LIGHT] {
            assert_eq!(rgb(query(257, theme)), theme.background);
            assert_eq!(rgb(query(256, theme)), theme.foreground);
        }
    }

    #[test]
    fn bold_brightens_base_colors() {
        let none = Colors::default();
        let dark = &TERMINAL_DARK;
        assert_eq!(resolve(Color::Named(NamedColor::Red), &none, true, dark), 0xef2929);
        assert_eq!(resolve(Color::Indexed(2), &none, true, dark), 0x8ae234);
        assert_eq!(resolve(Color::Named(NamedColor::Foreground), &none, true, dark), dark.foreground);
        assert_eq!(resolve(Color::Spec(Rgb { r: 1, g: 2, b: 3 }), &none, false, dark), 0x010203);
        assert_eq!(resolve(Color::Named(NamedColor::Foreground), &none, false, &TERMINAL_LIGHT), 0x1f1d1b);
    }

    #[test]
    fn dim_fades_toward_background() {
        assert_eq!(dim(0x000000, &TERMINAL_LIGHT), mix(0x000000, 0xfbfaf9, 2.0 / 3.0));
        assert!(dim(0x000000, &TERMINAL_LIGHT) > 0x404040);
        assert!(dim(0xffffff, &TERMINAL_DARK) < 0xc0c0c0);
    }
}
