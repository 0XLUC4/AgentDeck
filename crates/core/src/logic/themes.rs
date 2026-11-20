//! Built-in color themes shipped as plugins (category "Themes" of `plugins::CATALOG`).
//!
//! Each variant lists the theme's published colors: surfaces, text, its seven hues and the code
//! colors. [`ThemeVariant::palette`] maps them onto the app's [`Palette`] (status, agents, accent).

use super::theme::{Palette, Syntax, mix, on_accent, parse_hex};

/// A theme's colors, as published by its authors (surfaces in between are interpolated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spec {
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
    pub selection: u32,
    pub red: u32,
    pub orange: u32,
    pub yellow: u32,
    pub green: u32,
    pub cyan: u32,
    pub blue: u32,
    pub purple: u32,
    pub syntax: Syntax,
}

/// One variant of a theme plugin (Tokyo Night has Night, Storm and Light).
#[derive(Debug)]
pub struct ThemeVariant {
    /// Saved in `Appearance::color_theme`.
    pub id: &'static str,
    /// Short name on the card ("Storm", "Dark").
    pub name: &'static str,
    /// Full name in Settings > Appearance ("Tokyo Night Storm").
    pub title: &'static str,
    pub spec: Spec,
}

impl ThemeVariant {
    pub fn palette(&self) -> Palette {
        let s = &self.spec;
        let light = s.base > 0x808080;
        let accent_hex = format!("#{:06x}", s.accent);
        // Agent logos keep their brand colors in every theme: only their light/dark tone follows.
        let brand = if light { Palette::light() } else { Palette::dark() };
        Palette {
            gutter: s.gutter,
            base: s.base,
            panel: s.panel,
            raised: s.raised,
            hover: s.hover,
            line: s.line,
            line_strong: s.line_strong,
            fg: s.fg,
            muted: s.muted,
            subtle: s.subtle,
            accent: s.accent,
            accent_hover: mix(s.accent, if light { 0x000000 } else { 0xffffff }, 0.85),
            on_accent: parse_hex(on_accent(&accent_hex)).unwrap_or(s.base),
            ok: s.green,
            busy: s.blue,
            wait: s.yellow,
            bad: s.red,
            close: s.red,
            agent_claude: brand.agent_claude,
            agent_codex: brand.agent_codex,
            agent_gemini: brand.agent_gemini,
            agent_grok: brand.agent_grok,
            agent_opencode: brand.agent_opencode,
            agent_shell: brand.agent_shell,
            selection: s.selection,
            syntax: s.syntax,
        }
    }

    pub fn is_light(&self) -> bool {
        self.spec.base > 0x808080
    }
}

const fn syntax(keyword: u32, string: u32, function: u32, comment: u32, number: u32, type_: u32, tag: u32) -> Syntax {
    Syntax { keyword, string, function, comment, number, type_, tag }
}

pub const MONOKAI: &[ThemeVariant] = &[ThemeVariant {
    id: "monokai",
    name: "Dark",
    title: "Monokai",
    spec: Spec {
        gutter: 0x1e1f1c,
        base: 0x272822,
        panel: 0x2d2e27,
        raised: 0x3e3d32,
        hover: 0x49483e,
        line: 0x3b3a32,
        line_strong: 0x5b5a4e,
        fg: 0xf8f8f2,
        muted: 0xcfcfc2,
        subtle: 0x90908a,
        accent: 0xa6e22e,
        selection: 0x49483e,
        red: 0xf92672,
        orange: 0xfd971f,
        yellow: 0xe6db74,
        green: 0xa6e22e,
        cyan: 0x66d9ef,
        blue: 0x66d9ef,
        purple: 0xae81ff,
        syntax: syntax(0xf92672, 0xe6db74, 0xa6e22e, 0x75715e, 0xae81ff, 0x66d9ef, 0xf92672),
    },
}];

