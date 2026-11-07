//! Theme tokens from `src/index.css` and appearance logic from `src/lib/theme.ts`.
//! Colors are computed from the same OKLCH values as the CSS and returned as `0xRRGGBB`.

use super::settings::{Appearance, ThemeMode};

/// Accent swatches offered in Settings; "" keeps the theme's neutral grey.
pub const ACCENT_PRESETS: [(&str, &str); 7] = [
    ("Neutral", ""),
    ("Orange", "#d97757"),
    ("Blue", "#4f8ff7"),
    ("Violet", "#9b7fd4"),
    ("Green", "#3fb67a"),
    ("Pink", "#e0609a"),
    ("Yellow", "#e2b33c"),
];

/// Background swatches: the hue tints every surface, lightness stays the theme's. "" = neutral grey.
pub const BACKGROUND_PRESETS: [(&str, &str); 7] = [
    ("Neutral", ""),
    ("Navy", "#2b4c8c"),
    ("Teal", "#1f7a7a"),
    ("Forest", "#2f7a45"),
    ("Plum", "#6b3f8c"),
    ("Wine", "#8c2f4a"),
    ("Brown", "#7a5230"),
];

/// xterm theme of the terminal panes (TerminalPane.tsx).
pub const TERMINAL_BACKGROUND: u32 = 0x100e0c;
pub const TERMINAL_FOREGROUND: u32 = 0xebe7e2;
pub const TERMINAL_CURSOR: u32 = 0xf1e7d2;
pub const TERMINAL_SELECTION: u32 = 0x3b3732;

/// Colors of a terminal pane: the xterm theme plus its 16 ANSI colors (black..white, then bright).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalTheme {
    pub background: u32,
    pub foreground: u32,
    pub cursor: u32,
    pub selection: u32,
    pub ansi: [u32; 16],
}

/// Dark terminal: TerminalPane.tsx THEME with the xterm.js default ANSI palette.
pub const TERMINAL_DARK: TerminalTheme = TerminalTheme {
    background: TERMINAL_BACKGROUND,
    foreground: TERMINAL_FOREGROUND,
    cursor: TERMINAL_CURSOR,
    selection: TERMINAL_SELECTION,
    ansi: [
        0x2e3436, 0xcc0000, 0x4e9a06, 0xc4a000, 0x3465a4, 0x75507b, 0x06989a, 0xd3d7cf, //
        0x555753, 0xef2929, 0x8ae234, 0xfce94f, 0x729fcf, 0xad7fa8, 0x34e2e2, 0xeeeeec,
    ],
};

/// Light terminal (the original kept xterm dark): VS Code Light+ ANSI colors, greens darkened
/// and white/bright white greyed so every color stays readable on the light background.
pub const TERMINAL_LIGHT: TerminalTheme = TerminalTheme {
    background: 0xfbfaf9,
    foreground: 0x1f1d1b,
    cursor: 0x3b3732,
    selection: 0xd6d2cc,
    ansi: [
        0x000000, 0xcd3131, 0x107c10, 0x949800, 0x0451a5, 0xbc05bc, 0x0598bc, 0x555555, //
        0x666666, 0xcd3131, 0x14a014, 0xb5ba00, 0x0451a5, 0xbc05bc, 0x0598bc, 0xa5a5a5,
    ],
};

impl TerminalTheme {
    /// Light terminal under a light palette (same test as the app's light/dark mode).
    pub fn for_palette(palette: &Palette) -> &'static TerminalTheme {
        if palette.base > 0x808080 { &TERMINAL_LIGHT } else { &TERMINAL_DARK }
    }
}

pub const FONT_SANS: &str = "Segoe UI Variable Text";
pub const FONT_MONO: &str = "Cascadia Code";
pub const RADIUS_MD: f32 = 10.0;
pub const RADIUS_LG: f32 = 12.0;
pub const RADIUS_XL: f32 = 14.0;

/// Every color token of index.css, as 0xRRGGBB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub gutter: u32,
    pub base: u32,
    pub panel: u32,
    pub raised: u32,
    pub hover: u32,
    pub line: u32,
    pub line_strong: u32,
    pub fg: u32,
    pub muted: u32,
    pub subtle: u32,
    pub accent: u32,
    pub accent_hover: u32,
    pub on_accent: u32,
    pub ok: u32,
    pub busy: u32,
    pub wait: u32,
    pub bad: u32,
    pub close: u32,
    pub agent_claude: u32,
    pub agent_codex: u32,
    pub agent_gemini: u32,
    pub agent_grok: u32,
    pub agent_opencode: u32,
    pub agent_shell: u32,
    pub selection: u32,
}

