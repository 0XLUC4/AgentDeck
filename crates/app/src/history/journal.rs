//! Daily journal: every task that leaves a pane's "current" slot is saved to today's day file
//! (recorder), and the dialog shows them grouped by workspace > agent with today's commits and usage,
//! copyable / exportable as Markdown.
//! Original: `components/Journal/{journal.ts,recorder.ts,JournalDialog.tsx}`.

use std::collections::HashMap;

use agentdeck_core::agents::AgentKind;
use agentdeck_core::history::journal::{Commit, journal_append, journal_git_commits, journal_read, write_text_file};
use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::DeckState;
use agentdeck_core::logic::tasks::Task as TaskRecord;
use agentdeck_core::logic::text::{AGENT_KINDS, agent_name, format_cost, format_since, format_tokens, now_ms};
use agentdeck_core::usage::{UsageSummary, usage_summary};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Disableable, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde::{Deserialize, Serialize};

use super::{agent_mark, icon_button, modal, text_tooltip};
use crate::state::{Deck, DeckEvent};
use crate::theme::{hsla, palette, ui};

const MAX_ACTIONS: usize = 30;

/// What is saved for one finished (or interrupted) task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    #[serde(default)]
    pub pane_id: String,
    #[serde(default)]
    pub title: String,
    pub agent: AgentKind,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub workspace_name: String,
    /// "" when the pane belongs to the default workspace.
    #[serde(default)]
    pub workspace_path: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    pub actions: Vec<String>,
    pub started_at: i64,
    pub ended_at: i64,
    #[serde(default = "idle")]
    pub status: PaneStatus,
}

fn idle() -> PaneStatus {
    PaneStatus::Idle
}

/// The journal entry of the pane's latest finished task.
fn to_journal_entry(state: &DeckState, pane_id: &str, task: &TaskRecord) -> Option<JournalEntry> {
    let pane = state.panes.get(pane_id)?;
    let workspace = state
        .tabs
        .iter()
        .find(|t| t.pane_ids.iter().any(|id| id == pane_id))
        .and_then(|tab| state.workspaces.iter().find(|w| w.id == tab.workspace_id));
    let skip = task.actions.len().saturating_sub(MAX_ACTIONS);
    Some(JournalEntry {
        pane_id: pane_id.to_owned(),
        title: pane.title.clone(),
        agent: pane.agent,
        cwd: pane.cwd.clone(),
        workspace_name: workspace.map(|w| w.name.clone()).unwrap_or_default(),
        workspace_path: workspace.map(|w| w.path.clone()).unwrap_or_default(),
        prompt: task.prompt.clone(),
        summary: task.summary.clone(),
        actions: task.actions[skip..].iter().map(|a| a.text.clone()).collect(),
        started_at: task.started_at,
        ended_at: task.ended_at.unwrap_or_else(now_ms),
        status: task.status,
    })
}

/// Keeps only well-formed entries from a day file (it is plain JSON on disk).
pub fn parse_entries(raw: Vec<serde_json::Value>) -> Vec<JournalEntry> {
    raw.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect()
}

pub struct WorkspaceCommits {
    pub name: String,
    pub path: String,
    pub commits: Vec<Commit>,
}

pub struct JournalGroup {
    /// Workspace path, or the pane cwd when the workspace has none.
    pub key: String,
    pub name: String,
    pub agents: Vec<(AgentKind, Vec<JournalEntry>)>,
    pub commits: Vec<Commit>,
}

fn norm(path: &str) -> String {
    path.trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase()
}

fn last_segment(path: &str) -> &str {
    path.split(['\\', '/']).filter(|s| !s.is_empty()).last().unwrap_or(path)
}