/// Pure grayscale: status and agents differ by lightness only.
pub const MONO: &[ThemeVariant] = &[
    ThemeVariant {
        id: "mono-dark",
        name: "Dark",
        title: "Mono Dark",
        spec: Spec {
            gutter: 0x0a0a0a,
            base: 0x111111,
            panel: 0x161616,
            raised: 0x1f1f1f,
            hover: 0x262626,
            line: 0x2a2a2a,
            line_strong: 0x3a3a3a,
            fg: 0xe6e6e6,
            muted: 0xb3b3b3,
            subtle: 0x808080,
            accent: 0xffffff,
            selection: 0x3a3a3a,
            red: 0xf2f2f2,
            orange: 0xcccccc,
            yellow: 0xbdbdbd,
            green: 0xa8a8a8,
            cyan: 0x9a9a9a,
            blue: 0xd9d9d9,
            purple: 0xffffff,
            syntax: syntax(0xffffff, 0x9e9e9e, 0xd9d9d9, 0x5c5c5c, 0xbdbdbd, 0xcccccc, 0xffffff),
        },
    },
    ThemeVariant {
        id: "mono-light",
        name: "Light",
        title: "Mono Light",
        spec: Spec {
            gutter: 0xebebeb,
            base: 0xffffff,
            panel: 0xf7f7f7,
            raised: 0xeeeeee,
            hover: 0xe5e5e5,
            line: 0xe0e0e0,
            line_strong: 0xc7c7c7,
            fg: 0x171717,
            muted: 0x4d4d4d,
            subtle: 0x8a8a8a,
            accent: 0x000000,
            selection: 0xd9d9d9,
            red: 0x1a1a1a,
            orange: 0x404040,
            yellow: 0x595959,
            green: 0x4d4d4d,
            cyan: 0x666666,
            blue: 0x333333,
            purple: 0x000000,
            syntax: syntax(0x000000, 0x666666, 0x262626, 0xa3a3a3, 0x404040, 0x333333, 0x000000),
        },
    },
];

/// enkia/tokyo-night-vscode-theme and folke/tokyonight.nvim.
pub const TOKYO_NIGHT: &[ThemeVariant] = &[
    ThemeVariant {
        id: "tokyo-night",
        name: "Night",
        title: "Tokyo Night",
        spec: Spec {
            gutter: 0x16161e,
            base: 0x1a1b26,
            panel: 0x1f2030,
            raised: 0x292e42,
            hover: 0x30364d,
            line: 0x24273a,
            line_strong: 0x3b4261,
            fg: 0xc0caf5,
            muted: 0xa9b1d6,
            subtle: 0x737aa2,
            accent: 0x7aa2f7,
            selection: 0x283457,
            red: 0xf7768e,
            orange: 0xff9e64,
            yellow: 0xe0af68,
            green: 0x9ece6a,
            cyan: 0x7dcfff,
            blue: 0x7aa2f7,
            purple: 0xbb9af7,
            syntax: syntax(0xbb9af7, 0x9ece6a, 0x7aa2f7, 0x565f89, 0xff9e64, 0x2ac3de, 0xf7768e),
        },
    },
    ThemeVariant {
        id: "tokyo-night-storm",
        name: "Storm",
        title: "Tokyo Night Storm",
        spec: Spec {
            gutter: 0x1f2335,
            base: 0x24283b,
            panel: 0x282c40,
            raised: 0x2f3449,
            hover: 0x363c55,
            line: 0x2c3148,
            line_strong: 0x3b4261,
            fg: 0xc0caf5,
            muted: 0xa9b1d6,
            subtle: 0x737aa2,
            accent: 0x7aa2f7,
            selection: 0x2e3c64,
            red: 0xf7768e,
            orange: 0xff9e64,
            yellow: 0xe0af68,
            green: 0x9ece6a,
            cyan: 0x7dcfff,
            blue: 0x7aa2f7,
            purple: 0xbb9af7,
            syntax: syntax(0xbb9af7, 0x9ece6a, 0x7aa2f7, 0x565f89, 0xff9e64, 0x2ac3de, 0xf7768e),
        },
    },
    ThemeVariant {
        id: "tokyo-night-light",
        name: "Light",
        title: "Tokyo Night Light",
        spec: Spec {
            gutter: 0xcbccd1,
            base: 0xd5d6db,
            panel: 0xdcdde2,
            raised: 0xcbccd1,
            hover: 0xc4c5cb,
            line: 0xc4c5cb,
            line_strong: 0xa8aab2,
            fg: 0x343b58,
            muted: 0x565a6e,
            subtle: 0x6c6e75,
            accent: 0x2959aa,
            selection: 0xb7b9c3,
            red: 0x8c4351,
            orange: 0x965027,
            yellow: 0x8f5e15,
            green: 0x385f0d,
            cyan: 0x33635c,
            blue: 0x2959aa,
            purple: 0x5a3e8e,
            syntax: syntax(0x5a3e8e, 0x385f0d, 0x2959aa, 0x9699a3, 0x965027, 0x0f4b6e, 0x8c4351),
        },
    },
];

