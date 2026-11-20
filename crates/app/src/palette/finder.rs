//! Keyboard-first finders, one view in four modes sharing the list, keys and rows:
//! - Quick Open (Ctrl+P): files of the active session's folder or worktree;
//! - Jump (Ctrl+J): recent agent panes and sessions, or sessions, projects and panes matching the query,
//!   with project/host filter chips (Tab);
//! - New tab (Ctrl+T, the "+" of the session tab bar): open tabs, files, agents, URLs, web search;
//! - Run project command (command palette): package.json scripts and Cargo commands of the worktree,
//!   each run in its own terminal pane.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use agentdeck_core::agents::AgentKind;
use agentdeck_core::git;
use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::{ActivityKey, Pane, PaneSpec, Tab, activity_key};
use agentdeck_core::logic::project_commands::{ProjectCommand, shell_args};
use agentdeck_core::logic::text::{AGENT_KINDS, agent_name, split_args};
use agentdeck_core::logic::theme::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::command::{Run, close_wall, run_target};
use super::fuzzy::fuzzy_score;
use super::rank::{
    FileEntry, PaneInfo, by_score_then_recency, classify_web, pane_score, parent_label, parse_file_query, rank_files,
    recent_panes, web_search_url,
};
use super::ui;
use crate::panes::{display_title, status_color};
use crate::state::Deck;
use crate::theme::{hsla, palette, ui as size};

actions!(
    agentdeck_finder,
    [FinderNext, FinderPrev, FinderRun, FinderClose, FinderFilters, FinderRow1, FinderRow2, FinderRow3, FinderRow4, FinderRow5, FinderRow6]
);

const CONTEXT: &str = "Finder";
const MAX_FILE_ROWS: usize = 50;
/// Rows of "Recent chats & terminals" before "See more".
const RECENT_PANES: usize = 6;
const MAX_SESSIONS: usize = 8;
static ROW_KEYS: [[&str; 2]; 6] = [["Ctrl", "1"], ["Ctrl", "2"], ["Ctrl", "3"], ["Ctrl", "4"], ["Ctrl", "5"], ["Ctrl", "6"]];
const LOCAL_ONLY: &str = "This session runs over SSH: Quick Open and project commands work on local worktrees for now.";

/// Context-scoped: they win over the dialog's Enter/Esc, Root's Tab and the global Ctrl+1..6.
pub(super) fn bind_keys(cx: &mut App) {
    let row = |n: usize| super::primary(&n.to_string());
    cx.bind_keys([
        KeyBinding::new("down", FinderNext, Some(CONTEXT)),
        KeyBinding::new("up", FinderPrev, Some(CONTEXT)),
        KeyBinding::new("enter", FinderRun, Some(CONTEXT)),
        // shortcut: the dock has no "open a session in a split": Shift+Enter opens it like Enter.
        KeyBinding::new("shift-enter", FinderRun, Some(CONTEXT)),
        KeyBinding::new("escape", FinderClose, Some(CONTEXT)),
        KeyBinding::new("tab", FinderFilters, Some(CONTEXT)),
        KeyBinding::new(super::leak(row(1)), FinderRow1, Some(CONTEXT)),
        KeyBinding::new(super::leak(row(2)), FinderRow2, Some(CONTEXT)),
        KeyBinding::new(super::leak(row(3)), FinderRow3, Some(CONTEXT)),
        KeyBinding::new(super::leak(row(4)), FinderRow4, Some(CONTEXT)),
        KeyBinding::new(super::leak(row(5)), FinderRow5, Some(CONTEXT)),
        KeyBinding::new(super::leak(row(6)), FinderRow6, Some(CONTEXT)),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Files,
    Jump,
    NewTab,
    Commands,
}

/// A Jump filter chip; cleared when the finder closes.
#[derive(Clone, Debug, PartialEq)]
enum Filter {
    Project { id: String, name: String },
    /// `id` None: this computer.
    Host { id: Option<String>, name: String },
}

impl Filter {
    fn label(&self) -> String {
        match self {
            Filter::Project { name, .. } => format!("Project: {name}"),
            Filter::Host { name, .. } => format!("Host: {name}"),
        }
    }
}

#[derive(Clone)]
enum Target {
    Jump(Run),
    NewSession,
    /// Relative to the finder's root; `line` 1-based.
    File { path: String, line: Option<u32> },
    CreateWorktree,
    Agent(AgentKind),
    Url(String),
    SeeMore,
    AddFilter(Filter),
    Command(ProjectCommand),
}

#[derive(Clone)]
enum Lead {
    Icon(IconName),
    Agent(AgentKind),
}

#[derive(Clone)]
struct Row {
    target: Target,
    lead: Lead,
    title: String,
    /// Muted text after the title (parent folder, topic).
    detail: Option<String>,
    tooltip: Option<String>,
    /// Compact location chip ("project · session").
    chip: Option<String>,
    status: Option<ActivityKey>,
    keys: &'static [&'static str],
}

impl Row {
    fn new(target: Target, lead: Lead, title: impl Into<String>) -> Self {
        Self { target, lead, title: title.into(), detail: None, tooltip: None, chip: None, status: None, keys: &[] }
    }

    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into()).filter(|d: &String| !d.is_empty());
        self
    }

    fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    fn chip(mut self, chip: impl Into<String>) -> Self {
        self.chip = Some(chip.into());
        self
    }
}