/// Workspace > agent (fixed agent order) > tasks by start time. Workspaces with only commits are kept.
pub fn group_journal(mut entries: Vec<JournalEntry>, commits: Vec<WorkspaceCommits>) -> Vec<JournalGroup> {
    let mut groups: Vec<JournalGroup> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut group = |groups: &mut Vec<JournalGroup>, path: &str, name: &str| -> usize {
        *index.entry(norm(path)).or_insert_with(|| {
            let name = [name, last_segment(path)].into_iter().find(|n| !n.is_empty()).unwrap_or("No project");
            groups.push(JournalGroup { key: path.to_owned(), name: name.to_owned(), agents: Vec::new(), commits: Vec::new() });
            groups.len() - 1
        })
    };
    entries.sort_by_key(|e| e.started_at);
    for e in entries {
        let path = if e.workspace_path.is_empty() { e.cwd.clone() } else { e.workspace_path.clone() };
        let at = group(&mut groups, &path, &e.workspace_name);
        let g = &mut groups[at];
        match g.agents.iter_mut().find(|(a, _)| *a == e.agent) {
            Some((_, tasks)) => tasks.push(e),
            None => g.agents.push((e.agent, vec![e])),
        }
    }
    for c in commits.into_iter().filter(|c| !c.commits.is_empty()) {
        let at = group(&mut groups, &c.path, &c.name);
        groups[at].commits.extend(c.commits);
    }
    let order = |a: &AgentKind| AGENT_KINDS.iter().position(|k| k == a).unwrap_or(usize::MAX);
    for g in &mut groups {
        g.agents.sort_by_key(|(a, _)| order(a));
    }
    groups
}

/// Whitespace collapsed, cut to `max` chars with an ellipsis.
fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        format!("{}…", flat.chars().take(max - 1).collect::<String>())
    } else {
        flat
    }
}

/// "14:05", local time.
pub fn clock_time(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// Commit author date (ISO 8601) as local "14:05".
fn commit_time(iso: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|t| clock_time(t.timestamp_millis()))
        .unwrap_or_default()
}

fn status_label(status: PaneStatus) -> Option<&'static str> {
    match status {
        PaneStatus::Error => Some("error"),
        PaneStatus::Working | PaneStatus::Waiting => Some("interrupted"),
        _ => None,
    }
}

pub fn task_line(e: &JournalEntry) -> String {
    let head = format!(
        "{} → {} ({})",
        clock_time(e.started_at),
        clock_time(e.ended_at),
        format_since(e.ended_at - e.started_at)
    );
    let status = status_label(e.status).map(|s| format!(" [{s}]")).unwrap_or_default();
    let prompt = e.prompt.as_deref().filter(|p| !p.is_empty());
    let prompt = prompt.map_or_else(|| "(task without a prompt)".to_owned(), |p| one_line(p, 200));
    format!("{head}{status} - {prompt}")
}

/// Local YYYY-MM-DD, the backend's day-file name.
pub fn day_key() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// "2026-10-26" -> "10/26/2026".
fn us_date(date: &str) -> String {
    let mut parts = date.splitn(3, '-');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(y), Some(m), Some(d)) => format!("{m}/{d}/{y}"),
        _ => date.to_owned(),
    }
}

fn usage_agent_label(key: &str) -> String {
    super::agent_kind(key).map_or_else(|| key.to_owned(), |k| agent_name(k).to_owned())
}

