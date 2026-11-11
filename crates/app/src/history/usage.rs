//! Usage dialog (today / 7 days / 30 days: total, by day, by agent, by workspace) and the pane
//! header usage chip. Original: `components/Usage/{UsageDialog,UsageChip,format}.tsx`.

use std::time::Duration;

use agentdeck_core::agents::AgentKind;
use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::{Workspace, folder_name};
use agentdeck_core::logic::text::{agent_name, format_cost, format_tokens};
use agentdeck_core::usage::{Keyed, PaneUsage, Totals, UsageSummary, pane_usage, usage_summary};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{agent_kind, agent_mark, modal, segmented, text_tooltip};
use crate::state::Deck;
use crate::theme::{hsla, palette, ui};

const REFRESH: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Range {
    Today,
    Week,
    Month,
}

const RANGES: [(Range, &str); 3] = [(Range::Today, "Today"), (Range::Week, "7 days"), (Range::Month, "30 days")];

impl Range {
    fn id(self) -> &'static str {
        match self {
            Range::Today => "today",
            Range::Week => "7d",
            Range::Month => "30d",
        }
    }
}

fn tokens(n: u64) -> String {
    format_tokens(n as f64)
}

/// How full the conversation is: (percent, window size).
// shortcut: the transcript does not name the window, so it is 200k, or 1M once past that;
// read it from the model id if a third size appears.
pub fn context_fill(context: u64) -> Option<(u64, u64)> {
    let window = if context > 200_000 { 1_000_000 } else { 200_000 };
    (context > 0).then(|| (context * 100 / window, window))
}

/// Chip text: tokens, plus cost when some of it is priced.
pub fn usage_label(u: &Totals) -> String {
    if u.cost > 0.0 {
        format!("{} tok · {}", tokens(u.tokens), format_cost(u.cost))
    } else {
        format!("{} tok", tokens(u.tokens))
    }
}

/// Multi-line tooltip with the breakdown.
pub fn usage_details(u: &Totals) -> String {
    let mut lines = vec![
        format!("Input: {}", tokens(u.input)),
        format!("Output: {}", tokens(u.output)),
        format!("Cache write: {}", tokens(u.cache_write)),
        format!("Cache read: {} (not in the token total, priced at the cache rate)", tokens(u.cache_read)),
        format!("Estimated cost (API pricing): {}", format_cost(u.cost)),
    ];
    if u.unpriced_tokens > 0 {
        lines.push(format!("{} tokens from a model with no known price", tokens(u.unpriced_tokens)));
    }
    lines.join("\n")
}

fn norm(path: &str) -> String {
    path.trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase()
}

fn add(acc: &mut Totals, row: &Totals) {
    acc.input += row.input;
    acc.output += row.output;
    acc.cache_write += row.cache_write;
    acc.cache_read += row.cache_read;
    acc.tokens += row.tokens;
    acc.cost += row.cost;
    acc.unpriced_tokens += row.unpriced_tokens;
}

/// Folds per-cwd rows into linked workspaces (cwd inside the workspace folder), else the folder name.
pub fn by_workspace(rows: &[Keyed], workspaces: &[Workspace]) -> Vec<Keyed> {
    let mut merged: Vec<Keyed> = Vec::new();
    for row in rows {
        let cwd = norm(&row.key);
        let ws = workspaces.iter().find(|w| {
            let root = norm(&w.path);
            !w.path.is_empty() && (cwd == root || cwd.starts_with(&format!("{root}\\")))
        });
        let key = match ws {
            Some(w) => w.name.clone(),
            None => Some(folder_name(&row.key)).filter(|n| !n.is_empty()).unwrap_or_else(|| "Unknown".into()),
        };
        match merged.iter_mut().find(|m| m.key == key) {
            Some(acc) => add(&mut acc.totals, &row.totals),
            None => merged.push(Keyed { key, totals: row.totals.clone() }),
        }
    }
    merged.sort_by(|a, b| {
        b.totals.cost.total_cmp(&a.totals.cost).then(b.totals.tokens.cmp(&a.totals.tokens))
    });
    merged
}

/// Bar length: cost when anything is priced, else tokens.
fn measure(rows: &[&Totals]) -> impl Fn(&Totals) -> f64 + use<> {
    let priced = rows.iter().any(|r| r.cost > 0.0);
    move |r: &Totals| if priced { r.cost } else { r.tokens as f64 }
}