type Sections = Vec<(String, Vec<Row>)>;

pub struct Finder {
    deck: Entity<Deck>,
    mode: Mode,
    query: String,
    focus: FocusHandle,
    scroll: ScrollHandle,
    sections: Sections,
    active: usize,
    /// Error or notice above the list (SSH session, invalid manifest, missing package manager).
    message: Option<String>,
    loading: bool,
    /// Searched folder: the active pane's worktree, else the project folder.
    root: Option<PathBuf>,
    files: Arc<Vec<FileEntry>>,
    file_rows: Vec<Row>,
    /// Bumped by every search: a running one sees it changed and stops.
    search_gen: Arc<AtomicUsize>,
    commands: Vec<ProjectCommand>,
    /// Jump: pane order frozen at open, so rows do not move under the keyboard.
    recent: Vec<String>,
    see_more: bool,
    filters: Vec<Filter>,
    filter_menu: bool,
    // Replacing one drops (cancels) the previous run.
    _load: Option<Task<()>>,
    _search: Option<Task<()>>,
}

/// Opens a finder; the same shortcut again closes it, another one switches mode.
pub(super) fn open(mode: Mode, deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    let open = cx.global::<super::PaletteGlobal>().finder.as_ref().and_then(WeakEntity::upgrade);
    if let Some(view) = open {
        let same = view.read(cx).mode == mode;
        cx.global_mut::<super::PaletteGlobal>().finder = None;
        window.close_dialog(cx);
        if same {
            return;
        }
    }
    let view = cx.new(|cx| Finder::new(mode, deck, cx));
    cx.global_mut::<super::PaletteGlobal>().finder = Some(view.downgrade());
    let focus = view.read(cx).focus.clone();
    let top = window.viewport_size().height * 0.12;
    super::open_modal(view, 640.0, Some(top), focus, window, cx);
}

/// (cwd, pane id, worktree) whose folder the finder searches: the current local pane's, else the
/// active project's. Err: an SSH session or a project without folder.
fn root_source(deck: &Deck) -> Result<(String, String, bool), String> {
    if let Some((_, id)) = super::current_pane(deck)
        && let Some(pane) = deck.state.panes.get(&id)
    {
        if pane.ssh.is_some() {
            return Err(LOCAL_ONLY.into());
        }
        if !pane.cwd.is_empty() {
            return Ok((pane.cwd.clone(), id, pane.worktree));
        }
    }
    let path = deck.state.active_workspace().path.clone();
    if path.is_empty() {
        return Err("This project has no folder: link one first.".into());
    }
    Ok((path, String::new(), false))
}

fn host_key(pane: &Pane) -> Option<String> {
    pane.ssh.as_ref().map(|ssh| pane.host.clone().unwrap_or_else(|| ssh.clone()))
}

fn ws_name(deck: &Deck, id: &str) -> String {
    deck.state.workspaces.iter().find(|w| w.id == id).map(|w| w.name.clone()).unwrap_or_default()
}

fn pane_row(pane: &Pane, tab: &Tab, deck: &Deck) -> Row {
    let mut row = Row::new(Target::Jump(Run::Pane(pane.id.clone())), Lead::Agent(pane.agent), display_title(pane))
        .detail(pane.topic.clone().unwrap_or_default())
        .chip(format!("{} · {}", ws_name(deck, &tab.workspace_id), tab.name));
    row.status = Some(activity_key(pane));
    row
}

fn tab_row(tab: &Tab, deck: &Deck) -> Row {
    let ws = deck.state.workspaces.iter().find(|w| w.id == tab.workspace_id);
    let branch = ws.and_then(|w| w.git.as_ref()).and_then(|g| g.branch.clone()).unwrap_or_default();
    Row::new(Target::Jump(Run::Tab(tab.id.clone())), Lead::Icon(IconName::Layers), tab.name.clone())
        .detail(branch)
        .chip(ws.map(|w| w.name.clone()).unwrap_or_default())
}

fn file_row(file: &FileEntry, line: Option<u32>) -> Row {
    let name = file.path.rsplit('/').next().unwrap_or(&file.path);
    let title = match line {
        Some(line) => format!("{name}:{line}"),
        None => name.to_owned(),
    };
    let icon = if file.ignored { IconName::File } else { IconName::FileText };
    let row = Row::new(Target::File { path: file.path.clone(), line }, Lead::Icon(icon), title)
        .detail(parent_label(&file.path, 56))
        .tooltip(file.path.clone());
    if file.ignored { row.chip("ignored") } else { row }
}