/// draculatheme.com/contribute.
pub const DRACULA: &[ThemeVariant] = &[ThemeVariant {
    id: "dracula",
    name: "Dark",
    title: "Dracula",
    spec: Spec {
        gutter: 0x191a21,
        base: 0x282a36,
        panel: 0x21222c,
        raised: 0x343746,
        hover: 0x44475a,
        line: 0x343746,
        line_strong: 0x4d5066,
        fg: 0xf8f8f2,
        muted: 0xd6d6d0,
        subtle: 0x8a93c4,
        accent: 0xbd93f9,
        selection: 0x44475a,
        red: 0xff5555,
        orange: 0xffb86c,
        yellow: 0xf1fa8c,
        green: 0x50fa7b,
        cyan: 0x8be9fd,
        blue: 0x8be9fd,
        purple: 0xbd93f9,
        syntax: syntax(0xff79c6, 0xf1fa8c, 0x50fa7b, 0x6272a4, 0xbd93f9, 0x8be9fd, 0xff79c6),
    },
}];

/// nordtheme.com (nord0..nord15).
pub const NORD: &[ThemeVariant] = &[ThemeVariant {
    id: "nord",
    name: "Dark",
    title: "Nord",
    spec: Spec {
        gutter: 0x272c36,
        base: 0x2e3440,
        panel: 0x323846,
        raised: 0x3b4252,
        hover: 0x434c5e,
        line: 0x3b4252,
        line_strong: 0x4c566a,
        fg: 0xeceff4,
        muted: 0xd8dee9,
        subtle: 0x7b88a1,
        accent: 0x88c0d0,
        selection: 0x434c5e,
        red: 0xbf616a,
        orange: 0xd08770,
        yellow: 0xebcb8b,
        green: 0xa3be8c,
        cyan: 0x8fbcbb,
        blue: 0x81a1c1,
        purple: 0xb48ead,
        syntax: syntax(0x81a1c1, 0xa3be8c, 0x88c0d0, 0x616e88, 0xb48ead, 0x8fbcbb, 0x81a1c1),
    },
}];

/// morhetz/gruvbox, dark medium contrast.
pub const GRUVBOX: &[ThemeVariant] = &[ThemeVariant {
    id: "gruvbox-dark",
    name: "Dark",
    title: "Gruvbox Dark",
    spec: Spec {
        gutter: 0x1d2021,
        base: 0x282828,
        panel: 0x32302f,
        raised: 0x3c3836,
        hover: 0x504945,
        line: 0x3c3836,
        line_strong: 0x665c54,
        fg: 0xebdbb2,
        muted: 0xd5c4a1,
        subtle: 0xa89984,
        accent: 0xfe8019,
        selection: 0x504945,
        red: 0xfb4934,
        orange: 0xfe8019,
        yellow: 0xfabd2f,
        green: 0xb8bb26,
        cyan: 0x8ec07c,
        blue: 0x83a598,
        purple: 0xd3869b,
        syntax: syntax(0xfb4934, 0xb8bb26, 0x8ec07c, 0x928374, 0xd3869b, 0xfabd2f, 0xfb4934),
    },
}];

