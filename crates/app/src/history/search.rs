//! Terminal history dialog: debounced search over every saved run, results grouped by run,
//! and a viewer around the clicked line (original `components/History/HistoryDialog.tsx`).

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use agentdeck_core::history::{RunMeta, RunWindow, SearchHit, history_read, history_search};
use agentdeck_core::logic::deck::folder_name;
use agentdeck_core::logic::text::agent_name;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{agent_kind, agent_mark, icon_button, modal};
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

const LIMIT: usize = 200;
const DEBOUNCE: Duration = Duration::from_millis(250);
const LINE_HEIGHT: f32 = 20.0;

/// Byte ranges of the case-insensitive (ASCII) occurrences of `query` in `text`, left to right.
pub fn highlight(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    // ASCII folding keeps byte offsets identical between `text` and its lowered copy.
    let lower = text.to_ascii_lowercase();
    let q = query.to_ascii_lowercase();
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find(&q) {
        let start = from + at;
        ranges.push(start..start + q.len());
        from = start + q.len();
    }
    ranges
}

/// `text` with the matches of `query` marked (wait color at 30%, fg text).
fn highlighted(text: &str, query: &str, cx: &App) -> StyledText {
    let p = palette(cx);
    let style = HighlightStyle {
        color: Some(hsla(p.fg)),
        background_color: Some(hsla_a(p.wait, 0.3)),
        ..Default::default()
    };
    let ranges = highlight(text, query).into_iter().map(|r| (r, style));
    StyledText::new(SharedString::from(text.to_owned())).with_highlights(ranges)
}

/// "10/26, 14:05" in local time.
fn when(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%m/%d, %H:%M").to_string())
        .unwrap_or_default()
}

fn run_header(meta: Option<&RunMeta>, run_id: &str, cx: &App) -> Div {
    let p = palette(cx);
    let kind = meta.and_then(|m| agent_kind(&m.agent));
    let title = match (meta.filter(|m| !m.title.is_empty()), kind) {
        (Some(m), _) => m.title.clone(),
        (None, Some(k)) => agent_name(k).to_owned(),
        (None, None) => run_id.to_owned(),
    };
    div()
        .flex()
        .min_w_0()
        .items_center()
        .gap_2()
        .children(kind.map(|k| agent_mark(k, 14.0, cx)))
        .child(div().truncate().font_weight(FontWeight::MEDIUM).text_color(hsla(p.fg)).child(title))
        .children(meta.filter(|m| !m.cwd.is_empty()).map(|m| {
            div()
                .truncate()
                .font_family(agentdeck_core::logic::theme::FONT_MONO)
                .text_xs()
                .text_color(hsla(p.subtle))
                .child(folder_name(&m.cwd))
        }))
        .children(meta.map(|m| {
            div()
                .ml_auto()
                .flex_shrink_0()
                .text_xs()
                .text_color(hsla(p.subtle))
                .child(when(m.started_at))
        }))
}

/// Runs in first-hit order, each with its hits.
fn group_by_run(hits: &[SearchHit]) -> Vec<(&str, Vec<&SearchHit>)> {
    let mut groups: Vec<(&str, Vec<&SearchHit>)> = Vec::new();
    for hit in hits {
        match groups.iter_mut().find(|(id, _)| *id == hit.run_id) {
            Some((_, group)) => group.push(hit),
            None => groups.push((&hit.run_id, vec![hit])),
        }
    }
    groups
}

struct Viewer {
    run_id: String,
    line: usize,
    around: usize,
    window: Option<Rc<RunWindow>>,
    error: Option<String>,
    scroll: UniformListScrollHandle,
    _load: Task<()>,
}

pub struct HistoryDialog {
    deck: Entity<Deck>,
    input: Entity<InputState>,
    query: String,
    hits: Option<Vec<SearchHit>>,
    error: Option<String>,
    viewer: Option<Viewer>,
    focus: FocusHandle,
    _search: Task<()>,
    _input_events: Subscription,
}