impl Finder {
    fn new(mode: Mode, deck: Entity<Deck>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            deck,
            mode,
            query: String::new(),
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            sections: Vec::new(),
            active: 0,
            message: None,
            loading: false,
            root: None,
            files: Arc::new(Vec::new()),
            file_rows: Vec::new(),
            search_gen: Arc::new(AtomicUsize::new(0)),
            commands: Vec::new(),
            recent: Vec::new(),
            see_more: false,
            filters: Vec::new(),
            filter_menu: false,
            _load: None,
            _search: None,
        };
        match mode {
            Mode::Jump => this.recent = this.recent_pane_ids(cx),
            Mode::Files | Mode::NewTab | Mode::Commands => this.load_root(cx),
        }
        this.refresh(cx);
        this
    }

    fn recent_pane_ids(&self, cx: &App) -> Vec<String> {
        let deck = self.deck.read(cx);
        let panes: Vec<PaneInfo> = deck
            .state
            .tabs
            .iter()
            .flat_map(|t| &t.pane_ids)
            .filter_map(|id| deck.state.panes.get(id))
            .map(|p| PaneInfo { id: p.id.clone(), key: activity_key(p), focused_at: super::focused_at(&p.id, cx) })
            .collect();
        let current = super::current_pane(deck).map(|(_, pane)| pane);
        recent_panes(&panes, current.as_deref())
    }

    /// Finds the root folder, then lists its files (tracked first, gitignored second) or its commands.
    fn load_root(&mut self, cx: &mut Context<Self>) {
        let (cwd, pane, worktree) = match root_source(self.deck.read(cx)) {
            Ok(source) => source,
            Err(e) => {
                // The omnibox still has tabs, agents and the web without a folder.
                if self.mode != Mode::NewTab {
                    self.message = Some(e);
                }
                return;
            }
        };
        self.loading = true;
        let mode = self.mode;
        self._load = Some(cx.spawn(async move |this, cx| {
            let bg = cx.background_executor().clone();
            let root = bg.spawn(async move { PathBuf::from(git::git_pane_dir(cwd, pane, worktree)) }).await;
            if mode == Mode::Commands {
                let dir = root.clone();
                let found = bg.spawn(async move { super::scan::detect_commands(&dir) }).await;
                let _ = this.update(cx, |this, cx| {
                    this.loading = false;
                    this.root = Some(root);
                    this.commands = found.commands;
                    this.message = (!found.problems.is_empty()).then(|| found.problems.join("\n"));
                    this.refresh(cx);
                });
                return;
            }
            for ignored in [false, true] {
                let dir = root.clone();
                let listed = bg.spawn(async move { super::scan::list_files(&dir, ignored) }).await;
                let applied = this.update(cx, |this, cx| {
                    this.root = Some(root.clone());
                    let mut all = (*this.files).clone();
                    all.extend(listed);
                    this.files = Arc::new(all);
                    this.loading = !ignored;
                    this.search_files(cx);
                });
                if applied.is_err() {
                    return;
                }
            }
        }));
    }

    /// Ranks the files for the query off the UI thread; a newer search cancels this one.
    fn search_files(&mut self, cx: &mut Context<Self>) {
        let generation = self.search_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let skip = self.mode == Mode::NewTab && (self.query.trim().is_empty() || self.query.trim_start().starts_with('?'));
        let Some(root) = self.root.clone().filter(|_| !skip && matches!(self.mode, Mode::Files | Mode::NewTab)) else {
            self.file_rows.clear();
            self.refresh(cx);
            return;
        };
        let query = parse_file_query(&self.query);
        let recent = super::recent_files(&root.to_string_lossy(), cx);
        let (files, counter) = (self.files.clone(), self.search_gen.clone());
        self._search = Some(cx.spawn(async move |this, cx| {
            let line = query.line;
            let found = cx
                .background_executor()
                .spawn(async move {
                    let cancelled = || counter.load(Ordering::SeqCst) != generation;
                    let ranked = rank_files(&files, &query, &recent, MAX_FILE_ROWS, &cancelled)?;
                    Some(ranked.into_iter().map(|i| files[i].clone()).collect::<Vec<_>>())
                })
                .await;
            let Some(found) = found else { return };
            let _ = this.update(cx, |this, cx| {
                this.file_rows = found.iter().map(|f| file_row(f, line)).collect();
                this.refresh(cx);
            });
        }));
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.sections = self.build(cx);
        let count = self.flat().len();
        if self.active >= count {
            self.active = count.saturating_sub(1);
        }
        cx.notify();
    }

    fn build(&self, cx: &App) -> Sections {
        let sections = match self.mode {
            Mode::Files => {
                let label = if self.query.trim().is_empty() { "Recent and project files" } else { "Files" };
                vec![(label.to_owned(), self.file_rows.clone())]
            }
            Mode::Commands => self.command_sections(),
            Mode::Jump if self.filter_menu => self.filter_sections(cx),
            Mode::Jump if self.query.trim().is_empty() => self.recent_sections(cx),
            Mode::Jump => self.jump_sections(cx),
            Mode::NewTab => self.new_tab_sections(cx),
        };
        sections.into_iter().filter(|(_, rows)| !rows.is_empty()).collect()
    }

    fn command_sections(&self) -> Sections {
        let Some(root) = &self.root else { return Vec::new() };
        let dir = root.to_string_lossy().into_owned();
        let q = self.query.trim();
        let mut scored: Vec<(f64, &ProjectCommand)> =
            self.commands.iter().filter_map(|c| Some((fuzzy_score(q, &c.display())?, c))).collect();
        if !q.is_empty() {
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        }
        let rows = scored
            .into_iter()
            .map(|(_, c)| {
                Row::new(Target::Command(c.clone()), Lead::Icon(IconName::SquareTerminal), c.display())
                    .detail(format!("in {dir}"))
                    .tooltip(format!("{} (in {dir})", c.display()))
            })
            .collect();
        vec![(format!("Run in a new terminal · {dir}"), rows)]
    }

    fn project_ok(&self, ws: &str) -> bool {
        let projects: Vec<&String> = self
            .filters
            .iter()
            .filter_map(|f| match f {
                Filter::Project { id, .. } => Some(id),
                Filter::Host { .. } => None,
            })
            .collect();
        projects.is_empty() || projects.iter().any(|id| *id == ws)
    }

    fn host_ok(&self, pane: &Pane) -> bool {
        let hosts: Vec<&Option<String>> = self
            .filters
            .iter()
            .filter_map(|f| match f {
                Filter::Host { id, .. } => Some(id),
                Filter::Project { .. } => None,
            })
            .collect();
        hosts.is_empty() || hosts.contains(&&host_key(pane))
    }

    fn has_host_filter(&self) -> bool {
        self.filters.iter().any(|f| matches!(f, Filter::Host { .. }))
    }

    fn tab_ok(&self, tab: &Tab, deck: &Deck) -> bool {
        self.project_ok(&tab.workspace_id)
            && (!self.has_host_filter()
                || tab.pane_ids.iter().filter_map(|id| deck.state.panes.get(id)).any(|p| self.host_ok(p)))
    }

    /// Empty Jump: recent agent panes (frozen order), then sessions by last focus.
    fn recent_sections(&self, cx: &App) -> Sections {
        let deck = self.deck.read(cx);
        let s = &deck.state;
        let tab_of = |id: &str| s.tabs.iter().find(|t| t.pane_ids.iter().any(|p| p == id));
        let visible: Vec<(&Pane, &Tab)> = self
            .recent
            .iter()
            .filter_map(|id| Some((s.panes.get(id)?, tab_of(id)?)))
            .filter(|(p, t)| self.project_ok(&t.workspace_id) && self.host_ok(p))
            .collect();
        let shown = if self.see_more { visible.len() } else { visible.len().min(RECENT_PANES) };
        let mut panes: Vec<Row> = visible
            .iter()
            .take(shown)
            .enumerate()
            .map(|(i, (pane, tab))| {
                let mut row = pane_row(pane, tab, deck);
                if let Some(keys) = ROW_KEYS.get(i) {
                    row.keys = keys;
                }
                row
            })
            .collect();
        if shown < visible.len() {
            panes.push(
                Row::new(Target::SeeMore, Lead::Icon(IconName::Ellipsis), "See more")
                    .detail(format!("{} more", visible.len() - shown)),
            );
        }
        let mut tabs: Vec<&Tab> = s.tabs.iter().filter(|t| t.id != s.active_tab_id && self.tab_ok(t, deck)).collect();
        tabs.sort_by_key(|t| std::cmp::Reverse(super::focused_at(&format!("tab:{}", t.id), cx)));
        let sessions = tabs.into_iter().take(MAX_SESSIONS).map(|t| tab_row(t, deck)).collect();
        vec![
            ("Recent chats & terminals".to_owned(), panes),
            ("Recent worktrees & sessions".to_owned(), sessions),
        ]
    }

    /// Typed Jump: sessions grouped by project, projects, open panes; else "Create worktree".
    fn jump_sections(&self, cx: &App) -> Sections {
        let deck = self.deck.read(cx);
        let s = &deck.state;
        let q = self.query.trim();
        let mut groups: Vec<(f64, String, Vec<Row>)> = Vec::new();
        for ws in s.workspaces.iter().filter(|w| self.project_ok(&w.id)) {
            let mut scored: Vec<(f64, i64, Row)> = Vec::new();
            for tab in s.tabs.iter().filter(|t| t.workspace_id == ws.id && self.tab_ok(t, deck)) {
                if let Some(score) = fuzzy_score(q, &tab.name) {
                    scored.push((score, super::focused_at(&format!("tab:{}", tab.id), cx), tab_row(tab, deck)));
                }
            }
            if !self.has_host_filter() {
                for session in s.archived.iter().filter(|a| a.workspace_id == ws.id) {
                    if let Some(score) = fuzzy_score(q, &session.name) {
                        let row = Row::new(Target::Jump(Run::Archived(session.id.clone())), Lead::Icon(IconName::Archive), session.name.clone())
                            .detail("archived")
                            .chip(ws.name.clone());
                        scored.push((score, session.saved_at, row));
                    }
                }
            }
            scored.sort_by(|a, b| by_score_then_recency((a.0, a.1), (b.0, b.1)));
            if let Some(best) = scored.first().map(|r| r.0) {
                groups.push((best, format!("Sessions · {}", ws.name), scored.into_iter().map(|r| r.2).collect()));
            }
        }
        groups.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut sections: Sections = groups.into_iter().map(|(_, label, rows)| (label, rows)).collect();

        let mut projects: Vec<(f64, Row)> = s
            .workspaces
            .iter()
            .filter(|w| self.project_ok(&w.id))
            .filter_map(|w| {
                let score = fuzzy_score(q, &w.name).or_else(|| fuzzy_score(q, &w.path).map(|s| s - 5.0))?;
                let row = Row::new(Target::Jump(Run::Workspace(w.id.clone())), Lead::Icon(IconName::Folder), w.name.clone())
                    .detail(w.path.clone())
                    .tooltip(w.path.clone());
                Some((score, row))
            })
            .collect();
        projects.sort_by(|a, b| b.0.total_cmp(&a.0));
        sections.push(("Projects".to_owned(), projects.into_iter().map(|r| r.1).collect()));

        let mut panes: Vec<(f64, i64, Row)> = Vec::new();
        for tab in s.tabs.iter().filter(|t| self.project_ok(&t.workspace_id)) {
            for pane in tab.pane_ids.iter().filter_map(|id| s.panes.get(id)).filter(|p| self.host_ok(p)) {
                let own = format!(
                    "{} {} {}",
                    display_title(pane),
                    pane.topic.as_deref().unwrap_or(""),
                    pane.last_action.as_deref().unwrap_or("")
                );
                let location = format!("{} {}", ws_name(deck, &tab.workspace_id), tab.name);
                if let Some(score) = pane_score(q, &own, &location) {
                    panes.push((score, super::focused_at(&pane.id, cx), pane_row(pane, tab, deck)));
                }
            }
        }
        panes.sort_by(|a, b| by_score_then_recency((a.0, a.1), (b.0, b.1)));
        sections.push(("Open panes".to_owned(), panes.into_iter().map(|r| r.2).collect()));

        if sections.iter().all(|(_, rows)| rows.is_empty()) {
            let row = Row::new(Target::CreateWorktree, Lead::Icon(IconName::GitBranch), format!("Create worktree '{q}'"))
                .detail("Opens the Create worktree dialog");
            return vec![(String::new(), vec![row])];
        }
        sections
    }

    /// Tab in Jump: projects and hosts to filter by.
    fn filter_sections(&self, cx: &App) -> Sections {
        let deck = self.deck.read(cx);
        let q = self.query.trim();
        let wanted = |f: &Filter, name: &str| !self.filters.contains(f) && fuzzy_score(q, name).is_some();
        let projects = deck
            .state
            .workspaces
            .iter()
            .map(|w| (Filter::Project { id: w.id.clone(), name: w.name.clone() }, w.name.clone()))
            .filter(|(f, name)| wanted(f, name))
            .map(|(f, name)| Row::new(Target::AddFilter(f), Lead::Icon(IconName::Folder), name))
            .collect();
        let hosts = std::iter::once(Filter::Host { id: None, name: "This computer".into() })
            .chain(crate::hosts::list(cx).into_iter().map(|h| Filter::Host { id: Some(h.id), name: h.name }))
            .filter_map(|f| {
                let Filter::Host { name, .. } = &f else { return None };
                let name = name.clone();
                wanted(&f, &name).then(|| Row::new(Target::AddFilter(f), Lead::Icon(IconName::Server), name))
            })
            .collect();
        vec![("Filter by project".to_owned(), projects), ("Filter by host".to_owned(), hosts)]
    }

    fn new_tab_sections(&self, cx: &App) -> Sections {
        let deck = self.deck.read(cx);
        let s = &deck.state;
        let web = classify_web(&self.query);
        let mut web_rows = Vec::new();
        if let Some(url) = &web.url {
            web_rows.push(Row::new(Target::Url(url.clone()), Lead::Icon(IconName::Globe), format!("Open {url}")).detail("in the browser panel"));
        }
        if let Some(text) = &web.search {
            web_rows.push(
                Row::new(Target::Url(web_search_url(text)), Lead::Icon(IconName::Search), format!("Search the web for \"{text}\""))
                    .detail("DuckDuckGo"),
            );
        }
        if web.web_only {
            return vec![("Web".to_owned(), web_rows)];
        }
        let q = self.query.trim();
        let mut sections = Vec::new();
        if fuzzy_score(q, "New session").is_some() {
            let row = Row::new(Target::NewSession, Lead::Icon(IconName::SquarePlus), "New session").detail(s.active_workspace().name.clone());
            sections.push((String::new(), vec![row]));
        }
        let mut tabs: Vec<(f64, i64, Row)> = Vec::new();
        for tab in &s.tabs {
            if let Some(score) = fuzzy_score(q, &tab.name) {
                tabs.push((score, super::focused_at(&format!("tab:{}", tab.id), cx), tab_row(tab, deck)));
            }
            for pane in tab.pane_ids.iter().filter_map(|id| s.panes.get(id)) {
                let location = format!("{} {}", ws_name(deck, &tab.workspace_id), tab.name);
                if let Some(score) = pane_score(q, &display_title(pane), &location) {
                    tabs.push((score, super::focused_at(&pane.id, cx), pane_row(pane, tab, deck)));
                }
            }
        }
        tabs.sort_by(|a, b| by_score_then_recency((a.0, a.1), (b.0, b.1)));
        sections.push(("Open tabs".to_owned(), tabs.into_iter().take(MAX_SESSIONS).map(|r| r.2).collect()));
        if !q.is_empty() {
            sections.push(("Files".to_owned(), self.file_rows.clone()));
        }
        let agents = AGENT_KINDS
            .into_iter()
            .filter(|kind| fuzzy_score(q, agent_name(*kind)).is_some())
            .map(|kind| Row::new(Target::Agent(kind), Lead::Agent(kind), format!("Start {}", agent_name(kind))).detail(s.active_workspace().name.clone()))
            .collect();
        sections.push(("Agents".to_owned(), agents));
        sections.push(("Web".to_owned(), web_rows));
        sections
    }

    fn flat(&self) -> Vec<&Row> {
        self.sections.iter().flat_map(|(_, rows)| rows).collect()
    }

    /// Scroll row of flat row `index`: every section has one header row.
    fn row_of(&self, index: usize) -> usize {
        let mut start = 0;
        for (n, (_, rows)) in self.sections.iter().enumerate() {
            if index < start + rows.len() {
                return index + n + 1;
            }
            start += rows.len();
        }
        index
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.active = index;
        self.scroll.scroll_to_item(self.row_of(index));
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.flat().len() as isize;
        if count > 0 {
            self.select((self.active as isize + delta).rem_euclid(count) as usize, cx);
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.global_mut::<super::PaletteGlobal>().finder = None;
        window.close_dialog(cx);
    }

    fn on_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter_menu {
            self.filter_menu = false;
            self.query.clear();
            self.refresh(cx);
        } else {
            self.close(window, cx);
        }
    }

    fn toggle_filters(&mut self, cx: &mut Context<Self>) {
        if self.mode != Mode::Jump {
            return;
        }
        self.filter_menu = !self.filter_menu;
        self.query.clear();
        self.active = 0;
        self.refresh(cx);
    }

    fn run(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.flat().get(index).map(|r| (*r).clone()) else { return };
        match &row.target {
            Target::SeeMore => {
                self.see_more = true;
                return self.refresh(cx);
            }
            Target::AddFilter(filter) => {
                self.filters.push(filter.clone());
                self.filter_menu = false;
                self.query.clear();
                self.active = 0;
                return self.refresh(cx);
            }
            _ => {}
        }
        self.close(window, cx);
        let deck = self.deck.clone();
        match row.target {
            Target::Jump(run) => run_target(&deck, &run, &row.title, window, cx),
            Target::NewSession => deck.update(cx, |d, cx| {
                d.state.add_tab(None);
                d.save(cx);
                cx.notify();
            }),
            Target::File { path, line } => {
                let Some(root) = self.root.clone() else { return };
                super::remember_file(&deck, &root.to_string_lossy(), &path, cx);
                let file = root.join(&path);
                crate::skills::open_file_at(deck, root, file, line.unwrap_or(1).saturating_sub(1), window, cx);
            }
            Target::CreateWorktree => {
                let project_id = self.filters.iter().find_map(|f| match f {
                    Filter::Project { id, .. } => Some(id.clone()),
                    Filter::Host { .. } => None,
                });
                let prefill = crate::worktree::WorktreePrefill { project_id, source: None };
                crate::worktree::open_create_worktree(Some(prefill), window, cx);
            }
            Target::Agent(kind) => start_agent(&deck, kind, window, cx),
            Target::Url(url) => open_url(&deck, url, cx),
            Target::Command(command) => {
                if let Some(root) = self.root.clone() {
                    run_command(&deck, &root, &command, cx);
                }
            }
            Target::SeeMore | Target::AddFilter(_) => {}
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        let m = &ks.modifiers;
        match ks.key.as_str() {
            "backspace" if m.control => self.query.clear(),
            "backspace" if self.query.is_empty() && !self.filter_menu => {
                if self.filters.pop().is_none() {
                    return;
                }
            }
            "backspace" => {
                self.query.pop();
            }
            "v" if m.control || m.platform => {
                if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    self.query.push_str(&text.replace(['\r', '\n'], " "));
                }
            }
            _ if m.control || m.alt || m.platform => return,
            _ => match &ks.key_char {
                Some(ch) => self.query.push_str(ch),
                None => return,
            },
        }
        cx.stop_propagation();
        self.active = 0;
        self.scroll.scroll_to_item(0);
        self.refresh(cx);
        if matches!(self.mode, Mode::Files | Mode::NewTab) {
            self.search_files(cx);
        }
    }

    fn render_row(&self, index: usize, row: &Row, p: &Palette, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = index == self.active;
        let lead = match row.lead {
            Lead::Agent(kind) => ui::agent_mark(kind, 15.0, p).into_any_element(),
            Lead::Icon(icon) => div().text_color(hsla(p.subtle)).child(icon).into_any_element(),
        };
        let chip = row.chip.clone().filter(|c| !c.is_empty()).map(|chip| {
            div()
                .flex_shrink_0()
                .max_w(size(180.0))
                .truncate()
                .px_1p5()
                .rounded(size(4.0))
                .border_1()
                .border_color(hsla(p.line))
                .text_size(size(11.0))
                .text_color(hsla(p.subtle))
                .child(chip)
        });
        div()
            .id(index)
            .mx_1()
            .flex()
            .items_center()
            .gap_2p5()
            .h(size(36.0))
            .px_2()
            .rounded(size(6.0))
            .text_size(size(14.0))
            .when(selected, |s| s.bg(hsla(p.raised)).text_color(hsla(p.fg)))
            .when(!selected, |s| s.text_color(hsla(p.muted)))
            .child(div().flex().flex_shrink_0().size_4().items_center().justify_center().text_size(size(15.0)).child(lead))
            .child(div().flex_shrink_0().max_w(size(320.0)).truncate().child(row.title.clone()))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_xs()
                    .text_color(hsla(p.subtle))
                    .children(row.detail.clone()),
            )
            .children(chip)
            .children(row.status.map(|key| status_glyph(key, p)))
            .when(!row.keys.is_empty(), |el| el.child(ui::keys(row.keys, p)))
            .when_some(row.tooltip.clone(), |el, text| el.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx)))
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.active != index {
                    this.active = index;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.run(index, window, cx)))
    }

    fn placeholder(&self) -> &'static str {
        match self.mode {
            Mode::Jump if self.filter_menu => "Filter by project or host",
            Mode::Files => "Go to file (name, path parts, path:line)",
            Mode::Jump => "repo/worktree",
            Mode::NewTab => "Search open tabs, files, URLs, agents…",
            Mode::Commands => "Run project command…",
        }
    }
}