fn figure(totals: &Totals, id: impl Into<ElementId>, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .gap_1()
        .font_family(agentdeck_core::logic::theme::FONT_MONO)
        .text_xs()
        .text_color(hsla(p.muted))
        .tooltip(text_tooltip(usage_details(totals)))
        .child(format!("{} tok ·", tokens(totals.tokens)))
        .child(div().text_color(hsla(p.fg)).child(format_cost(totals.cost)))
}

fn bar_list(title: &'static str, rows: &[Keyed], with_icon: bool, cx: &App) -> Option<Div> {
    if rows.is_empty() {
        return None;
    }
    let p = palette(cx);
    let value = measure(&rows.iter().map(|r| &r.totals).collect::<Vec<_>>());
    let max = rows.iter().map(|r| value(&r.totals)).fold(1e-9, f64::max);
    let items = rows.iter().take(10).enumerate().map(|(i, row)| {
        let kind = agent_kind(&row.key);
        let label = kind.map_or_else(|| row.key.clone(), |k| agent_name(k).to_owned());
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .children(kind.filter(|_| with_icon).map(|k: AgentKind| agent_mark(k, 14.0, cx)))
                    .child(
                        div()
                            .id((title, i))
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_color(hsla(p.fg))
                            .tooltip(text_tooltip(row.key.clone()))
                            .child(label),
                    )
                    .child(figure(&row.totals, (SharedString::from(format!("{title}-fig")), i), cx)),
            )
            .child(
                div().h_1().rounded_full().bg(hsla(p.base)).child(
                    div().h_1().rounded_full().bg(hsla(p.muted)).w(relative((value(&row.totals) / max) as f32)),
                ),
            )
    });
    Some(
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(hsla(p.subtle)).child(title))
            .child(div().flex().flex_col().gap_2().children(items)),
    )
}

/// "2026-10-26" -> "10/26".
fn day_label(key: &str) -> String {
    let mut parts = key.splitn(3, '-').skip(1);
    match (parts.next(), parts.next()) {
        (Some(m), Some(d)) => format!("{m}/{d}"),
        _ => key.to_owned(),
    }
}

fn day_chart(days: &[Keyed], cx: &App) -> Div {
    let p = palette(cx);
    let value = measure(&days.iter().map(|d| &d.totals).collect::<Vec<_>>());
    let max = days.iter().map(|d| value(&d.totals)).fold(1e-9, f64::max);
    let bars = days.iter().enumerate().map(|(i, day)| {
        let tip = format!("{}\n{}", day_label(&day.key), usage_details(&day.totals));
        div()
            .id(("usage-day", i))
            .group("usage-day")
            .flex()
            .flex_col()
            .justify_end()
            .h_full()
            .min_w_0()
            .flex_1()
            .tooltip(text_tooltip(tip))
            .child(
                div()
                    .min_h(px(1.0))
                    .rounded_t_sm()
                    .bg(hsla(p.muted))
                    .hover(|s| s.bg(hsla(p.fg)))
                    .h(relative((value(&day.totals) / max) as f32)),
            )
    });
    let first = days.first().map(|d| day_label(&d.key)).unwrap_or_default();
    let last = days.last().map(|d| day_label(&d.key)).unwrap_or_default();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(hsla(p.subtle)).child("By day"))
        .child(div().flex().h_32().items_end().gap_1().children(bars))
        .child(
            div()
                .flex()
                .justify_between()
                .font_family(agentdeck_core::logic::theme::FONT_MONO)
                .text_size(ui(11.0))
                .text_color(hsla(p.subtle))
                .child(first)
                .child(last),
        )
}

pub struct UsageDialog {
    deck: Entity<Deck>,
    range: Range,
    data: Option<UsageSummary>,
    error: Option<String>,
    focus: FocusHandle,
    _load: Task<()>,
}

impl UsageDialog {
    pub fn new(deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut this = Self { deck, range: Range::Today, data: None, error: None, focus, _load: Task::ready(()) };
        this.load(Range::Today, cx);
        this
    }

