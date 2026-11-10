//! Small text helpers ported from `src/lib`: splitArgs, fileRefs, shortAction, shortcut, tts, clock, agents.

use std::sync::LazyLock;

use regex::Regex;

use crate::agents::AgentKind;

/// Splits a CLI argument string on whitespace; "double" or 'single' quotes group words.
pub fn split_args(input: &str) -> Vec<String> {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#""([^"]*)"|'([^']*)'|(\S+)"#).expect("static regex"));
    RE.captures_iter(input)
        .map(|c| {
            c.get(1)
                .or_else(|| c.get(2))
                .or_else(|| c.get(3))
                .map_or(String::new(), |m| m.as_str().to_owned())
        })
        .collect()
}

/// Paths as the agent expects them, space-separated with a trailing space: Gemini wants `@path`,
/// others a (quoted) path.
pub fn file_refs(agent: AgentKind, paths: &[String]) -> String {
    let refs: Vec<String> = paths
        .iter()
        .map(|p| {
            if agent == AgentKind::Gemini {
                format!("@{}", p.replace(' ', "\\ "))
            } else if p.chars().any(char::is_whitespace) {
                format!("\"{p}\"")
            } else {
                p.clone()
            }
        })
        .collect();
    if refs.is_empty() {
        String::new()
    } else {
        format!("{} ", refs.join(" "))
    }
}

/// File extension for a clipboard image mime type, or None when it is not an image we save.
pub fn image_ext(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/bmp" => Some("bmp"),
        _ => None,
    }
}

/// Key that interrupts the current task without killing the session: Esc for agents, Ctrl+C for a shell.
pub fn interrupt_key(kind: AgentKind) -> &'static str {
    if kind == AgentKind::Shell { "\x03" } else { "\x1b" }
}

fn words(text: &str, n: usize) -> String {
    text.split_whitespace().take(n).collect::<Vec<_>>().join(" ")
}

fn base(path: &str) -> &str {
    path.split('/').filter(|s| !s.is_empty()).last().unwrap_or(path)
}

/// Subject of a conversation, from the terminal title Claude Code sets ("✳ Fix the login bug"):
/// the status glyph is dropped. Its default title and the process path conhost announces are none.
pub fn topic(title: &str) -> Option<String> {
    let text = title.trim_start_matches(|c: char| !c.is_alphanumeric()).trim();
    let default = text.is_empty() || text.eq_ignore_ascii_case("claude code") || text.eq_ignore_ascii_case("claude");
    let path = text.contains(":\\") || text.ends_with(".exe") || text.ends_with(".cmd");
    (!default && !path).then(|| text.to_owned())
}

/// Few-word version of a pane's last action for the header ("Editing deck.ts", "Running pnpm test").
pub fn short_action(action: Option<&str>) -> String {
    let Some(action) = action.filter(|a| !a.is_empty()) else {
        return String::new();
    };
    if let Some(cmd) = action.strip_prefix("Bash: ") {
        return format!("Running {}", words(cmd, 3));
    }
    let (tool, rest) = action.split_once(' ').unwrap_or((action, ""));
    // mcp__server__tool_name -> "tool name"
    static MCP: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^mcp__[^_]+(?:_[^_]+)*?__(.+)$").expect("static regex"));
    if let Some(c) = MCP.captures(tool) {
        return c[1].replace('_', " ");
    }
    if rest.is_empty() {
        return words(action, 4);
    }
    match tool {
        "Read" => format!("Reading {}", base(rest)),
        "Edit" | "MultiEdit" | "NotebookEdit" => format!("Editing {}", base(rest)),
        "Write" => format!("Writing {}", base(rest)),
        "Grep" => format!("Searching {}", words(rest, 3)),
        "Glob" => format!("Finding {}", words(rest, 3)),
        "WebFetch" => {
            let host = rest
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            format!("Opening {}", host.split('/').next().unwrap_or(""))
        }
        "WebSearch" => format!("Searching web: {}", words(rest, 3)),
        _ => words(action, 4),
    }
}

/// A key press, layout independent (`code` is KeyboardEvent.code style: "KeyA", "Digit1", "F9", "Space").
#[derive(Debug, Clone, Default)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    pub code: String,
}

/// Global-shortcut string for a key press ("ctrl+shift+f9"), or None while only modifiers are held.
pub fn to_shortcut(e: &KeyCombo) -> Option<String> {
    static MODS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(Control|Shift|Alt|Meta|OS)(Left|Right)?$").expect("static regex")
    });
    if e.code.is_empty() || MODS.is_match(&e.code) {
        return None;
    }
    let key = e
        .code
        .strip_prefix("Key")
        .or_else(|| e.code.strip_prefix("Digit"))
        .unwrap_or(&e.code)
        .to_lowercase();
    let mut parts: Vec<&str> = [(e.ctrl, "ctrl"), (e.alt, "alt"), (e.shift, "shift"), (e.meta, "super")]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
    parts.push(&key);
    Some(parts.join("+"))
}