/// OKLCH (L 0..1, C, H degrees) to 0xRRGGBB, gamut clipped. Ref: https://bottosson.github.io/posts/oklab/
pub fn oklch(l: f64, c: f64, h: f64) -> u32 {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548 * b;
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    let r = 4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3;
    let g = -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3;
    let bl = -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701 * s3;
    let encode = |x: f64| {
        let x = x.clamp(0.0, 1.0);
        let v = if x <= 0.003_130_8 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        };
        (v * 255.0).round() as u32
    };
    (encode(r) << 16) | (encode(g) << 8) | encode(bl)
}

/// "#rrggbb" -> (L, C, H) in OKLCH.
pub fn hex_to_oklch(hex: &str) -> Option<(f64, f64, f64)> {
    let rgb = parse_hex(hex)?;
    let lin = |shift: u32| {
        let c = ((rgb >> shift) & 0xff) as f64 / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(16), lin(8), lin(0));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    let ll = 0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s;
    let a = 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s;
    let bb = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s;
    Some((ll, a.hypot(bb), bb.atan2(a).to_degrees().rem_euclid(360.0)))
}

/// "#rrggbb" (case-insensitive) -> 0xRRGGBB.
pub fn parse_hex(hex: &str) -> Option<u32> {
    let digits = hex.strip_prefix('#')?;
    (digits.len() == 6)
        .then(|| u32::from_str_radix(digits, 16).ok())
        .flatten()
}

/// sRGB relative luminance (WCAG) of "#rrggbb".
pub fn luminance(hex: &str) -> f64 {
    let Some(rgb) = parse_hex(hex) else {
        return 0.0;
    };
    let ch = |shift: u32| {
        let c = ((rgb >> shift) & 0xff) as f64 / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
}

/// Text color readable on the accent: whichever of near-black / white contrasts more.
pub fn on_accent(hex: &str) -> &'static str {
    if luminance(hex) > 0.18 { "#141414" } else { "#ffffff" }
}