/// catppuccin.com/palette.
pub const CATPPUCCIN: &[ThemeVariant] = &[
    ThemeVariant {
        id: "catppuccin-mocha",
        name: "Mocha",
        title: "Catppuccin Mocha",
        spec: Spec {
            gutter: 0x181825,
            base: 0x1e1e2e,
            panel: 0x252536,
            raised: 0x313244,
            hover: 0x45475a,
            line: 0x313244,
            line_strong: 0x45475a,
            fg: 0xcdd6f4,
            muted: 0xbac2de,
            subtle: 0x9399b2,
            accent: 0xcba6f7,
            selection: 0x45475a,
            red: 0xf38ba8,
            orange: 0xfab387,
            yellow: 0xf9e2af,
            green: 0xa6e3a1,
            cyan: 0x94e2d5,
            blue: 0x89b4fa,
            purple: 0xcba6f7,
            syntax: syntax(0xcba6f7, 0xa6e3a1, 0x89b4fa, 0x9399b2, 0xfab387, 0xf9e2af, 0xcba6f7),
        },
    },
    ThemeVariant {
        id: "catppuccin-latte",
        name: "Latte",
        title: "Catppuccin Latte",
        spec: Spec {
            gutter: 0xdce0e8,
            base: 0xeff1f5,
            panel: 0xe6e9ef,
            raised: 0xdce0e8,
            hover: 0xccd0da,
            line: 0xccd0da,
            line_strong: 0xacb0be,
            fg: 0x4c4f69,
            muted: 0x5c5f77,
            subtle: 0x7c7f93,
            accent: 0x8839ef,
            selection: 0xbcc0cc,
            red: 0xd20f39,
            orange: 0xfe640b,
            yellow: 0xdf8e1d,
            green: 0x40a02b,
            cyan: 0x179299,
            blue: 0x1e66f5,
            purple: 0x8839ef,
            syntax: syntax(0x8839ef, 0x40a02b, 0x1e66f5, 0x7c7f93, 0xfe640b, 0xdf8e1d, 0x8839ef),
        },
    },
];

/// Atom One Dark.
pub const ONE_DARK: &[ThemeVariant] = &[ThemeVariant {
    id: "one-dark",
    name: "Dark",
    title: "One Dark",
    spec: Spec {
        gutter: 0x21252b,
        base: 0x282c34,
        panel: 0x2c313a,
        raised: 0x333842,
        hover: 0x3a3f4b,
        line: 0x30353f,
        line_strong: 0x4b5263,
        fg: 0xd7dae0,
        muted: 0xabb2bf,
        subtle: 0x7f848e,
        accent: 0x61afef,
        selection: 0x3e4451,
        red: 0xe06c75,
        orange: 0xd19a66,
        yellow: 0xe5c07b,
        green: 0x98c379,
        cyan: 0x56b6c2,
        blue: 0x61afef,
        purple: 0xc678dd,
        syntax: syntax(0xc678dd, 0x98c379, 0x61afef, 0x5c6370, 0xd19a66, 0xe5c07b, 0xe06c75),
    },
}];

/// ethanschoonover.com/solarized (base03..base3 and the eight accents).
pub const SOLARIZED: &[ThemeVariant] = &[
    ThemeVariant {
        id: "solarized-dark",
        name: "Dark",
        title: "Solarized Dark",
        spec: Spec {
            gutter: 0x00212b,
            base: 0x002b36,
            panel: 0x03303c,
            raised: 0x073642,
            hover: 0x0b4250,
            line: 0x073642,
            line_strong: 0x1e4b57,
            fg: 0x93a1a1,
            muted: 0x839496,
            subtle: 0x657b83,
            accent: 0x268bd2,
            selection: 0x274642,
            red: 0xdc322f,
            orange: 0xcb4b16,
            yellow: 0xb58900,
            green: 0x859900,
            cyan: 0x2aa198,
            blue: 0x268bd2,
            purple: 0x6c71c4,
            syntax: syntax(0x859900, 0x2aa198, 0x268bd2, 0x586e75, 0xd33682, 0xb58900, 0x268bd2),
        },
    },
    ThemeVariant {
        id: "solarized-light",
        name: "Light",
        title: "Solarized Light",
        spec: Spec {
            gutter: 0xeee8d5,
            base: 0xfdf6e3,
            panel: 0xf7f0dc,
            raised: 0xeee8d5,
            hover: 0xe6dfc9,
            line: 0xe6dfc9,
            line_strong: 0xd3cbb7,
            fg: 0x586e75,
            muted: 0x657b83,
            subtle: 0x93a1a1,
            accent: 0x268bd2,
            selection: 0xe3dcc6,
            red: 0xdc322f,
            orange: 0xcb4b16,
            yellow: 0xb58900,
            green: 0x859900,
            cyan: 0x2aa198,
            blue: 0x268bd2,
            purple: 0x6c71c4,
            syntax: syntax(0x859900, 0x2aa198, 0x268bd2, 0x93a1a1, 0xd33682, 0xb58900, 0x268bd2),
        },
    },
];

