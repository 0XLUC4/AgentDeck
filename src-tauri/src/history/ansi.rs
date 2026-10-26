//! Streaming ANSI/VT stripper: terminal output in, plain text lines out.
//! Escape sequences may be split across chunks, so the parser state is kept between calls.

#[derive(Default, Clone, Copy, PartialEq, Debug)]
enum State {
    #[default]
    Text,
    Esc,
    /// CSI: parameters until a final byte (0x40..=0x7E).
    Csi,
    /// OSC/DCS/APC/PM/SOS string: until BEL or ESC \.
    Str,
    StrEsc,
}

#[derive(Default)]
pub struct Stripper {
    state: State,
    /// Consecutive newlines just written: caps blank lines and avoids one per cursor move.
    newlines: u8,
    started: bool,
}

impl Stripper {
    pub fn push(&mut self, input: &str, out: &mut String) {
        for c in input.chars() {
            self.state = match self.state {
                State::Text => match c {
                    '\x1b' => State::Esc,
                    '\n' => {
                        self.emit('\n', out);
                        State::Text
                    }
                    '\t' => {
                        self.emit('\t', out);
                        State::Text
                    }
                    c if c.is_control() => State::Text, // \r, BEL, backspace...
                    c => {
                        self.emit(c, out);
                        State::Text
                    }
                },
                State::Esc => match c {
                    '[' => State::Csi,
                    ']' | 'P' | '_' | '^' | 'X' => State::Str,
                    _ => State::Text, // two-byte sequence (ESC 7, ESC =, ESC ( B approximated)
                },
                State::Csi => match c {
                    '\x40'..='\x7e' => {
                        // Cursor positioning / next line: what ConPTY uses instead of newlines.
                        if matches!(c, 'H' | 'f' | 'E') && self.started && self.newlines == 0 {
                            self.emit('\n', out);
                        }
                        State::Text
                    }
                    _ => State::Csi,
                },
                State::Str => match c {
                    '\x07' => State::Text,
                    '\x1b' => State::StrEsc,
                    _ => State::Str,
                },
                State::StrEsc => match c {
                    '\\' => State::Text,
                    '\x1b' => State::StrEsc,
                    _ => State::Str,
                },
            };
        }
    }

    fn emit(&mut self, c: char, out: &mut String) {
        if c == '\n' {
            if self.newlines >= 2 {
                return; // at most one blank line in a row
            }
            self.newlines += 1;
        } else {
            self.newlines = 0;
        }
        self.started = true;
        out.push(c);
    }
}

#[cfg(test)]
mod tests {
    use super::Stripper;

    fn strip(chunks: &[&str]) -> String {
        let mut s = Stripper::default();
        let mut out = String::new();
        for c in chunks {
            s.push(c, &mut out);
        }
        out
    }

    #[test]
    fn removes_colors_titles_and_carriage_returns() {
        assert_eq!(strip(&["\x1b[1;31mred\x1b[0m ok\r\n"]), "red ok\n");
        assert_eq!(strip(&["\x1b]0;title\x07after"]), "after");
        assert_eq!(strip(&["\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\"]), "link");
        assert_eq!(strip(&["a\x08b\x07c"]), "abc");
    }

    #[test]
    fn sequences_split_across_chunks() {
        assert_eq!(
            strip(&["x\x1b", "[3", "2mgreen\x1b]0;t", "itle\x07!"]),
            "xgreen!"
        );
    }

    #[test]
    fn cursor_positioning_becomes_one_newline() {
        assert_eq!(
            strip(&["one\x1b[2;1Htwo\x1b[3;1H\x1b[4;1Hthree"]),
            "one\ntwo\nthree"
        );
        assert_eq!(strip(&["a\n\n", "\n\n\nb"]), "a\n\nb");
        assert_eq!(strip(&["\x1b[1;1Hstart"]), "start");
    }

    #[test]
    fn keeps_unicode() {
        assert_eq!(strip(&["héllo ✓ 日本\n"]), "héllo ✓ 日本\n");
    }
}
