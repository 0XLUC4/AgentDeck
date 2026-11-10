//! Find in terminal (Ctrl+Shift+F): a small bar that searches the scrollback of the pane with
//! alacritty's regex search. Nothing here runs while the bar is closed.

use std::sync::Once;

use alacritty_terminal::Term;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Line, Point as AlacPoint};
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::*;

use super::TerminalView;
use crate::theme::{hsla, palette, ui};

actions!(agentdeck, [FindInTerminal]);

/// Matches counted before the label gives up ("999+").
/// shortcut: Enter only walks the newest `COUNT_CAP + 1` matches, raise it if users ask.
const COUNT_CAP: usize = 999;

/// Binds Ctrl+Shift+F for focused terminals. The "Terminal" context wins over the global binding
/// of the same keys (history search), which still applies everywhere else.
pub(super) fn bind_keys(cx: &mut App) {
    static BOUND: Once = Once::new();
    BOUND.call_once(|| cx.bind_keys([KeyBinding::new("ctrl-shift-f", FindInTerminal, Some("Terminal"))]));
}

/// Regex matching `query` literally, whatever its case.
fn literal_regex(query: &str) -> String {
    let mut regex = String::from("(?i)");
    for c in query.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            regex.push('\\');
        }
        regex.push(c);
    }
    regex
}

/// Index of the match to move to. `starts` are the match starts, newest first. `from` is the
/// start of the current match, which may be gone (new output): the step is then taken from
/// where it was. Both directions wrap.
fn step_from<P: Ord>(starts: &[P], from: Option<&P>, older: bool) -> Option<usize> {
    let len = starts.len();
    if len == 0 {
        return None;
    }
    let Some(from) = from else { return Some(0) };
    // First match at or above the old position.
    let at = starts.iter().position(|start| start <= from).unwrap_or(len);
    Some(match older {
        true if at < len && starts[at] == *from => (at + 1) % len,
        true => at % len,
        false => (at + len - 1) % len,
    })
}

/// "3/17", "3/999+" past the cap, "0/0" without a match.
fn count_label(index: Option<usize>, count: usize, cap: usize) -> String {
    let total = if count > cap { format!("{cap}+") } else { count.to_string() };
    format!("{}/{total}", index.map_or(0, |i| i + 1))
}

/// Up to `cap + 1` matches of the whole grid (scrollback included), newest first.
fn find_all<T>(term: &Term<T>, regex: &mut RegexSearch, cap: usize) -> Vec<Match> {
    let newest = AlacPoint::new(term.bottommost_line(), term.last_column());
    let oldest = AlacPoint::new(term.topmost_line(), Column(0));
    RegexIter::new(newest, oldest, Direction::Left, term, regex).take(cap + 1).collect()
}

pub(super) struct Search {
    input: Entity<InputState>,
    /// None for an empty query.
    regex: Option<RegexSearch>,
    current: Option<Match>,
    label: SharedString,
    _events: Subscription,
}

impl Search {
    /// New output pushed `lines` lines into the scrollback: the current match moved up with them.
    /// shortcut: a full scrollback drops lines without growing, the current match then loses its
    /// strong highlight until the next Enter (which steps from where it was).
    pub(super) fn scrolled(&mut self, lines: usize) {
        if lines == 0 {
            return;
        }
        let up = |p: &AlacPoint| AlacPoint::new(Line(p.line.0 - lines as i32), p.column);
        self.current = self.current.take().map(|m| up(m.start())..=up(m.end()));
    }
}