/// Spinner, question mark, check or dot in the status color.
fn status_glyph(key: ActivityKey, p: &Palette) -> AnyElement {
    let (icon, color) = match key {
        ActivityKey::Exited => (IconName::Circle, p.line_strong),
        ActivityKey::Status(status) => {
            let icon = match status {
                PaneStatus::Working => IconName::LoaderCircle,
                PaneStatus::Waiting => IconName::CircleQuestionMark,
                PaneStatus::Error => IconName::CircleAlert,
                PaneStatus::Done => IconName::Check,
                PaneStatus::Idle => IconName::Dot,
            };
            (icon, status_color(status, p))
        }
    };
    div().flex_shrink_0().text_size(size(14.0)).text_color(hsla(color)).child(icon).into_any_element()
}

/// Starts `kind` in the active project with its Settings defaults (the tab bar's quick add).
fn start_agent(deck: &Entity<Deck>, kind: AgentKind, window: &mut Window, cx: &mut App) {
    let (path, defaults) = {
        let d = deck.read(cx);
        (d.state.active_workspace().path.clone(), d.settings.agent(kind))
    };
    if path.is_empty() {
        return super::dispatch_named(deck, "AddAgent", "Add agent", window, cx);
    }
    crate::panes::spawn_agent(kind, &path, split_args(&defaults.args), defaults.worktree, None, cx);
}

