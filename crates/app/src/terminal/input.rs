//! Pure input encoding, same bytes as xterm.js: keys to escape sequences, mouse reports, paste
//! text, and URL lookup under the mouse.

use std::borrow::Cow;

use alacritty_terminal::term::TermMode;
use gpui_kit::{Keystroke, Modifiers};

/// xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl (only sent when it is not 1).
fn modifier_param(m: &Modifiers) -> u8 {
    1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.control as u8
}

/// Escape sequence for a key that is not plain text. None means: let the text input handle it
/// (printable characters, AltGr combos) or the key is not for the terminal.
pub fn key_to_esc(key: &Keystroke, mode: TermMode) -> Option<Cow<'static, str>> {
    let m = &key.modifiers;
    if m.platform || m.function {
        return None;
    }
    let param = modifier_param(m);
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    // CSI sequences ending in a letter: arrows, home, end, F1..F4.
    let letter = |l: char, ss3: bool| -> Cow<'static, str> {
        if param > 1 {
            format!("\x1b[1;{param}{l}").into()
        } else if ss3 {
            format!("\x1bO{l}").into()
        } else {
            format!("\x1b[{l}").into()
        }
    };
    let tilde = |n: u8| -> Cow<'static, str> {
        if param > 1 { format!("\x1b[{n};{param}~").into() } else { format!("\x1b[{n}~").into() }
    };
    let seq: Cow<'static, str> = match key.key.as_str() {
        "enter" => (if m.alt { "\x1b\r" } else { "\r" }).into(),
        "backspace" if m.control => "\x08".into(),
        "backspace" if m.alt => "\x1b\x7f".into(),
        "backspace" => "\x7f".into(),
        "tab" if m.shift => "\x1b[Z".into(),
        "tab" => "\t".into(),
        "escape" => (if m.alt { "\x1b\x1b" } else { "\x1b" }).into(),
        "up" => letter('A', app_cursor),
        "down" => letter('B', app_cursor),
        "right" => letter('C', app_cursor),
        "left" => letter('D', app_cursor),
        "home" => letter('H', app_cursor),
        "end" => letter('F', app_cursor),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => letter('P', true),
        "f2" => letter('Q', true),
        "f3" => letter('R', true),
        "f4" => letter('S', true),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "space" if m.control => "\x00".into(),
        _ => return control_or_alt(key),
    };
    Some(seq)
}

/// Ctrl+key -> C0 control char, Alt+key -> ESC prefix. Ctrl+Alt with a typed char is AltGr: text.
fn control_or_alt(key: &Keystroke) -> Option<Cow<'static, str>> {
    let m = &key.modifiers;
    if m.control && m.alt && key.key_char.is_some() {
        return None;
    }
    let mut chars = key.key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    if m.control {
        let code = match c.to_ascii_lowercase() {
            l @ 'a'..='z' => l as u8 - b'a' + 1,
            '@' | '2' => 0,
            '[' | '3' => 0x1b,
            '\\' | '4' => 0x1c,
            ']' | '5' => 0x1d,
            '^' | '6' => 0x1e,
            '_' | '-' | '/' | '7' => 0x1f,
            '8' => 0x7f,
            _ => return None,
        };
        let ctrl = (code as char).to_string();
        return Some(if m.alt { format!("\x1b{ctrl}") } else { ctrl }.into());
    }
    if m.alt {
        let typed = key.key_char.clone().unwrap_or_else(|| c.to_string());
        return Some(format!("\x1b{typed}").into());
    }
    None
}

/// Mouse button codes of the xterm protocol.
pub const MOUSE_LEFT: u8 = 0;
pub const MOUSE_MIDDLE: u8 = 1;
pub const MOUSE_RIGHT: u8 = 2;
pub const MOUSE_WHEEL_UP: u8 = 64;
pub const MOUSE_WHEEL_DOWN: u8 = 65;
/// Added to the button code for motion while a button is held.
pub const MOUSE_MOTION: u8 = 32;