/// GitHub's Primer colors (github-vscode-theme).
pub const GITHUB: &[ThemeVariant] = &[
    ThemeVariant {
        id: "github-dark",
        name: "Dark",
        title: "GitHub Dark",
        spec: Spec {
            gutter: 0x010409,
            base: 0x0d1117,
            panel: 0x161b22,
            raised: 0x21262d,
            hover: 0x262c36,
            line: 0x21262d,
            line_strong: 0x30363d,
            fg: 0xe6edf3,
            muted: 0xc9d1d9,
            subtle: 0x8b949e,
            accent: 0x2f81f7,
            selection: 0x264f78,
            red: 0xf85149,
            orange: 0xdb6d28,
            yellow: 0xd29922,
            green: 0x3fb950,
            cyan: 0x39c5cf,
            blue: 0x58a6ff,
            purple: 0xa371f7,
            syntax: syntax(0xff7b72, 0xa5d6ff, 0xd2a8ff, 0x8b949e, 0x79c0ff, 0xffa657, 0x7ee787),
        },
    },
    ThemeVariant {
        id: "github-light",
        name: "Light",
        title: "GitHub Light",
        spec: Spec {
            gutter: 0xf6f8fa,
            base: 0xffffff,
            panel: 0xf6f8fa,
            raised: 0xeaeef2,
            hover: 0xe1e6ea,
            line: 0xd8dee4,
            line_strong: 0xd0d7de,
            fg: 0x1f2328,
            muted: 0x424a53,
            subtle: 0x6e7781,
            accent: 0x0969da,
            selection: 0xb6e3ff,
            red: 0xcf222e,
            orange: 0xbc4c00,
            yellow: 0x9a6700,
            green: 0x1a7f37,
            cyan: 0x1b7c83,
            blue: 0x0969da,
            purple: 0x8250df,
            syntax: syntax(0xcf222e, 0x0a3069, 0x8250df, 0x6e7781, 0x0550ae, 0x953800, 0x116329),
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [&[ThemeVariant]; 10] =
        [MONOKAI, MONO, TOKYO_NIGHT, DRACULA, NORD, GRUVBOX, CATPPUCCIN, ONE_DARK, SOLARIZED, GITHUB];

    #[test]
    fn variants_are_unique_and_light_ones_look_light() {
        let mut ids: Vec<&str> = ALL.iter().flat_map(|t| t.iter().map(|v| v.id)).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate theme id");
        for variant in ALL.iter().flat_map(|t| t.iter()) {
            let light = variant.name == "Light" || variant.name == "Latte";
            assert_eq!(variant.is_light(), light, "{}", variant.id);
            let p = variant.palette();
            assert_eq!(p.is_light(), light, "{}", variant.id);
            // Text stands out from the background.
            let red = |c: u32| i64::from((c >> 16) & 0xff);
            assert!((red(p.fg) - red(p.base)).abs() > 0x60, "{}: fg too close to base", variant.id);
        }
    }

    #[test]
    fn palette_maps_hues_and_syntax() {
        let p = MONOKAI[0].palette();
        assert_eq!((p.base, p.ok, p.bad), (0x272822, 0xa6e22e, 0xf92672));
        // Logos keep their brand colors whatever the theme.
        assert_eq!((p.agent_claude, p.agent_codex), (Palette::dark().agent_claude, Palette::dark().agent_codex));
        assert_eq!(p.syntax.keyword, 0xf92672);
        assert_eq!(p.syntax.comment, 0x75715e);
        assert_eq!(p.on_accent, 0x141414);
    }
}