/// "ctrl+space" -> "Ctrl + Space".
pub fn format_shortcut(shortcut: &str) -> String {
    shortcut
        .split('+')
        .filter(|p| !p.is_empty())
        .map(|part| {
            let label = match part.to_lowercase().as_str() {
                "ctrl" | "control" | "commandorcontrol" => "Ctrl",
                "alt" => "Alt",
                "shift" => "Shift",
                "super" => "Win",
                "space" => "Space",
                "enter" => "Enter",
                "backspace" => "Backspace",
                "arrowup" => "\u{2191}",
                "arrowdown" => "\u{2193}",
                "arrowleft" => "\u{2190}",
                "arrowright" => "\u{2192}",
                _ => "",
            };
            if label.is_empty() {
                let mut chars = part.chars();
                chars
                    .next()
                    .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            } else {
                label.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

pub const SPEECH_MAX_CHARS: usize = 200;

/// Markdown/code to plain speakable text, cut at a word boundary after `max` chars.
pub fn clean_for_speech(text: &str, max: usize) -> String {
    static RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        [
            (r"```[\s\S]*?(```|$)", " "),
            (r"`([^`]*)`", "$1"),
            (r"!?\[([^\]]*)\]\([^)]*\)", "$1"),
            (r"https?://\S+", " "),
            (r"(?m)^\s*(#{1,6}|>|[-*+]|\d+[.)])\s+", ""),
            (r"[*~#|]+", ""),
            (r"\s+", " "),
        ]
        .into_iter()
        .map(|(re, to)| (Regex::new(re).expect("static regex"), to))
        .collect()
    });
    let mut plain = text.to_owned();
    for (re, to) in RULES.iter() {
        plain = re.replace_all(&plain, *to).into_owned();
    }
    let plain = plain.trim();
    let chars: Vec<char> = plain.chars().collect();
    if chars.len() <= max {
        return plain.to_owned();
    }
    let cut: String = chars[..max].iter().collect();
    let kept = match cut.rfind(' ') {
        Some(space) if cut[..space].chars().count() as f64 > max as f64 * 0.6 => &cut[..space],
        _ => cut.as_str(),
    };
    format!(
        "{}\u{2026}",
        kept.trim_end_matches(|c: char| c.is_whitespace() || ",;:.-".contains(c))
    )
}

/// "<Agent> finished: <summary>".
pub fn speech_text(agent_name: &str, summary: Option<&str>) -> String {
    let clean = summary.map_or(String::new(), |s| clean_for_speech(s, SPEECH_MAX_CHARS));
    if clean.is_empty() {
        format!("{agent_name} finished")
    } else {
        format!("{agent_name} finished: {clean}")
    }
}

/// "mm:ss" or "h:mm:ss".
pub fn format_elapsed(ms: i64) -> String {
    let total = ms.max(0) / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Coarse duration for lists: "<1 min", "12 min", "2 h 05".
pub fn format_since(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0 => "<1 min".into(),
        1..=59 => format!("{minutes} min"),
        _ => format!("{} h {:02}", minutes / 60, minutes % 60),
    }
}

/// Budget: a limit of 0 is off.
pub fn over_limit(cost: f64, limit: f64) -> bool {
    limit > 0.0 && cost >= limit
}

/// Display name and theme color token per agent (`src/lib/agents.ts`).
pub fn agent_name(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "Claude Code",
        AgentKind::Codex => "Codex",
        AgentKind::Gemini => "Gemini CLI",
        AgentKind::Grok => "Grok Build",
        AgentKind::OpenCode => "OpenCode",
        AgentKind::Shell => "Shell",
    }
}

/// Every agent kind, in picker order.
pub const AGENT_KINDS: [AgentKind; 6] = [
    AgentKind::Claude,
    AgentKind::Codex,
    AgentKind::Gemini,
    AgentKind::Grok,
    AgentKind::OpenCode,
    AgentKind::Shell,
];

/// en-US number with up to `digits` decimals and thousands separators ("12,345.6").
fn decimal(n: f64, digits: usize, min_digits: usize) -> String {
    let fixed = format!("{n:.digits$}");
    let (int, frac) = fixed.split_once('.').unwrap_or((&fixed, ""));
    let mut frac = frac.trim_end_matches('0').to_owned();
    while frac.len() < min_digits {
        frac.push('0');
    }
    let digits_only = int.trim_start_matches('-');
    let mut grouped = String::new();
    for (i, c) in digits_only.chars().enumerate() {
        if i > 0 && (digits_only.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let sign = if int.starts_with('-') { "-" } else { "" };
    if frac.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{frac}")
    }
}

/// Usage tokens: 950 -> "950", 12,345 -> "12k", 1,234,567 -> "1.2M", 5.4e9 -> "5.4B".
pub fn format_tokens(n: f64) -> String {
    if n < 1e3 {
        format!("{}", n.round())
    } else if n < 1e6 {
        format!("{}k", decimal(n / 1e3, if n < 1e4 { 1 } else { 0 }, 0))
    } else if n < 1e9 {
        format!("{}M", decimal(n / 1e6, if n < 1e8 { 1 } else { 0 }, 0))
    } else {
        format!("{}B", decimal(n / 1e9, 1, 0))
    }
}

/// "$0.31", "$2,061.04".
pub fn format_cost(usd: f64) -> String {
    format!("${}", decimal(usd, 2, 2))
}

/// Last non-blank lines of raw terminal output as plain text (wall cards). A bare \r rewinds the line.
pub fn last_lines(raw: &str, count: usize) -> Vec<String> {
    static ANSI: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()#][0-9A-Za-z]|\x1b[0-~]")
            .expect("static regex")
    });
    let plain = ANSI.replace_all(raw, "");
    let lines: Vec<String> = plain
        .split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let last = line.split('\r').filter(|s| !s.is_empty()).last().unwrap_or("");
            last.chars()
                .filter(|c| !matches!(*c as u32, 0..=8 | 0x0b..=0x1f | 0x7f))
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .filter(|l| !l.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(count)..].to_vec()
}