/// Opens `url` in the current agent's browser panel (turned on if needed), else the system browser.
fn open_url(deck: &Entity<Deck>, url: String, cx: &mut App) {
    let pane = super::current_pane(deck.read(cx)).map(|(_, pane)| pane).filter(|id| {
        deck.read(cx).state.panes.get(id).is_some_and(|p| p.ssh.is_none() && crate::browser::supported(p.agent))
    });
    let Some(pane) = pane else {
        cx.open_url(&url);
        return;
    };
    deck.update(cx, |d, cx| {
        d.state.update_pane(&pane, |p| p.spec.browser = true);
        d.save(cx);
    });
    crate::panes::runtime::show_browser(&pane, cx);
    // The webview opens on a later frame: navigate once it exists (up to 3 s).
    cx.spawn(async move |cx| {
        for _ in 0..30 {
            cx.background_executor().timer(Duration::from_millis(100)).await;
            let done = cx.update(|cx| {
                if crate::browser::url(&pane, cx).is_err() {
                    return false;
                }
                if let Err(e) = crate::browser::go(&pane, &url, cx) {
                    eprintln!("finder: browser: {e}");
                }
                crate::layout::reveal_pane(&format!("browser:{pane}"), cx);
                true
            });
            if done {
                return;
            }
        }
    })
    .detach();
}