pub fn journal_markdown(date: &str, groups: &[JournalGroup], usage: Option<&UsageSummary>) -> String {
    let mut out = vec![format!("# Journal for {}", us_date(date)), String::new()];
    if let Some(usage) = usage.filter(|u| u.total.tokens > 0) {
        let agents = usage
            .by_agent
            .iter()
            .map(|a| format!("{} {}", usage_agent_label(&a.key), format_cost(a.totals.cost)))
            .collect::<Vec<_>>()
            .join(", ");
        out.push(format!(
            "**Usage**: {} tokens · {} ({agents})",
            format_tokens(usage.total.tokens as f64),
            format_cost(usage.total.cost)
        ));
        out.push(String::new());
    }
    if groups.is_empty() {
        out.extend(["_No tasks or commits today._".to_owned(), String::new()]);
    }
    for g in groups {
        out.extend([format!("## {}", g.name), String::new()]);
        if !g.key.is_empty() {
            out.extend([format!("`{}`", g.key), String::new()]);
        }
        for (agent, tasks) in &g.agents {
            out.extend([format!("### {} - {} task(s)", agent_name(*agent), tasks.len()), String::new()]);
            for t in tasks {
                out.push(format!("- {}", task_line(t)));
                if let Some(summary) = t.summary.as_deref().filter(|s| !s.is_empty()) {
                    out.push(format!("  - Result: {}", one_line(summary, 400)));
                }
                if !t.actions.is_empty() {
                    let last = t.actions[t.actions.len().saturating_sub(3)..]
                        .iter()
                        .map(|a| one_line(a, 80))
                        .collect::<Vec<_>>()
                        .join(" · ");
                    out.push(format!("  - {} action(s): {last}", t.actions.len()));
                }
            }
            out.push(String::new());
        }
        if !g.commits.is_empty() {
            out.extend([format!("### Commits - {}", g.commits.len()), String::new()]);
            for c in &g.commits {
                out.push(format!("- `{}` {} {}", c.hash, commit_time(&c.time), c.subject));
            }
            out.push(String::new());
        }
    }
    format!("{}\n", out.join("\n").trim_end())
}

/// Saves every task that leaves a pane's "current" slot to today's journal file
/// (original `recorder.ts`). Tasks change only on `agent-status`, so only that event is checked.
pub(super) fn start_recorder(deck: &Entity<Deck>, cx: &mut App) {
    let mut seen: HashMap<String, TaskRecord> = HashMap::new();
    cx.subscribe(deck, move |deck, event: &DeckEvent, cx| {
        let DeckEvent::Backend(event) = event;
        if event.name != "agent-status" {
            return;
        }
        let Some(pane_id) = event.payload["paneId"].as_str() else { return };
        let deck = deck.read(cx);
        let Some(latest) = deck.tasks.get(pane_id).and_then(|log| log.history.first()) else { return };
        if seen.get(pane_id) == Some(latest) {
            return;
        }
        seen.insert(pane_id.to_owned(), latest.clone());
        if latest.ended_at.is_none() {
            return;
        }
        let Some(entry) = to_journal_entry(&deck.state, pane_id, latest) else { return };
        let value = match serde_json::to_value(entry) {
            Ok(v) => v,
            Err(e) => return eprintln!("journal entry: {e}"),
        };
        cx.background_spawn(async move {
            if let Err(e) = journal_append(value) {
                eprintln!("journal_append failed: {e}");
            }
        })
        .detach();
    })
    .detach();
}

struct Loaded {
    groups: Vec<JournalGroup>,
    usage: Option<UsageSummary>,
}

/// Day file, today's usage, and today's commits of linked workspaces plus the folders tasks ran in.
fn load(date: String, workspaces: Vec<(String, String)>) -> Result<Loaded, String> {
    let entries = parse_entries(journal_read(date)?);
    let usage = usage_summary("today".into()).map_err(|e| eprintln!("usage_summary failed: {e}")).ok();
    let mut places: Vec<(String, String)> = Vec::new();
    let candidates = workspaces.into_iter().filter(|(_, path)| !path.is_empty()).chain(entries.iter().map(|e| {
        let path = if e.workspace_path.is_empty() { &e.cwd } else { &e.workspace_path };
        (e.workspace_name.clone(), path.clone())
    }));
    for (name, path) in candidates {
        if !path.is_empty() && !places.iter().any(|(_, p)| p.to_lowercase() == path.to_lowercase()) {
            places.push((name, path));
        }
    }
    let commits = places
        .into_iter()
        .map(|(name, path)| {
            let commits = journal_git_commits(path.clone()).unwrap_or_else(|e| {
                eprintln!("journal_git_commits failed: {e}");
                Vec::new()
            });
            WorkspaceCommits { name, path, commits }
        })
        .collect();
    Ok(Loaded { groups: group_journal(entries, commits), usage })
}