/// Mouse report for a press/release at a 0-based cell. SGR when the program asked for it,
/// otherwise the legacy X10 encoding (limited to 95 columns/rows: None beyond).
pub fn mouse_report(button: u8, col: usize, row: usize, pressed: bool, m: &Modifiers, mode: TermMode) -> Option<String> {
    let mods = 4 * m.shift as u8 + 8 * m.alt as u8 + 16 * m.control as u8;
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{};{};{}{end}", button + mods, col + 1, row + 1));
    }
    // X10 has no release button: release is 3.
    let code = if pressed { button } else { 3 } + mods;
    let encode = |v: usize| char::from_u32(32 + v as u32 + 1).filter(|c| (*c as u32) < 128);
    let (x, y) = (encode(col)?, encode(row)?);
    Some(format!("\x1b[M{}{x}{y}", (32 + code) as char))
}

/// Text to write for a paste: newlines become CR like xterm.js, wrapped in bracketed-paste
/// markers when the program enabled them (end markers in the text are removed).
pub fn paste_text(text: &str, bracketed: bool) -> String {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        format!("\x1b[200~{}\x1b[201~", text.replace("\x1b[201~", ""))
    } else {
        text
    }
}

/// The http(s) URL covering column `col` of a row (one char per column), if any.
/// shortcut: URLs wrapped over two rows are cut at the row end; join wrapped rows if users ask.
pub fn url_at(row: &[char], col: usize) -> Option<String> {
    let stop = |c: &char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`');
    if col >= row.len() || stop(&row[col]) {
        return None;
    }
    let start = row[..col].iter().rposition(stop).map_or(0, |i| i + 1);
    let end = row[col..].iter().position(stop).map_or(row.len(), |i| col + i);
    let token: String = row[start..end].iter().collect();
    let scheme = token.find("https://").or_else(|| token.find("http://"))?;
    if start + token[..scheme].chars().count() > col {
        return None;
    }
    let url = token[scheme..].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}']);
    let url_end = start + token[..scheme].chars().count() + url.chars().count();
    (col < url_end && url.len() > "https://".len()).then(|| url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    fn key(spec: &'static str) -> Keystroke {
        Keystroke::parse(spec).expect("valid keystroke")
    }

    #[test]
    fn keys() {
        let none = TermMode::empty();
        assert_eq!(key_to_esc(&key("enter"), none).as_deref(), Some("\r"));
        assert_eq!(key_to_esc(&key("up"), none).as_deref(), Some("\x1b[A"));
        assert_eq!(key_to_esc(&key("up"), TermMode::APP_CURSOR).as_deref(), Some("\x1bOA"));
        assert_eq!(key_to_esc(&key("ctrl-left"), none).as_deref(), Some("\x1b[1;5D"));
        assert_eq!(key_to_esc(&key("shift-tab"), none).as_deref(), Some("\x1b[Z"));
        assert_eq!(key_to_esc(&key("ctrl-c"), none).as_deref(), Some("\x03"));
        assert_eq!(key_to_esc(&key("alt-b"), none).as_deref(), Some("\x1bb"));
        assert_eq!(key_to_esc(&key("delete"), none).as_deref(), Some("\x1b[3~"));
        assert_eq!(key_to_esc(&key("f5"), none).as_deref(), Some("\x1b[15~"));
        assert_eq!(key_to_esc(&key("a"), none), None);
    }

    #[test]
    fn mouse() {
        let m = Modifiers::default();
        assert_eq!(mouse_report(0, 0, 0, true, &m, TermMode::SGR_MOUSE).as_deref(), Some("\x1b[<0;1;1M"));
        assert_eq!(mouse_report(0, 4, 2, false, &m, TermMode::SGR_MOUSE).as_deref(), Some("\x1b[<0;5;3m"));
        assert_eq!(mouse_report(0, 0, 0, true, &m, TermMode::empty()).as_deref(), Some("\x1b[M !!"));
        assert_eq!(mouse_report(0, 200, 0, true, &m, TermMode::empty()), None);
    }

    #[test]
    fn paste() {
        assert_eq!(paste_text("a\r\nb\nc", false), "a\rb\rc");
        assert_eq!(paste_text("x\x1b[201~y", true), "\x1b[200~xy\x1b[201~");
    }

    #[test]
    fn urls() {
        let row: Vec<char> = "see (https://example.com/a?b=1). ok".chars().collect();
        assert_eq!(url_at(&row, 10).as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(url_at(&row, 2), None);
        assert_eq!(url_at(&row, 4), None);
        assert_eq!(url_at(&row, 31), None);
    }
}