/// "now" in ms since the epoch, like `Date.now()`.
/// Status event name for a desktop notification an agent printed to its terminal.
pub const TERMINAL_NOTICE: &str = "terminal-notification";

/// Text of the last desktop notification escape in `output`: OSC 9 (`ESC ] 9 ; text BEL`, Codex,
/// Windows Terminal style) or OSC 777 (`ESC ] 777 ; notify ; title ; body BEL`). ConEmu progress
/// (`9;4;...`) is not a notification. A sequence split across two chunks is missed.
pub fn osc_notification(output: &str) -> Option<String> {
    let mut found = None;
    let mut rest = output;
    while let Some(start) = rest.find("]") {
        rest = &rest[start + 2..];
        let end = rest.find(['', '']).unwrap_or(rest.len());
        let body = &rest[..end];
        let text = if let Some(t) = body.strip_prefix("9;") {
            (!t.starts_with("4;")).then_some(t)
        } else {
            body.strip_prefix("777;notify;")
        };
        if let Some(text) = text.map(|t| t.replace(';', " ").trim().to_owned()).filter(|t| !t.is_empty()) {
            found = Some(text);
        }
        rest = &rest[end..];
    }
    found
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_terminal_notifications() {
        use super::osc_notification as osc;
        assert_eq!(osc("x]9;Approval requested: lsy").as_deref(), Some("Approval requested: ls"));
        assert_eq!(osc("]777;notify;Codex;Needs you\\").as_deref(), Some("Codex Needs you"));
        assert_eq!(osc("]9;4;1;50"), None); // progress bar
        assert_eq!(osc("]0;window titleplain text"), None);
        assert_eq!(osc("no escapes"), None);
    }

    use super::*;

    #[test]
    fn split_args_cases() {
        assert_eq!(split_args("  --model  sonnet -v "), ["--model", "sonnet", "-v"]);
        assert_eq!(
            split_args(r#"-p "hello world" 'a b' """#),
            ["-p", "hello world", "a b", ""]
        );
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn file_refs_cases() {
        let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            file_refs(AgentKind::Claude, &p(&["C:\\a.png", "C:\\my pics\\b.png"])),
            "C:\\a.png \"C:\\my pics\\b.png\" "
        );
        assert_eq!(
            file_refs(AgentKind::Gemini, &p(&["C:\\my pics\\b.png"])),
            "@C:\\my\\ pics\\b.png "
        );
        assert_eq!(file_refs(AgentKind::Codex, &[]), "");
        assert_eq!(image_ext("image/jpeg"), Some("jpg"));
        assert_eq!(image_ext("text/plain"), None);
    }

    #[test]
    fn topic_from_terminal_title() {
        use super::topic;
        assert_eq!(topic("✳ Fix the login bug"), Some("Fix the login bug".into()));
        assert_eq!(topic("⠂ Étapes des agents "), Some("Étapes des agents".into()));
        for none in ["", "✳ Claude Code", "claude", "C:\\Windows\\system32\\cmd.exe", "node.exe"] {
            assert_eq!(topic(none), None, "{none}");
        }
    }

    #[test]
    fn short_action_cases() {
        assert_eq!(short_action(Some("Edit src/store/deck.ts")), "Editing deck.ts");
        assert_eq!(
            short_action(Some("Bash: pnpm test --run src/x")),
            "Running pnpm test --run"
        );
        assert_eq!(
            short_action(Some("WebFetch https://docs.rs/tauri/latest")),
            "Opening docs.rs"
        );
        assert_eq!(short_action(Some("mcp__agentdeck__send_prompt")), "send prompt");
        assert_eq!(short_action(Some("TodoWrite")), "TodoWrite");
        assert_eq!(short_action(None), "");
    }

    fn press(code: &str, ctrl: bool, shift: bool, alt: bool, meta: bool) -> KeyCombo {
        KeyCombo {
            ctrl,
            shift,
            alt,
            meta,
            code: code.into(),
        }
    }

    #[test]
    fn shortcut_cases() {
        let s = |k: KeyCombo| to_shortcut(&k);
        assert_eq!(s(press("Space", true, false, false, false)).as_deref(), Some("ctrl+space"));
        assert_eq!(s(press("F9", true, true, false, false)).as_deref(), Some("ctrl+shift+f9"));
        assert_eq!(s(press("KeyA", false, false, true, true)).as_deref(), Some("alt+super+a"));
        assert_eq!(s(press("Digit1", true, false, false, false)).as_deref(), Some("ctrl+1"));
        assert_eq!(s(press("F13", false, false, false, false)).as_deref(), Some("f13"));
        assert_eq!(s(press("KeyK", true, true, true, false)).as_deref(), Some("ctrl+alt+shift+k"));
        assert_eq!(s(press("ControlLeft", true, false, false, false)), None);
        assert_eq!(s(press("ShiftRight", false, true, false, false)), None);
        assert_eq!(s(press("MetaLeft", false, false, false, false)), None);
        assert_eq!(s(press("", false, false, false, false)), None);
        assert_eq!(format_shortcut("ctrl+space"), "Ctrl + Space");
        assert_eq!(format_shortcut("ctrl+shift+f9"), "Ctrl + Shift + F9");
        assert_eq!(format_shortcut("CommandOrControl+Shift+A"), "Ctrl + Shift + A");
    }

    #[test]
    fn speech_cleanup() {
        let md = "## Done\n\n- Added `parseArgs` in **src/lib**\n- See [the docs](https://x.dev)\n\n```ts\nconst a = 1;\n```\nFinished.";
        assert_eq!(
            clean_for_speech(md, SPEECH_MAX_CHARS),
            "Done Added parseArgs in src/lib See the docs Finished."
        );
        assert_eq!(
            clean_for_speech("See https://example.com here\n```\ncode", SPEECH_MAX_CHARS),
            "See here"
        );
        let long = clean_for_speech(&"word ".repeat(100), SPEECH_MAX_CHARS);
        assert!(long.chars().count() <= 201);
        assert!(long.ends_with("word\u{2026}"));
        assert_eq!(speech_text("Codex", Some("Tests **green**.")), "Codex finished: Tests green.");
        assert_eq!(speech_text("Codex", Some("```\nx\n```")), "Codex finished");
        assert_eq!(speech_text("Codex", None), "Codex finished");
    }

    #[test]
    fn usage_formats() {
        assert_eq!(format_tokens(950.0), "950");
        assert_eq!(format_tokens(1234.0), "1.2k");
        assert_eq!(format_tokens(12_345.0), "12k");
        assert_eq!(format_tokens(1_234_567.0), "1.2M");
        assert_eq!(format_tokens(5.4e9), "5.4B");
        assert_eq!(format_cost(0.31), "$0.31");
        assert_eq!(format_cost(2061.04), "$2,061.04");
        assert_eq!(format_cost(3.0), "$3.00");
    }

    #[test]
    fn last_lines_strip_ansi_and_cr() {
        let raw = "\x1b[32mok\x1b[0m\r\nloading 10%\rloading 100%\n\n  \nlast";
        assert_eq!(last_lines(raw, 6), ["ok", "loading 100%", "last"]);
        assert_eq!(last_lines(raw, 1), ["last"]);
    }

    #[test]
    fn clock_formats() {
        assert_eq!(format_elapsed(65_000), "01:05");
        assert_eq!(format_elapsed(3_725_000), "1:02:05");
        assert_eq!(format_since(30_000), "<1 min");
        assert_eq!(format_since(12 * 60_000), "12 min");
        assert_eq!(format_since(125 * 60_000), "2 h 05");
        assert!(over_limit(5.0, 5.0) && !over_limit(5.0, 0.0));
    }
}