/// Linear mix in sRGB (approximates CSS `color-mix(in oklch, ...)` closely enough for hover tints).
pub fn mix(a: u32, b: u32, a_weight: f64) -> u32 {
    let ch = |shift: u32| {
        let x = ((a >> shift) & 0xff) as f64;
        let y = ((b >> shift) & 0xff) as f64;
        ((x * a_weight + y * (1.0 - a_weight)).round() as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}

impl Palette {
    pub fn dark() -> Self {
        Self {
            gutter: oklch(0.13, 0.0, 0.0),
            base: oklch(0.165, 0.0, 0.0),
            panel: oklch(0.195, 0.0, 0.0),
            raised: oklch(0.235, 0.0, 0.0),
            hover: oklch(0.27, 0.0, 0.0),
            line: oklch(0.28, 0.0, 0.0),
            line_strong: oklch(0.36, 0.0, 0.0),
            fg: oklch(0.93, 0.0, 0.0),
            muted: oklch(0.76, 0.0, 0.0),
            subtle: oklch(0.6, 0.0, 0.0),
            accent: oklch(0.93, 0.0, 0.0),
            accent_hover: oklch(0.98, 0.0, 0.0),
            on_accent: oklch(0.2, 0.0, 0.0),
            ok: oklch(0.76, 0.15, 150.0),
            busy: oklch(0.74, 0.13, 245.0),
            wait: oklch(0.8, 0.16, 65.0),
            bad: oklch(0.68, 0.19, 25.0),
            close: oklch(0.58, 0.21, 27.0),
            agent_claude: 0xd97757,
            agent_codex: oklch(0.86, 0.0, 0.0),
            agent_gemini: 0x8e75b2,
            agent_grok: oklch(0.9, 0.0, 0.0),
            agent_opencode: oklch(0.84, 0.0, 0.0),
            agent_shell: oklch(0.76, 0.0, 0.0),
            selection: oklch(0.4, 0.0, 0.0),
        }
    }

    pub fn light() -> Self {
        Self {
            gutter: oklch(0.925, 0.0, 0.0),
            base: oklch(0.985, 0.0, 0.0),
            panel: oklch(0.965, 0.0, 0.0),
            raised: oklch(0.93, 0.0, 0.0),
            hover: oklch(0.9, 0.0, 0.0),
            line: oklch(0.87, 0.0, 0.0),
            line_strong: oklch(0.78, 0.0, 0.0),
            fg: oklch(0.21, 0.0, 0.0),
            muted: oklch(0.4, 0.0, 0.0),
            subtle: oklch(0.53, 0.0, 0.0),
            accent: oklch(0.25, 0.0, 0.0),
            accent_hover: oklch(0.15, 0.0, 0.0),
            on_accent: oklch(0.98, 0.0, 0.0),
            ok: oklch(0.56, 0.15, 150.0),
            busy: oklch(0.55, 0.16, 245.0),
            wait: oklch(0.62, 0.16, 65.0),
            bad: oklch(0.55, 0.2, 25.0),
            close: oklch(0.52, 0.21, 27.0),
            agent_claude: 0xd97757,
            agent_codex: oklch(0.3, 0.0, 0.0),
            agent_gemini: 0x8e75b2,
            agent_grok: oklch(0.22, 0.0, 0.0),
            agent_opencode: oklch(0.3, 0.0, 0.0),
            agent_shell: oklch(0.4, 0.0, 0.0),
            selection: oklch(0.85, 0.0, 0.0),
        }
    }

    /// Palette for the appearance settings. `system_light`: the OS uses a light theme.
    pub fn for_appearance(appearance: &Appearance, system_light: bool) -> Self {
        let light = match appearance.theme {
            ThemeMode::Light => true,
            ThemeMode::Dark => false,
            ThemeMode::System => system_light,
        };
        let mut p = if light { Self::light() } else { Self::dark() };
        if let Some(accent) = parse_hex(&appearance.accent) {
            p.accent = accent;
            p.accent_hover = mix(accent, if light { 0x000000 } else { 0xffffff }, 0.82);
            p.on_accent = parse_hex(on_accent(&appearance.accent)).unwrap_or(p.on_accent);
        }
        if let Some((_, c, h)) = hex_to_oklch(&appearance.background) {
            // Same as `oklch(from <bg> <lightness> min(c, max) h)` in theme.ts.
            let max = if light { 0.03 } else { 0.045 };
            let tint = |l: f64| oklch(l, c.min(max), h);
            let l = if light {
                [0.925, 0.985, 0.965, 0.93, 0.9, 0.87, 0.78]
            } else {
                [0.13, 0.165, 0.195, 0.235, 0.27, 0.28, 0.36]
            };
            [p.gutter, p.base, p.panel, p.raised, p.hover, p.line, p.line_strong] =
                l.map(tint);
        }
        p
    }

    pub fn agent(&self, kind: crate::agents::AgentKind) -> u32 {
        use crate::agents::AgentKind::*;
        match kind {
            Claude => self.agent_claude,
            Codex => self.agent_codex,
            Gemini => self.agent_gemini,
            Grok => self.agent_grok,
            OpenCode => self.agent_opencode,
            Shell => self.agent_shell,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklch_greys_match_css() {
        assert_eq!(oklch(0.0, 0.0, 0.0), 0x000000);
        assert_eq!(oklch(1.0, 0.0, 0.0), 0xffffff);
        // oklch(0.165 0 0) is #0e0e0e, oklch(0.93 0 0) is #e8e8e8.
        assert_eq!(Palette::dark().base, 0x0e0e0e);
        assert_eq!(Palette::dark().fg, 0xe8e8e8);
    }

    #[test]
    fn hex_roundtrip() {
        let (l, c, h) = hex_to_oklch("#d97757").unwrap();
        assert_eq!(oklch(l, c, h), 0xd97757);
    }

    #[test]
    fn accent_contrast() {
        assert_eq!(luminance("#000000"), 0.0);
        assert!((luminance("#ffffff") - 1.0).abs() < 1e-9);
        assert_eq!(on_accent("#e2b33c"), "#141414");
        assert_eq!(on_accent("#4f8ff7"), "#141414");
        assert_eq!(on_accent("#7c3aed"), "#ffffff");
    }

    #[test]
    fn background_tint_keeps_lightness() {
        let tinted = Palette::for_appearance(
            &Appearance {
                theme: ThemeMode::Dark,
                accent: String::new(),
                background: "#2b4c8c".into(),
            },
            false,
        );
        assert_ne!(tinted.base, Palette::dark().base);
        let (l, _, _) = hex_to_oklch(&format!("#{:06x}", tinted.base)).unwrap();
        assert!((l - 0.165).abs() < 0.01);
    }

    #[test]
    fn terminal_follows_palette() {
        assert_eq!(TerminalTheme::for_palette(&Palette::dark()), &TERMINAL_DARK);
        assert_eq!(TerminalTheme::for_palette(&Palette::light()), &TERMINAL_LIGHT);
    }
}