    fn load(&mut self, range: Range, cx: &mut Context<Self>) {
        self.range = range;
        self.data = None;
        self.error = None;
        self._load = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { usage_summary(range.id().into()) }).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(data) => this.data = Some(data),
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn render_body(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let this = cx.entity().downgrade();
        let picker = segmented(&RANGES, self.range, move |range, cx| {
            let _ = this.update(cx, |this, cx| this.load(range, cx));
        }, cx);
        let body = div().flex().flex_col().gap_5().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(picker)
                .child(div().text_xs().text_color(hsla(p.subtle)).child("Claude Code and Codex · cost estimated at API pricing")),
        );
        if let Some(error) = &self.error {
            return body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(hsla(p.bad))
                    .child(Icon::from(IconName::TriangleAlert))
                    .child(format!("Could not read usage: {error}")),
            );
        }
        let Some(data) = &self.data else {
            return body.child(div().py_8().flex().justify_center().text_color(hsla(p.muted)).child("Reading transcripts…"));
        };
        let total = &data.total;
        let body = body
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_3()
                    .child(div().text_2xl().font_weight(FontWeight::SEMIBOLD).child(format_cost(total.cost)))
                    .child(
                        div()
                            .id("usage-total")
                            .font_family(agentdeck_core::logic::theme::FONT_MONO)
                            .text_xs()
                            .text_color(hsla(p.muted))
                            .tooltip(text_tooltip(usage_details(total)))
                            .child(format!("{} tokens · {} output", tokens(total.tokens), tokens(total.output))),
                    ),
            )
            .when(total.unpriced_tokens > 0, |d| {
                d.child(div().text_xs().text_color(hsla(p.subtle)).child(format!(
                    "{} tokens from models with no known price are not included in the cost.",
                    tokens(total.unpriced_tokens)
                )))
            });
        if total.tokens == 0 {
            return body.child(div().py_6().flex().justify_center().text_color(hsla(p.muted)).child("No usage in this period"));
        }
        let workspaces = by_workspace(&data.by_workspace_cwd, &self.deck.read(cx).state.workspaces);
        body.when(data.by_day.len() > 1, |d| d.child(day_chart(&data.by_day, cx)))
            .children(bar_list("By agent", &data.by_agent, true, cx))
            .children(bar_list("By project", &workspaces, false, cx))
    }
}

impl Render for UsageDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.render_body(cx);
        modal("Usage", 44.0, Vec::new(), &self.focus, body, window, cx)
    }
}

/// What makes the chip reload: a turn starting or ending, a restart, the hook connecting, exit.
#[derive(Clone, PartialEq)]
struct ChipKey {
    cwd: String,
    agent: AgentKind,
    generation: u32,
    hooked: bool,
    exited: bool,
    working: bool,
}

/// Tokens and cost of the pane's current process (Claude Code and Codex transcripts only).
/// Reloads when a turn starts or ends and every 30 s while a hooked agent runs.
pub struct UsageChip {
    deck: Entity<Deck>,
    pane_id: String,
    key: Option<ChipKey>,
    usage: Option<PaneUsage>,
    _load: Task<()>,
    _refresh: Task<()>,
    _watch: Subscription,
}

impl UsageChip {
    pub fn new(deck: Entity<Deck>, pane_id: String, cx: &mut Context<Self>) -> Self {
        let watch = cx.observe(&deck, |this, _, cx| this.sync(cx));
        let mut this = Self {
            deck,
            pane_id,
            key: None,
            usage: None,
            _load: Task::ready(()),
            _refresh: Task::ready(()),
            _watch: watch,
        };
        this.sync(cx);
        this
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let key = self.deck.read(cx).state.panes.get(&self.pane_id).map(|p| ChipKey {
            cwd: p.cwd.clone(),
            agent: p.agent,
            generation: p.generation,
            hooked: p.hooked,
            exited: p.exited,
            working: p.status == PaneStatus::Working,
        });
        if key == self.key {
            return;
        }
        self.key = key.clone();
        let Some(key) = key.filter(|k| matches!(k.agent, AgentKind::Claude | AgentKind::Codex)) else {
            self._refresh = Task::ready(());
            return;
        };
        self.load(cx);
        self._refresh = if key.hooked && !key.exited {
            cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(REFRESH).await;
                if this.update(cx, |this, cx| this.load(cx)).is_err() {
                    break;
                }
            })
        } else {
            Task::ready(())
        };
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(pane) = self.deck.read(cx).state.panes.get(&self.pane_id).cloned() else { return };
        let Some(started_at) = crate::panes::runtime::pane_started_at(&pane.id, cx) else { return };
        let (id, cwd, agent) = (pane.id.clone(), pane.cwd.clone(), agent_id(pane.agent));
        self._load = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { pane_usage(id, cwd, started_at, agent) }).await;
            match result {
                Ok(usage) => {
                    let _ = this.update(cx, |this, cx| {
                        this.usage = Some(usage);
                        cx.notify();
                    });
                }
                Err(e) => eprintln!("pane_usage failed: {e}"),
            }
        });
    }
}