pub struct JournalDialog {
    deck: Entity<Deck>,
    date: String,
    data: Option<Loaded>,
    error: Option<String>,
    focus: FocusHandle,
    _load: Task<()>,
}

impl JournalDialog {
    pub fn new(deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let date = day_key();
        let workspaces = deck.read(cx).state.workspaces.iter().map(|w| (w.name.clone(), w.path.clone())).collect();
        let day = date.clone();
        let load = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { load(day, workspaces) }).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(data) => this.data = Some(data),
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        });
        Self { deck, date, data: None, error: None, focus, _load: load }
    }

    fn markdown(&self) -> Option<String> {
        let data = self.data.as_ref()?;
        Some(journal_markdown(&self.date, &data.groups, data.usage.as_ref()))
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let Some(md) = self.markdown() else { return };
        cx.write_to_clipboard(ClipboardItem::new_string(md));
        self.deck.update(cx, |deck, cx| deck.toast("Journal copied as Markdown", cx));
    }

    fn export(&mut self, cx: &mut Context<Self>) {
        let Some(md) = self.markdown() else { return };
        let name = format!("journal-{}.md", self.date);
        let deck = self.deck.clone();
        cx.spawn(async move |_, cx| {
            let picked = rfd::AsyncFileDialog::new().set_file_name(name).add_filter("Markdown", &["md"]).save_file().await;
            let Some(file) = picked else { return };
            let path = file.path().to_path_buf();
            let shown = path.display().to_string();
            let written = cx.background_spawn(async move { write_text_file(&path, &md) }).await;
            let message = match written {
                Ok(()) => format!("Journal exported: {shown}"),
                Err(e) => format!("Could not export: {e}"),
            };
            deck.update(cx, |deck, cx| deck.toast(message, cx));
        })
        .detach();
    }

    fn render_body(&self, cx: &App) -> Div {
        let p = palette(cx);
        if let Some(error) = &self.error {
            return div().text_xs().text_color(hsla(p.bad)).child(format!("Journal unavailable: {error}"));
        }
        let Some(data) = &self.data else {
            return div().py_8().flex().justify_center().text_color(hsla(p.muted)).child("Preparing journal…");
        };
        let mut body = div().flex().flex_col().gap_6();
        if let Some(usage) = data.usage.as_ref().filter(|u| u.total.tokens > 0) {
            body = body.child(
                div()
                    .flex()
                    .gap_1()
                    .text_color(hsla(p.muted))
                    .child(div().text_color(hsla(p.fg)).child(format_cost(usage.total.cost)))
                    .child(format!("· {} tokens today (Claude Code, Codex)", format_tokens(usage.total.tokens as f64))),
            );
        }
        if data.groups.is_empty() {
            body = body.child(
                div().py_6().flex().justify_center().text_color(hsla(p.muted)).child("No tasks or commits today"),
            );
        }
        let mono = agentdeck_core::logic::theme::FONT_MONO;
        for (gi, g) in data.groups.iter().enumerate() {
            let mut section = div().flex().flex_col().gap_3().child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .border_b_1()
                    .border_color(hsla(p.line))
                    .pb(ui(6.0))
                    .child(div().text_size(ui(15.0)).font_weight(FontWeight::MEDIUM).child(g.name.clone()))
                    .when(!g.key.is_empty(), |d| {
                        d.child(div().truncate().font_family(mono).text_xs().text_color(hsla(p.subtle)).child(g.key.clone()))
                    }),
            );
            for (ai, (agent, tasks)) in g.agents.iter().enumerate() {
                let items = tasks.iter().enumerate().map(|(ti, t)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(ui(2.0))
                        .child(div().text_color(hsla(p.fg)).child(task_line(t)))
                        .children(t.summary.clone().filter(|s| !s.is_empty()).map(|s| {
                            div().text_xs().text_color(hsla(p.muted)).line_clamp(3).child(s)
                        }))
                        .when(!t.actions.is_empty(), |d| {
                            let last = t.actions[t.actions.len().saturating_sub(3)..].join(" · ");
                            d.child(
                                div()
                                    .id(("journal-actions", gi * 1_000_000 + ai * 10_000 + ti))
                                    .truncate()
                                    .text_xs()
                                    .text_color(hsla(p.subtle))
                                    .tooltip(text_tooltip(t.actions.join("\n")))
                                    .child(format!("{} action(s) · {last}", t.actions.len())),
                            )
                        })
                });
                section = section.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(ui(6.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(hsla(p.muted))
                                .child(agent_mark(*agent, 13.0, cx))
                                .child(format!("{} · {} task(s)", agent_name(*agent), tasks.len())),
                        )
                        .child(div().flex().flex_col().gap_2().pl_5().children(items)),
                );
            }
            if !g.commits.is_empty() {
                let rows = g.commits.iter().map(|c| {
                    div()
                        .flex()
                        .gap_2()
                        .text_xs()
                        .child(div().font_family(mono).text_color(hsla(p.subtle)).child(c.hash.clone()))
                        .child(div().text_color(hsla(p.subtle)).child(commit_time(&c.time)))
                        .child(div().truncate().text_color(hsla(p.fg)).child(c.subject.clone()))
                });
                section = section.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(ui(6.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(hsla(p.muted))
                                .child(Icon::from(IconName::GitCommitHorizontal))
                                .child(format!("{} commit(s)", g.commits.len())),
                        )
                        .child(div().flex().flex_col().gap_1().pl_5().children(rows)),
                );
            }
            body = body.child(section);
        }
        body
    }
}