impl HistoryDialog {
    pub fn new(deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search the output of every terminal…"));
        input.update(cx, |state, cx| state.focus(window, cx));
        let input_events = cx.subscribe(&input, |this: &mut Self, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().trim().to_owned();
                this.search(query, cx);
            }
        });
        Self {
            deck,
            input,
            query: String::new(),
            hits: None,
            error: None,
            viewer: None,
            focus: cx.focus_handle(),
            _search: Task::ready(()),
            _input_events: input_events,
        }
    }

    /// Debounced search; a newer query drops the pending one.
    fn search(&mut self, query: String, cx: &mut Context<Self>) {
        if query == self.query {
            return;
        }
        self.query = query.clone();
        if query.is_empty() {
            self.hits = None;
            self._search = Task::ready(());
            cx.notify();
            return;
        }
        self._search = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let hits = cx.background_spawn(async move { history_search(query, LIMIT) }).await;
            let _ = this.update(cx, |this, cx| {
                this.hits = Some(hits);
                this.error = None;
                cx.notify();
            });
        });
        cx.notify();
    }

    fn open(&mut self, run_id: String, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.viewer = Some(Viewer {
            run_id,
            line,
            around: line,
            window: None,
            error: None,
            scroll: UniformListScrollHandle::new(),
            _load: Task::ready(()),
        });
        self.load(line, cx);
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.viewer = None;
        self.input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Loads the window of lines around `around`, then scrolls the clicked line to the middle.
    fn load(&mut self, around: usize, cx: &mut Context<Self>) {
        let Some(viewer) = self.viewer.as_mut() else { return };
        viewer.around = around;
        let run_id = viewer.run_id.clone();
        viewer._load = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { history_read(run_id, around) }).await;
            let _ = this.update(cx, |this, cx| {
                let Some(viewer) = this.viewer.as_mut() else { return };
                match result {
                    Ok(window) => {
                        if let Some(ix) = viewer.line.checked_sub(window.start_line) {
                            viewer.scroll.scroll_to_item(ix, ScrollStrategy::Center);
                        }
                        viewer.window = Some(Rc::new(window));
                    }
                    Err(e) => viewer.error = Some(e),
                }
                cx.notify();
            });
        });
    }

    fn render_results(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let mut body = div().flex().flex_col().gap_4().child(
            Input::new(&self.input).prefix(Icon::from(IconName::Search).text_color(hsla(p.subtle))),
        );
        if let Some(error) = &self.error {
            body = body.child(error_line(error.clone(), cx));
        }
        let Some(hits) = &self.hits else {
            return body.child(empty_line("Terminal output is saved as plain text and kept for 30 days.", cx));
        };
        if hits.is_empty() {
            return body.child(empty_line("No results", cx));
        }
        let count = if hits.len() >= LIMIT {
            format!("{LIMIT}+ results (most recent)")
        } else {
            format!("{} result(s)", hits.len())
        };
        body = body.child(div().text_xs().text_color(hsla(p.subtle)).child(count));
        for (run_id, group) in group_by_run(hits) {
            let rows = group.into_iter().map(|hit| {
                let (run, line) = (hit.run_id.clone(), hit.line);
                div()
                    .id(SharedString::from(format!("hit-{run}-{line}")))
                    .flex()
                    .w_full()
                    .gap_3()
                    .rounded_md()
                    .px_2()
                    .py_1()
                    .font_family(agentdeck_core::logic::theme::FONT_MONO)
                    .text_xs()
                    .line_height(ui(LINE_HEIGHT))
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(p.hover)))
                    .on_click(cx.listener(move |this, _, window, cx| this.open(run.clone(), line, window, cx)))
                    .child(line_number(hit.line, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .children(hit.before.clone().map(|t| div().truncate().text_color(hsla(p.subtle)).child(t)))
                            .child(
                                div()
                                    .truncate()
                                    .text_color(hsla(p.muted))
                                    .child(highlighted(&hit.text, &self.query, cx)),
                            )
                            .children(hit.after.clone().map(|t| div().truncate().text_color(hsla(p.subtle)).child(t))),
                    )
            });
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(ui(6.0))
                    .child(run_header(group_meta(hits, run_id), run_id, cx))
                    .child(div().flex().flex_col().children(rows)),
            );
        }
        body
    }

    fn render_viewer(&self, viewer: &Viewer, window: &Window, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let mut body = div().flex().flex_col().gap_3().child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(icon_button("history-back", IconName::ArrowLeft, "Back to results", {
                    let this = cx.entity().downgrade();
                    move |_, window, cx| {
                        let _ = this.update(cx, |this, cx| this.back(window, cx));
                    }
                }))
                .child(div().min_w_0().flex_1().children(
                    viewer.window.as_ref().map(|w| run_header(w.meta.as_ref(), &viewer.run_id, cx)),
                )),
        );
        if let Some(error) = &viewer.error {
            body = body.child(div().text_xs().text_color(hsla(p.bad)).child(error.clone()));
        }
        let Some(win) = viewer.window.clone() else { return body };
        if win.start_line > 0 {
            let to = win.start_line.saturating_sub(1000);
            body = body.child(page_button("history-prev", IconName::ChevronUp, "Previous lines", to, cx));
        }
        let (query, target, fg_line) = (self.query.clone(), viewer.line, p);
        let rows = win.clone();
        // The window holds up to a few thousand lines: a uniform list renders only the visible ones.
        let list = uniform_list("history-lines", win.lines.len(), move |range, _, cx| {
            range
                .map(|i| {
                    let n = rows.start_line + i;
                    div()
                        .flex()
                        .gap_3()
                        .px_2()
                        .h(ui(LINE_HEIGHT))
                        .when(n == target, |d| d.bg(hsla(fg_line.raised)))
                        .child(line_number(n, cx))
                        .child(
                            div()
                                .whitespace_nowrap()
                                .text_color(hsla(fg_line.muted))
                                .child(highlighted(&rows.lines[i], &query, cx)),
                        )
                })
                .collect()
        })
        .track_scroll(&viewer.scroll)
        // Room left by the modal header, the run header and the paging buttons.
        .h((window.viewport_size().height * 0.86 - ui(200.0).to_pixels(window.rem_size())).max(ui(160.0).to_pixels(window.rem_size())));
        body = body.child(
            div()
                .rounded_md()
                .bg(hsla(p.base))
                .py_2()
                .font_family(agentdeck_core::logic::theme::FONT_MONO)
                .text_xs()
                .line_height(ui(LINE_HEIGHT))
                .overflow_hidden()
                .child(list),
        );
        let end = win.start_line + win.lines.len();
        if end < win.total_lines {
            body = body.child(page_button("history-next", IconName::ChevronDown, "Next lines", end + 1000, cx));
        }
        body
    }
}