/// The lowercase agent id the backend expects ("claude", "codex").
pub(super) fn agent_id(kind: AgentKind) -> String {
    serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

impl Render for UsageChip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tracked = self.key.as_ref().is_some_and(|k| matches!(k.agent, AgentKind::Claude | AgentKind::Codex));
        let Some(usage) = self.usage.as_ref().filter(|u| tracked && u.totals.tokens > 0) else {
            return div().id("usage-chip");
        };
        let models = if usage.models.is_empty() { String::new() } else { format!("\nModels: {}", usage.models.join(", ")) };
        let p = palette(cx);
        let fill = context_fill(usage.context);
        let room = fill.map_or_else(String::new, |(percent, window)| {
            format!("\nConversation: {} of {} tokens ({percent}%)", tokens(usage.context), tokens(window))
        });
        let tip = format!("Since the agent started\n{}{models}{room}", usage_details(&usage.totals));
        let label = match fill {
            Some((percent, _)) => format!("{} · ctx {percent}%", usage_label(&usage.totals)),
            None => usage_label(&usage.totals),
        };
        // Nearly full: the conversation is about to be compacted.
        let color = if fill.is_some_and(|(percent, _)| percent >= 80) { p.wait } else { p.subtle };
        div()
            .id("usage-chip")
            .flex_shrink_0()
            .font_family(agentdeck_core::logic::theme::FONT_MONO)
            .text_xs()
            .text_color(hsla(color))
            .tooltip(text_tooltip(tip))
            .child(label)
    }
}

#[cfg(test)]
mod tests {
    use super::{agent_id, by_workspace, context_fill, day_label, usage_details, usage_label};
    use agentdeck_core::agents::AgentKind;
    use agentdeck_core::logic::deck::Workspace;
    use agentdeck_core::usage::{Keyed, Totals};

    fn row(key: &str, tokens: u64, cost: f64) -> Keyed {
        Keyed { key: key.into(), totals: Totals { tokens, cost, ..Totals::default() } }
    }

    fn ws(name: &str, path: &str) -> Workspace {
        Workspace { id: name.into(), name: name.into(), path: path.into(), git: None }
    }

    #[test]
    fn folds_rows_into_workspaces() {
        let rows = [row("D:\\proj", 10, 1.0), row("d:/proj/sub/", 5, 0.5), row("C:\\tmp\\x", 100, 0.0), row("D:\\project", 1, 3.0)];
        let merged = by_workspace(&rows, &[ws("Proj", "D:\\proj"), ws("Default", "")]);
        let keys: Vec<_> = merged.iter().map(|m| (m.key.as_str(), m.totals.tokens)).collect();
        assert_eq!(keys, [("project", 1), ("Proj", 15), ("x", 100)]);
    }

    #[test]
    fn labels_and_details() {
        let priced = Totals { tokens: 12_345, cost: 0.31, ..Totals::default() };
        assert_eq!(usage_label(&priced), "12k tok · $0.31");
        assert_eq!(usage_label(&Totals { tokens: 950, ..Totals::default() }), "950 tok");
        let details = usage_details(&Totals { unpriced_tokens: 2_000, ..priced });
        assert!(details.starts_with("Input: 0\nOutput: 0"));
        assert!(details.ends_with("2k tokens from a model with no known price"));
        assert_eq!(agent_id(AgentKind::OpenCode), "opencode");
        assert_eq!(day_label("2026-10-26"), "10/26");
        assert_eq!(context_fill(100_000), Some((50, 200_000)));
        assert_eq!(context_fill(500_000), Some((50, 1_000_000)));
        assert_eq!(context_fill(0), None);
    }
}
