//! Voice targeting: "Codex, run the tests" goes to a Codex pane (prefix stripped) instead of the
//! active pane (`src/lib/voiceRoute.ts`).

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::agents::AgentKind;

#[derive(Debug, Clone)]
pub struct RouteCandidate {
    pub id: String,
    pub agent: AgentKind,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub pane_id: Option<String>,
    pub text: String,
}

/// Spoken names per agent, normalized. Includes common mis-transcriptions ("cloud" for Claude).
pub fn agent_aliases(kind: AgentKind) -> &'static [&'static str] {
    match kind {
        AgentKind::Claude => &["claude", "claude code", "claud", "clode", "clod", "cloud", "cloude"],
        AgentKind::Codex => &["codex", "codecs", "codec", "kodex", "codexe", "code x"],
        AgentKind::Gemini => &["gemini", "gemini cli", "gemeni", "jemini", "jiminy", "gemmini"],
        AgentKind::Grok => &["grok", "groc", "grock", "groque", "grok build"],
        AgentKind::OpenCode => &["opencode", "open code", "open-code", "op\u{e8}ne code"],
        AgentKind::Shell => &["shell", "chell", "terminal", "le shell", "le terminal"],
    }
}

/// Lowercase, accents removed.
fn norm(s: &str) -> String {
    s.nfd().filter(|c| !is_combining_mark(*c)).collect::<String>().to_lowercase()
}

/// Words (letters/digits runs) with their byte range in `text`.
fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                spans.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        spans.push((s, text.len()));
    }
    spans
}

fn words(s: &str) -> Vec<String> {
    word_spans(s).into_iter().map(|(a, b)| norm(&s[a..b])).collect()
}

/// Picks the pane a transcript is addressed to. `candidates` are in preference order (most recently
/// active first). A pane title beats an agent name of the same length; the longest spoken name wins.
/// Returns `pane_id: None` and the text untouched when nothing matches.
pub fn route_transcript(text: &str, candidates: &[RouteCandidate]) -> Route {
    let untouched = || Route {
        pane_id: None,
        text: text.to_owned(),
    };
    let mut tokens: Vec<(String, usize)> = Vec::new();
    for (start, end) in word_spans(text) {
        if tokens.len() == 4 {
            break;
        }
        let prev_end = tokens.last().map_or(0, |t| t.1);
        let gap = &text[prev_end..start];
        let allowed: &[char] = if tokens.is_empty() {
            &['"', '\'', '\u{ab}', '(']
        } else {
            &['\'', '-']
        };
        if !gap.chars().all(|c| c.is_whitespace() || allowed.contains(&c)) {
            break;
        }
        tokens.push((norm(&text[start..end]), end));
    }
    let entries = candidates
        .iter()
        .map(|c| (c.id.as_str(), words(&c.title)))
        .chain(candidates.iter().flat_map(|c| {
            agent_aliases(c.agent)
                .iter()
                .map(move |alias| (c.id.as_str(), alias.split(' ').map(str::to_owned).collect()))
        }));
    let mut best: Option<(&str, usize, usize)> = None; // (id, end, length)
    for (id, key) in entries {
        if key.is_empty() || key.len() > tokens.len() || best.is_some_and(|b| key.len() <= b.2) {
            continue;
        }
        if key.iter().zip(&tokens).all(|(w, t)| *w == t.0) {
            best = Some((id, tokens[key.len() - 1].1, key.len()));
        }
    }
    let Some((id, end, _)) = best else {
        return untouched();
    };
    let rest = text[end..].trim_start_matches(|c: char| {
        c.is_whitespace() || ",:;.!?\u{bb}\"')\u{2014}\u{2013}-".contains(c)
    });
    if rest.is_empty() {
        return untouched(); // "Claude." alone: nothing to send
    }
    Route {
        pane_id: Some(id.to_owned()),
        text: rest.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panes() -> Vec<RouteCandidate> {
        let c = |id: &str, agent, title: &str| RouteCandidate {
            id: id.into(),
            agent,
            title: title.into(),
        };
        vec![
            c("c1", AgentKind::Claude, "Front"),
            c("x1", AgentKind::Codex, "agentdeck"),
            c("c2", AgentKind::Claude, "Back end"),
            c("s1", AgentKind::Shell, "Shell"),
        ]
    }

    fn r(id: Option<&str>, text: &str) -> Route {
        Route {
            pane_id: id.map(str::to_owned),
            text: text.into(),
        }
    }

    #[test]
    fn routes_agent_prefix() {
        let p = panes();
        assert_eq!(route_transcript("Codex, run the tests.", &p), r(Some("x1"), "run the tests."));
        assert_eq!(route_transcript("Gemini: nothing", &p), r(None, "Gemini: nothing"));
        assert_eq!(route_transcript("Shell dir", &p), r(Some("s1"), "dir"));
    }

    #[test]
    fn tolerant() {
        let p = panes();
        assert_eq!(route_transcript("CLAUDE do a review", &p).pane_id.as_deref(), Some("c1"));
        assert_eq!(route_transcript("Cloud, do a review", &p), r(Some("c1"), "do a review"));
        assert_eq!(route_transcript("Codecs : fix the build", &p), r(Some("x1"), "fix the build"));
        assert_eq!(route_transcript("Cl\u{e0}ude Code, go", &p), r(Some("c1"), "go"));
    }

    #[test]
    fn preference_and_titles() {
        let p = panes();
        let reordered = vec![p[2].clone(), p[0].clone(), p[1].clone(), p[3].clone()];
        assert_eq!(route_transcript("Claude, go", &reordered).pane_id.as_deref(), Some("c2"));
        assert_eq!(route_transcript("Back end, add a route", &p), r(Some("c2"), "add a route"));
        assert_eq!(route_transcript("front: the button", &p), r(Some("c1"), "the button"));
    }

    #[test]
    fn no_match() {
        let p = panes();
        assert_eq!(route_transcript("Add a test", &p), r(None, "Add a test"));
        assert_eq!(route_transcript("Claude.", &p), r(None, "Claude."));
        assert_eq!(route_transcript("Codex", &[]), r(None, "Codex"));
        assert_eq!(route_transcript("Ask Claude", &p).pane_id, None);
        assert_eq!(route_transcript("Claudette is coming", &p).pane_id, None);
    }
}