fn group_meta<'a>(hits: &'a [SearchHit], run_id: &str) -> Option<&'a RunMeta> {
    hits.iter().find(|h| h.run_id == run_id).and_then(|h| h.meta.as_ref())
}

fn line_number(n: usize, cx: &App) -> Div {
    div()
        .w_12()
        .flex_shrink_0()
        .flex()
        .justify_end()
        .text_color(hsla(palette(cx).subtle))
        .child((n + 1).to_string())
}

fn empty_line(text: &'static str, cx: &App) -> Div {
    div().py_8().flex().justify_center().text_color(hsla(palette(cx).muted)).child(text)
}

fn error_line(text: String, cx: &App) -> Div {
    let p = palette(cx);
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(hsla(p.bad))
        .child(Icon::from(IconName::TriangleAlert).text_color(hsla(p.bad)))
        .child(text)
}

fn page_button(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    around: usize,
    cx: &mut Context<HistoryDialog>,
) -> Stateful<Div> {
    let p = palette(cx);
    div()
        .id(id)
        .self_center()
        .flex()
        .items_center()
        .gap(ui(6.0))
        .text_xs()
        .text_color(hsla(p.muted))
        .cursor_pointer()
        .hover(|s| s.text_color(hsla(p.fg)))
        .on_click(cx.listener(move |this, _, _, cx| this.load(around, cx)))
        .child(Icon::from(icon))
        .child(label)
}

impl Render for HistoryDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.deck;
        let body = match &self.viewer {
            Some(viewer) => self.render_viewer(viewer, window, cx),
            None => self.render_results(cx),
        };
        modal("Terminal history", 56.0, Vec::new(), &self.focus, body, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::highlight;

    #[test]
    fn highlights_every_case_insensitive_match() {
        assert_eq!(highlight("Error: error ERROR", "error"), vec![0..5, 7..12, 13..18]);
        assert_eq!(highlight("aaa", "aa"), vec![0..2]);
        assert!(highlight("text", "").is_empty());
        assert!(highlight("text", "zz").is_empty());
        // Non-ASCII text keeps valid byte ranges.
        assert_eq!(highlight("é Café", "caf"), vec![3..6]);
    }
}