/// Runs `command` in `dir` in its own terminal pane of the active session. The same command
/// already running there is shown instead: a double activation opens one terminal.
fn run_command(deck: &Entity<Deck>, dir: &Path, command: &ProjectCommand, cx: &mut App) {
    let cwd = dir.to_string_lossy().into_owned();
    let args = shell_args(command, cfg!(windows));
    let running = deck
        .read(cx)
        .state
        .panes
        .values()
        .find(|p| p.agent == AgentKind::Shell && p.cwd == cwd && p.args == args && !p.exited)
        .map(|p| p.id.clone());
    close_wall(cx);
    let id = match running {
        Some(id) => id,
        None => {
            let (tab, font_size) = {
                let d = deck.read(cx);
                (d.state.active_tab_id.clone(), d.settings.font_size)
            };
            let mut spec = PaneSpec::new(AgentKind::Shell, command.display(), cwd);
            spec.args = args;
            spec.font_size = font_size;
            let id = spec.id.clone();
            crate::panes::runtime::create_pane(spec, &tab, false, cx);
            id
        }
    };
    // After the dock has picked up the new pane.
    cx.defer(move |cx| crate::layout::jump_to_pane(&id, cx));
}

impl Focusable for Finder {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Finder {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut index = 0;
        for (label, list) in self.sections.clone() {
            rows.push(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_xs()
                    .text_color(hsla(p.subtle))
                    .truncate()
                    .child(label)
                    .into_any_element(),
            );
            for row in &list {
                rows.push(self.render_row(index, row, &p, cx).into_any_element());
                index += 1;
            }
        }
        let empty = if self.loading {
            "Loading…"
        } else if self.message.is_some() {
            ""
        } else {
            "No results."
        };
        let chips: Vec<_> = self.filters.iter().enumerate().map(|(i, filter)| {
            div()
                .id(SharedString::from(format!("finder-filter-{i}")))
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_1()
                .h(size(22.0))
                .px_1p5()
                .rounded(size(4.0))
                .bg(hsla(p.raised))
                .text_xs()
                .text_color(hsla(p.fg))
                .cursor_pointer()
                .child(filter.label())
                .child(div().text_color(hsla(p.subtle)).child(IconName::X))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if i < this.filters.len() {
                        this.filters.remove(i);
                        this.refresh(cx);
                    }
                }))
        }).collect();
        let query = if self.query.is_empty() {
            div().text_color(hsla(p.subtle)).child(self.placeholder())
        } else {
            div().text_color(hsla(p.fg)).child(self.query.clone())
        };
        let caret = div().w(px(1.0)).h(size(16.0)).bg(hsla(p.fg));
        let max_list = window.viewport_size().height * 0.6;
        let is_jump = self.mode == Mode::Jump;
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|this, _: &FinderNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &FinderPrev, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &FinderRun, window, cx| this.run(this.active, window, cx)))
            .on_action(cx.listener(|this, _: &FinderClose, window, cx| this.on_escape(window, cx)))
            .on_action(cx.listener(|this, _: &FinderFilters, _, cx| this.toggle_filters(cx)))
            .on_action(cx.listener(|this, _: &FinderRow1, window, cx| this.run(0, window, cx)))
            .on_action(cx.listener(|this, _: &FinderRow2, window, cx| this.run(1, window, cx)))
            .on_action(cx.listener(|this, _: &FinderRow3, window, cx| this.run(2, window, cx)))
            .on_action(cx.listener(|this, _: &FinderRow4, window, cx| this.run(3, window, cx)))
            .on_action(cx.listener(|this, _: &FinderRow5, window, cx| this.run(4, window, cx)))
            .on_action(cx.listener(|this, _: &FinderRow6, window, cx| this.run(5, window, cx)))
            .flex()
            .flex_col()
            .w_full()
            .overflow_hidden()
            .text_size(size(14.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(size(44.0))
                    .px_3()
                    .border_b_1()
                    .border_color(hsla(p.line))
                    .child(div().text_size(size(16.0)).text_color(hsla(p.subtle)).child(IconName::Search))
                    .children(chips)
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .overflow_hidden()
                            .map(|s| if self.query.is_empty() { s.child(caret).child(query) } else { s.child(query).child(caret) }),
                    )
                    .when(is_jump, |el| el.child(ui::keys(&["Tab"], &p)).child(div().text_xs().text_color(hsla(p.subtle)).child("filter")))
                    .child(ui::keys(&["Esc"], &p)),
            )
            .children(self.message.clone().map(|message| {
                div().px_3().py_2().border_b_1().border_color(hsla(p.line)).text_xs().text_color(hsla(p.wait)).child(message)
            }))
            .child(
                div()
                    .id("finder-listbox")
                    .track_scroll(&self.scroll)
                    .overflow_y_scroll()
                    .max_h(max_list)
                    .py_1()
                    .when(index == 0 && !empty.is_empty(), |s| {
                        s.child(div().px_3().py_4().text_xs().text_color(hsla(p.subtle)).child(empty))
                    })
                    .children(rows),
            )
    }
}
