//! Fuzzy matching for the command palette (components/CommandPalette/fuzzy.ts).

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Lowercase, accents stripped (NFD minus combining marks).
fn normalize(text: &str) -> Vec<char> {
    text.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_word_start(c: char) -> bool {
    c.is_whitespace() || matches!(c, '›' | '/' | '\\' | '_' | '.' | ':' | '-')
}

/// Subsequence fuzzy match, case- and accent-insensitive; spaces in the query are ignored.
/// Higher is better; `None` when the text lacks a query character.
/// Bonuses: consecutive chars, word starts, substring.
pub fn fuzzy_score(query: &str, text: &str) -> Option<f64> {
    let q: Vec<char> = normalize(query).into_iter().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0.0);
    }
    let t = normalize(text);
    let mut score = 0.0;
    let mut from = 0;
    let mut prev: Option<usize> = None;
    for &ch in &q {
        let at = from + t[from..].iter().position(|&c| c == ch)?;
        score += 1.0;
        if prev.is_some_and(|p| at == p + 1) {
            score += 3.0;
        }
        if at == 0 || is_word_start(t[at - 1]) {
            score += 2.0;
        }
        prev = Some(at);
        from = at + 1;
    }
    if t.windows(q.len()).any(|w| w == q.as_slice()) {
        score += q.len() as f64 * 2.0;
    }
    Some(score - t.len() as f64 * 0.01) // shorter text wins ties
}

#[cfg(test)]
mod tests {
    use super::fuzzy_score;

    #[test]
    fn matches_subsequences_ignoring_case_accents_and_spaces() {
        assert!(fuzzy_score("SETT", "Open settings").is_some());
        assert!(fuzzy_score("resume", "Résumé").is_some());
        assert!(fuzzy_score("stop a", "Stop all").is_some());
        assert_eq!(fuzzy_score("", "anything"), Some(0.0));
    }

    #[test]
    fn rejects_missing_characters_and_wrong_order() {
        assert!(fuzzy_score("xyz", "Stop all").is_none());
        assert!(fuzzy_score("ba", "ab").is_none());
    }

    #[test]
    fn ranks_substring_and_word_start_above_scattered() {
        let sub = fuzzy_score("wall", "Wall mode").unwrap_or(-1.0);
        let scattered = fuzzy_score("wall", "Window with all lines").unwrap_or(-1.0);
        assert!(sub > scattered);
        let word_start = fuzzy_score("ns", "New session").unwrap_or(-1.0);
        let middle = fuzzy_score("ns", "Maximize panes").unwrap_or(-1.0);
        assert!(word_start > middle);
    }

    #[test]
    fn prefers_shorter_text_on_equal_matches() {
        let short = fuzzy_score("claude", "Claude").unwrap_or(-1.0);
        let long = fuzzy_score("claude", "Claude Code session").unwrap_or(-1.0);
        assert!(short > long);
    }
}