impl Render for JournalDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loading = self.data.is_none();
        let this = cx.entity().downgrade();
        let copy = icon_button("journal-copy", IconName::ClipboardCopy, "Copy as Markdown", {
            let this = this.clone();
            move |_, _, cx| {
                let _ = this.update(cx, |this, cx| this.copy(cx));
            }
        })
        .disabled(loading);
        let export = icon_button("journal-export", IconName::Download, "Export (.md)", move |_, _, cx| {
            let _ = this.update(cx, |this, cx| this.export(cx));
        })
        .disabled(loading);
        let title = format!("Journal for {}", us_date(&self.date));
        let body = self.render_body(cx);
        modal(title, 44.0, vec![copy.into_any_element(), export.into_any_element()], &self.focus, body, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::{JournalEntry, WorkspaceCommits, group_journal, journal_markdown, parse_entries};
    use agentdeck_core::agents::AgentKind;
    use agentdeck_core::history::journal::Commit;
    use agentdeck_core::hooks_server::PaneStatus;
    use agentdeck_core::usage::{Keyed, Totals, UsageSummary};
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> i64 {
        chrono::Local.with_ymd_and_hms(2026, 10, 26, h, m, 0).single().map_or(0, |t| t.timestamp_millis())
    }

    fn entry() -> JournalEntry {
        JournalEntry {
            pane_id: "p1".into(),
            title: "Claude Code".into(),
            agent: AgentKind::Claude,
            cwd: "D:\\proj".into(),
            workspace_name: "proj".into(),
            workspace_path: "D:\\proj".into(),
            prompt: Some("Fix the bug".into()),
            summary: Some("Fixed.".into()),
            actions: vec!["Edit a.ts".into()],
            started_at: at(10, 0),
            ended_at: at(10, 12),
            status: PaneStatus::Done,
        }
    }

    fn commit(hash: &str, time: String, subject: &str) -> Commit {
        Commit { hash: hash.into(), time, author: "L".into(), subject: subject.into() }
    }

    #[test]
    fn groups_by_workspace_then_agent_in_agent_order() {
        let groups = group_journal(
            vec![
                JournalEntry { agent: AgentKind::Codex, started_at: at(11, 0), ended_at: at(11, 5), prompt: Some("B".into()), ..entry() },
                JournalEntry { started_at: at(9, 0), ended_at: at(9, 30), prompt: Some("A".into()), ..entry() },
                JournalEntry { workspace_path: "D:/proj/".into(), prompt: Some("C".into()), started_at: at(12, 0), ended_at: at(12, 1), ..entry() },
                JournalEntry { workspace_name: String::new(), workspace_path: String::new(), cwd: "C:\\tmp\\scratch".into(), prompt: Some("D".into()), ..entry() },
            ],
            Vec::new(),
        );
        assert_eq!(groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["proj", "scratch"]);
        assert_eq!(groups[0].agents.iter().map(|(a, _)| *a).collect::<Vec<_>>(), [AgentKind::Claude, AgentKind::Codex]);
        let prompts: Vec<_> = groups[0].agents[0].1.iter().map(|t| t.prompt.clone().unwrap_or_default()).collect();
        assert_eq!(prompts, ["A", "C"]);
    }

    #[test]
    fn keeps_workspaces_that_only_have_commits() {
        let commits = vec![WorkspaceCommits {
            name: "other".into(),
            path: "E:\\other".into(),
            commits: vec![commit("abc1234", "2026-10-26T14:05:00+01:00".into(), "feat: x")],
        }];
        let groups = group_journal(Vec::new(), commits);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].commits.len(), 1);
    }

    #[test]
    fn drops_malformed_day_file_entries() {
        let good = serde_json::to_value(entry()).unwrap_or_default();
        let mut bad_agent = good.clone();
        bad_agent["agent"] = "nope".into();
        let raw = vec![good, serde_json::json!({ "foo": 1 }), serde_json::Value::Null, bad_agent];
        assert_eq!(parse_entries(raw).len(), 1);
    }

    #[test]
    fn renders_markdown() {
        let at15 = chrono::Local.with_ymd_and_hms(2026, 10, 26, 15, 0, 0).single().map(|t| t.to_rfc3339()).unwrap_or_default();
        let groups = group_journal(
            vec![
                entry(),
                JournalEntry {
                    status: PaneStatus::Working,
                    prompt: Some("multi\nline   prompt".into()),
                    summary: None,
                    actions: Vec::new(),
                    ..entry()
                },
            ],
            vec![WorkspaceCommits { name: "proj".into(), path: "D:\\proj".into(), commits: vec![commit("abc1234", at15, "fix: y")] }],
        );
        let totals = Totals { tokens: 12_000, cost: 0.31, ..Totals::default() };
        let usage = UsageSummary {
            total: totals.clone(),
            by_agent: vec![Keyed { key: "claude".into(), totals }],
            by_workspace_cwd: Vec::new(),
            by_day: Vec::new(),
        };
        let md = journal_markdown("2026-10-26", &groups, Some(&usage));
        for line in [
            "# Journal for 10/26/2026",
            "**Usage**: 12k tokens · $0.31 (Claude Code $0.31)",
            "## proj",
            "### Claude Code - 2 task(s)",
            "- 10:00 → 10:12 (12 min) - Fix the bug",
            "  - Result: Fixed.",
            "[interrupted] - multi line prompt",
            "### Commits - 1",
            "- `abc1234` 15:00 fix: y",
        ] {
            assert!(md.contains(line), "missing {line:?} in\n{md}");
        }
        assert!(md.ends_with('\n'));
    }

    #[test]
    fn says_when_the_day_is_empty() {
        assert!(journal_markdown("2026-10-26", &[], None).contains("No tasks"));
    }
}