impl TerminalView {
    /// Opens the search bar (or focuses it again).
    pub(super) fn open_search(&mut self, _: &FindInTerminal, window: &mut Window, cx: &mut Context<Self>) {
        let input = match &self.search {
            Some(search) => search.input.clone(),
            None => {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
                let events = cx.subscribe(&input, |view: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => view.search_changed(cx),
                    // Enter goes to the next (older) match, Shift+Enter back.
                    InputEvent::PressEnter { shift, .. } => view.search_step(!*shift, cx),
                    _ => {}
                });
                let label = count_label(None, 0, COUNT_CAP).into();
                self.search = Some(Search { input: input.clone(), regex: None, current: None, label, _events: events });
                input
            }
        };
        input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Esc: the bar and its highlights go away, the keyboard goes back to the terminal.
    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search = None;
        window.focus(&self.focus_handle(cx), cx);
        cx.notify();
    }

    fn search_changed(&mut self, cx: &mut Context<Self>) {
        let Some(search) = self.search.as_mut() else { return };
        let query = search.input.read(cx).value();
        // A pattern over the automaton size limit finds nothing, like an empty one.
        search.regex = (!query.is_empty()).then(|| RegexSearch::new(&literal_regex(&query)).ok()).flatten();
        search.current = None;
        self.search_step(true, cx);
    }

    /// Searches the grid again (the output may have changed), moves to the next match and
    /// scrolls it into view.
    fn search_step(&mut self, older: bool, cx: &mut Context<Self>) {
        let Some(search) = self.search.as_mut() else { return };
        let matches = match search.regex.as_mut() {
            Some(regex) => find_all(&self.term, regex, COUNT_CAP),
            None => Vec::new(),
        };
        let starts: Vec<AlacPoint> = matches.iter().map(|m| *m.start()).collect();
        let index = step_from(&starts, search.current.as_ref().map(|m| m.start()), older);
        search.current = index.map(|i| matches[i].clone());
        search.label = count_label(index, matches.len(), COUNT_CAP).into();
        if let Some(start) = index.map(|i| starts[i]) {
            self.term.scroll_to_point(start);
        }
        cx.notify();
    }

    /// Matches on screen and the current one, for the highlights. Empty while the bar is closed.
    pub(super) fn visible_matches(&mut self) -> (Vec<Match>, Option<Match>) {
        let Some(search) = self.search.as_mut() else { return Default::default() };
        let Some(regex) = search.regex.as_mut() else { return Default::default() };
        let term = &self.term;
        let offset = term.grid().display_offset() as i32;
        let top = AlacPoint::new(Line(-offset), Column(0));
        let bottom = AlacPoint::new(Line(term.screen_lines() as i32 - 1 - offset), term.last_column());
        // Same direction as `find_all`, so the current match is found again as is.
        (RegexIter::new(bottom, top, Direction::Left, term, regex).collect(), search.current.clone())
    }

    /// The bar at the top right of the terminal.
    pub(super) fn render_search(&self, cx: &mut Context<Self>) -> Option<Div> {
        let search = self.search.as_ref()?;
        let p = palette(cx);
        let bar = div()
            .absolute()
            .top(ui(6.0))
            .right(ui(14.0))
            .w(ui(260.0))
            .flex()
            .items_center()
            .gap_2()
            .pr_2()
            .rounded_md()
            .border_1()
            .border_color(hsla(p.line_strong))
            .bg(hsla(p.raised))
            .shadow_md()
            .cursor(CursorStyle::Arrow)
            // Clicks on the bar neither select terminal text nor reach the program.
            .occlude()
            .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
            .on_action(cx.listener(|view, _: &Escape, window, cx| view.close_search(window, cx)))
            .child(div().flex_1().min_w_0().child(Input::new(&search.input).xsmall().appearance(false)))
            .child(div().flex_shrink_0().text_xs().text_color(hsla(p.subtle)).child(search.label.clone()));
        Some(bar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    use alacritty_terminal::term::Config;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::Processor;

    #[test]
    fn escapes_regex_metacharacters() {
        assert_eq!(literal_regex("a.b"), "(?i)a\\.b");
        assert_eq!(literal_regex("f(x)*[1]|$"), "(?i)f\\(x\\)\\*\\[1\\]\\|\\$");
        assert_eq!(literal_regex("C:\\dir"), "(?i)C:\\\\dir");
        assert_eq!(literal_regex("plain text"), "(?i)plain text");
    }

    #[test]
    fn steps_through_matches() {
        // Starts, newest first.
        let starts = [30, 20, 10];
        assert_eq!(step_from(&starts, None, true), Some(0));
        assert_eq!(step_from(&starts, Some(&30), true), Some(1));
        assert_eq!(step_from(&starts, Some(&10), true), Some(0));
        assert_eq!(step_from(&starts, Some(&30), false), Some(2));
        assert_eq!(step_from(&starts, Some(&20), false), Some(0));
        // The current match is gone: step from where it was.
        assert_eq!(step_from(&starts, Some(&25), true), Some(1));
        assert_eq!(step_from(&starts, Some(&25), false), Some(0));
        assert_eq!(step_from(&starts, Some(&5), true), Some(0));
        assert_eq!(step_from(&starts, Some(&5), false), Some(2));
        assert_eq!(step_from::<i32>(&[], Some(&5), true), None);
    }

    #[test]
    fn labels() {
        assert_eq!(count_label(None, 0, 999), "0/0");
        assert_eq!(count_label(Some(2), 17, 999), "3/17");
        assert_eq!(count_label(Some(0), 1000, 999), "1/999+");
    }

    #[test]
    fn finds_literal_matches_in_the_scrollback() {
        let mut term = Term::new(Config::default(), &TermSize::new(20, 2), super::super::Listener::default());
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"Foo.bar one\r\nfooxbar\r\nx foo.BAR\r\nlast\r\n");
        assert!(term.history_size() > 0);
        let find = |query: &str, cap: usize| {
            let mut regex = RegexSearch::new(&literal_regex(query)).expect("valid regex");
            find_all(&term, &mut regex, cap)
        };
        // Case-insensitive even with capitals in the query, "." is literal, newest first.
        let matches = find("FOO.b", 999);
        assert_eq!(matches.len(), 2);
        assert!(matches[0].start() > matches[1].start());
        assert_eq!((matches[0].start().column.0, matches[0].end().column.0), (2, 6));
        assert_eq!(matches[1].start().column.0, 0);
        assert_eq!(find("o", 2).len(), 3);
        assert!(find("nothing", 999).is_empty());
    }
}
