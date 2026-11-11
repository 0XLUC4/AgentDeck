//! UI state model (`src/store/deck.ts`): workspaces, panes, session tabs, archived sessions, toasts.
//! Plain data + methods; the app wraps [`DeckState`] in a GPUI entity and does the side effects.
//!
//! Layouts are opaque JSON (`serde_json::Value`): the layout module owns their shape. Old dockview
//! layouts from the Tauri build load fine and are simply replaced.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agents::AgentKind;
use crate::hooks_server::PaneStatus;

pub const FONT_SIZE_MIN: f32 = 8.0;
pub const FONT_SIZE_MAX: f32 = 32.0;
pub const FONT_SIZE_DEFAULT: f32 = 13.0;
pub const DEFAULT_WORKSPACE_NAME: &str = "Default";
/// Archived sessions kept; the oldest beyond this are dropped when a new one is archived.
pub const MAX_ARCHIVED: usize = 100;
const MAX_TOASTS: usize = 5;

pub type Layout = Option<Value>;

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// What is saved for a pane and enough to respawn it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneSpec {
    pub id: String,
    pub agent: AgentKind,
    pub title: String,
    pub cwd: String,
    pub args: Vec<String>,
    pub worktree: bool,
    #[serde(default)]
    pub completed_count: u32,
    #[serde(default = "default_font")]
    pub font_size: f32,
    /// A Claude pane wired to AgentDeck's MCP server.
    #[serde(default, skip_serializing_if = "is_false")]
    pub orchestrator: bool,
    /// Claude pane driving its own AgentDeck browser window.
    #[serde(default, skip_serializing_if = "is_false")]
    pub browser: bool,
    /// Claude conversation to continue on the next spawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_id: Option<String>,
    /// Color tag ("#rrggbb") to tell same-agent panes apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// PORT given to the agent (settings.basePort + n).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// "user@host:/dir": the agent runs on that machine through ssh (no hooks, no worktree).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<String>,
}

fn default_font() -> f32 {
    FONT_SIZE_DEFAULT
}

fn is_false(b: &bool) -> bool {
    !b
}

impl PaneSpec {
    pub fn new(agent: AgentKind, title: impl Into<String>, cwd: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            agent,
            title: title.into(),
            cwd: cwd.into(),
            args: Vec::new(),
            worktree: false,
            completed_count: 0,
            font_size: FONT_SIZE_DEFAULT,
            orchestrator: false,
            browser: false,
            resume_id: None,
            color: None,
            port: None,
            ssh: None,
        }
    }
}

/// A live pane: its spec plus runtime fields (not persisted).
#[derive(Debug, Clone, PartialEq)]
pub struct Pane {
    pub spec: PaneSpec,
    pub status: PaneStatus,
    pub last_action: Option<String>,
    /// Subject of the conversation (`text::topic`), shown as the title until the user names the pane.
    pub topic: Option<String>,
    pub task_started_at: Option<i64>,
    /// When `status` (or `exited`) last changed.
    pub status_since: i64,
    pub exited: bool,
    /// True once a hooks event arrived; otherwise status is derived from output.
    pub hooked: bool,
    /// Bumped on restart so the terminal view remounts on a fresh buffer.
    pub generation: u32,
}

impl std::ops::Deref for Pane {
    type Target = PaneSpec;
    fn deref(&self) -> &PaneSpec {
        &self.spec
    }
}

impl std::ops::DerefMut for Pane {
    fn deref_mut(&mut self) -> &mut PaneSpec {
        &mut self.spec
    }
}

pub fn clamp_font_size(size: f32) -> f32 {
    if size.is_finite() {
        size.round().clamp(FONT_SIZE_MIN, FONT_SIZE_MAX)
    } else {
        FONT_SIZE_DEFAULT
    }
}

pub fn new_pane(mut spec: PaneSpec, now: i64) -> Pane {
    spec.font_size = clamp_font_size(spec.font_size);
    Pane {
        spec,
        status: PaneStatus::Idle,
        last_action: None,
        topic: None,
        task_started_at: None,
        status_since: now,
        exited: false,
        hooked: false,
        generation: 0,
    }
}

/// Pure status transition: starts the task timer, counts completed tasks.
pub fn apply_status(pane: &Pane, status: PaneStatus, last_action: Option<String>, now: i64) -> Pane {
    let keeps_timer = matches!(status, PaneStatus::Working | PaneStatus::Waiting);
    let task_started_at = keeps_timer.then(|| pane.task_started_at.unwrap_or(now));
    let action = last_action.or_else(|| pane.last_action.clone());
    let mut next = pane.clone();
    if status != pane.status {
        next.status_since = now;
    }
    if status == PaneStatus::Done && pane.status != PaneStatus::Done {
        next.spec.completed_count += 1;
    }
    next.status = status;
    next.last_action = action;
    next.task_started_at = task_started_at;
    next
}

/// A linked project folder. Owns its sessions. Unlinking never touches files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    /// "" for the default workspace (agents then start in the last used folder).
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<WorkspaceGit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceGit {
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: String,
    pub name: String,
    pub workspace_id: String,
    pub pane_ids: Vec<String>,
    pub layout: Layout,
}

/// A closed (or saved) session tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSession {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub saved_at: i64,
    #[serde(default)]
    pub layout: Layout,
    #[serde(default)]
    pub panes: Vec<PaneSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub message: String,
    pub at: i64,
}

/// Toasts disappear after this (the app prunes with [`DeckState::prune_toasts`]).
pub const TOAST_MS: i64 = 6000;

/// Last segment of a Windows or POSIX path.
pub fn folder_name(path: &str) -> String {
    path.split(['\\', '/'])
        .filter(|s| !s.is_empty())
        .last()
        .unwrap_or("")
        .to_owned()
}

fn normalize_path(path: &str) -> String {
    path.trim_end_matches(['\\', '/'])
        .replace('/', "\\")
        .to_lowercase()
}

pub fn new_workspace(path: &str, name: Option<&str>) -> Workspace {
    let fallback = folder_name(path);
    let name = name
        .map(str::to_owned)
        .unwrap_or(if fallback.is_empty() {
            DEFAULT_WORKSPACE_NAME.to_owned()
        } else {
            fallback
        });
    Workspace {
        id: new_id(),
        name,
        path: path.to_owned(),
        git: None,
    }
}

pub fn new_tab(name: &str, workspace_id: &str, layout: Layout) -> Tab {
    Tab {
        id: new_id(),
        name: name.to_owned(),
        workspace_id: workspace_id.to_owned(),
        pane_ids: Vec::new(),
        layout,
    }
}

/// Saved form of a tab: its layout plus the spec of each live pane.
pub fn tab_to_session(tab: &Tab, panes: &HashMap<String, Pane>, now: i64) -> SavedSession {
    SavedSession {
        id: tab.id.clone(),
        name: tab.name.clone(),
        workspace_id: tab.workspace_id.clone(),
        saved_at: now,
        layout: tab.layout.clone(),
        panes: tab
            .pane_ids
            .iter()
            .filter_map(|id| panes.get(id).map(|p| p.spec.clone()))
            .collect(),
    }
}

/// The status a session shows in lists: its most urgent pane's.
pub fn session_status(pane_ids: &[String], panes: &HashMap<String, Pane>) -> Option<PaneStatus> {
    use PaneStatus::*;
    let has = |s| pane_ids.iter().any(|id| panes.get(id).is_some_and(|p| p.status == s));
    [Waiting, Error, Working, Done, Idle].into_iter().find(|s| has(*s))
}

/// Activity panel grouping: a stopped process shows as "exited" unless it failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActivityKey {
    Status(PaneStatus),
    Exited,
}

pub const ACTIVITY_ORDER: [ActivityKey; 6] = [
    ActivityKey::Status(PaneStatus::Waiting),
    ActivityKey::Status(PaneStatus::Error),
    ActivityKey::Status(PaneStatus::Done),
    ActivityKey::Status(PaneStatus::Working),
    ActivityKey::Status(PaneStatus::Idle),
    ActivityKey::Exited,
];

pub fn activity_key(pane: &Pane) -> ActivityKey {
    if pane.exited && pane.status != PaneStatus::Error {
        ActivityKey::Exited
    } else {
        ActivityKey::Status(pane.status)
    }
}

impl ActivityKey {
    /// "waiting", ..., "exited" (orchestrator tool results use these strings).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exited => "exited",
            Self::Status(PaneStatus::Idle) => "idle",
            Self::Status(PaneStatus::Working) => "working",
            Self::Status(PaneStatus::Waiting) => "waiting",
            Self::Status(PaneStatus::Done) => "done",
            Self::Status(PaneStatus::Error) => "error",
        }
    }
}

/// Every live pane in the state.
#[derive(Debug, Clone)]
pub struct DeckState {
    pub workspaces: Vec<Workspace>,
    pub panes: HashMap<String, Pane>,
    pub tabs: Vec<Tab>,
    pub active_tab_id: String,
    pub active_pane_id: Option<String>,
    pub archived: Vec<SavedSession>,
    pub last_cwd: String,
    pub toasts: Vec<Toast>,
    next_toast: u64,
}

impl Default for DeckState {
    fn default() -> Self {
        let ws = new_workspace("", None);
        let tab = new_tab("Session 1", &ws.id, None);
        Self {
            active_tab_id: tab.id.clone(),
            workspaces: vec![ws],
            panes: HashMap::new(),
            tabs: vec![tab],
            active_pane_id: None,
            archived: Vec::new(),
            last_cwd: String::new(),
            toasts: Vec::new(),
            next_toast: 0,
        }
    }
}

/// A session can be archived while its workspace has another open one.
pub fn can_archive(tabs: &[Tab], tab_id: &str) -> bool {
    let Some(ws) = tabs.iter().find(|t| t.id == tab_id).map(|t| &t.workspace_id) else {
        return false;
    };
    tabs.iter().filter(|t| &t.workspace_id == ws).count() > 1
}

impl DeckState {
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == self.active_tab_id)
    }

    /// The active workspace is the one owning the active tab.
    pub fn active_workspace(&self) -> &Workspace {
        let ws_id = self.active_tab().map(|t| t.workspace_id.as_str());
        self.workspaces
            .iter()
            .find(|w| Some(w.id.as_str()) == ws_id)
            .unwrap_or(&self.workspaces[0])
    }

    /// Workspace already linked to this folder (case- and slash-insensitive, Windows style).
    pub fn workspace_for_path(&self, path: &str) -> Option<&Workspace> {
        if path.is_empty() {
            return None;
        }
        let wanted = normalize_path(path);
        self.workspaces
            .iter()
            .find(|w| !w.path.is_empty() && normalize_path(&w.path) == wanted)
    }

    /// Every live pane of every workspace, grouped by ACTIVITY_ORDER, longest in status first.
    pub fn activity_groups(&self) -> Vec<(ActivityKey, Vec<(&Pane, &Tab, &Workspace)>)> {
        let mut items = Vec::new();
        for tab in &self.tabs {
            let Some(ws) = self.workspaces.iter().find(|w| w.id == tab.workspace_id) else {
                continue;
            };
            for id in &tab.pane_ids {
                if let Some(p) = self.panes.get(id) {
                    items.push((p, tab, ws));
                }
            }
        }
        ACTIVITY_ORDER
            .into_iter()
            .filter_map(|key| {
                let mut group: Vec<_> = items
                    .iter()
                    .filter(|(p, _, _)| activity_key(p) == key)
                    .copied()
                    .collect();
                group.sort_by_key(|(p, _, _)| p.status_since);
                (!group.is_empty()).then_some((key, group))
            })
            .collect()
    }

    /// Applies `f` to a pane; returns false when the pane is unknown.
    pub fn update_pane(&mut self, id: &str, f: impl FnOnce(&mut Pane)) -> bool {
        self.panes.get_mut(id).map(f).is_some()
    }

    /// Idempotent: a pane is never listed twice in its tab.
    pub fn add_pane(&mut self, pane: Pane, tab_id: &str) {
        let id = pane.id.clone();
        self.panes.insert(id.clone(), pane);
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id)
            && !tab.pane_ids.contains(&id)
        {
            tab.pane_ids.push(id);
        }
    }

    pub fn remove_pane(&mut self, id: &str) {
        self.panes.remove(id);
        for tab in &mut self.tabs {
            tab.pane_ids.retain(|p| p != id);
        }
        if self.active_pane_id.as_deref() == Some(id) {
            self.active_pane_id = None;
        }
    }

    /// Returns true when the pane changed.
    pub fn set_status(&mut self, id: &str, status: PaneStatus, last_action: Option<String>, now: i64) -> bool {
        let Some(pane) = self.panes.get(id) else {
            return false;
        };
        let next = apply_status(pane, status, last_action, now);
        let changed = next != *pane;
        self.panes.insert(id.to_owned(), next);
        changed
    }

    /// "Seen": a finished pane goes back to idle.
    pub fn acknowledge(&mut self, id: &str, now: i64) {
        if self.panes.get(id).is_some_and(|p| p.status == PaneStatus::Done) {
            self.set_status(id, PaneStatus::Idle, None, now);
        }
    }

    pub fn rename(&mut self, id: &str, title: &str) {
        self.update_pane(id, |p| p.spec.title = title.to_owned());
    }

    /// `delta: None` resets to the default size.
    pub fn zoom(&mut self, id: &str, delta: Option<f32>) {
        self.update_pane(id, |p| {
            p.spec.font_size = match delta {
                None => FONT_SIZE_DEFAULT,
                Some(d) => clamp_font_size(p.spec.font_size + d),
            }
        });
    }

    fn sessions_of(&self, ws: &str) -> usize {
        self.archived.iter().filter(|s| s.workspace_id == ws).count()
    }

    /// Opens a new session tab (in the active workspace by default) and shows it.
    pub fn add_tab(&mut self, tab: Option<Tab>) -> String {
        let tab = tab.unwrap_or_else(|| {
            let ws = self.active_workspace().id.clone();
            let count = self.tabs.iter().filter(|t| t.workspace_id == ws).count() + self.sessions_of(&ws);
            new_tab(&format!("Session {}", count + 1), &ws, None)
        });
        let id = tab.id.clone();
        self.tabs.push(tab);
        self.active_tab_id = id.clone();
        id
    }

    fn pick_active(&self, prefer_workspace: Option<&str>) -> String {
        if self.tabs.iter().any(|t| t.id == self.active_tab_id) {
            return self.active_tab_id.clone();
        }
        self.tabs
            .iter()
            .find(|t| Some(t.workspace_id.as_str()) == prefer_workspace)
            .or(self.tabs.first())
            .map(|t| t.id.clone())
            .unwrap_or_default()
    }

    /// Removes the tab only. Every workspace keeps at least one open tab.
    pub fn remove_tab(&mut self, id: &str) {
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let removed = self.tabs.remove(pos);
        if !self.tabs.iter().any(|t| t.workspace_id == removed.workspace_id) {
            self.tabs.push(new_tab("Session 1", &removed.workspace_id, None));
        }
        self.active_tab_id = self.pick_active(Some(&removed.workspace_id));
    }

    /// Archives a session (never the last open one of its workspace). Returns its pane ids to kill.
    pub fn archive_tab(&mut self, id: &str, now: i64) -> Vec<String> {
        if !can_archive(&self.tabs, id) {
            return Vec::new();
        }
        let Some(tab) = self.tabs.iter().find(|t| t.id == id).cloned() else {
            return Vec::new();
        };
        self.archived.retain(|s| s.id != id);
        self.archived.insert(0, tab_to_session(&tab, &self.panes, now));
        self.archived.truncate(MAX_ARCHIVED);
        for pane_id in &tab.pane_ids {
            self.remove_pane(pane_id);
        }
        self.remove_tab(id);
        tab.pane_ids
    }

    /// Removes a session from the archive and opens an empty tab with its layout. Returns
    /// (tab id, pane specs to respawn).
    pub fn unarchive(&mut self, id: &str) -> Option<(String, Vec<PaneSpec>)> {
        let pos = self.archived.iter().position(|s| s.id == id)?;
        let session = self.archived.remove(pos);
        let workspace_id = if self.workspaces.iter().any(|w| w.id == session.workspace_id) {
            session.workspace_id.clone()
        } else {
            self.active_workspace().id.clone()
        };
        let tab_id = self.add_tab(Some(Tab {
            id: session.id,
            name: session.name,
            workspace_id,
            pane_ids: Vec::new(),
            layout: session.layout,
        }));
        Some((tab_id, session.panes))
    }

    pub fn delete_archived(&mut self, id: &str) {
        self.archived.retain(|s| s.id != id);
    }

    pub fn rename_tab(&mut self, id: &str, name: &str) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
            t.name = name.to_owned();
        }
    }

    pub fn set_tab_layout(&mut self, id: &str, layout: Value) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
            t.layout = Some(layout);
        }
    }

    /// Links a folder as a new workspace with one empty session, and shows it.
    pub fn link_workspace(&mut self, path: &str, name: Option<&str>, git: Option<WorkspaceGit>) -> Workspace {
        let mut ws = new_workspace(path, name);
        ws.git = git;
        self.workspaces.push(ws.clone());
        self.add_tab(Some(new_tab("Session 1", &ws.id, None)));
        ws
    }

    pub fn rename_workspace(&mut self, id: &str, name: &str) {
        if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == id) {
            w.name = name.to_owned();
        }
    }

    pub fn select_workspace(&mut self, id: &str) {
        if self.active_workspace().id == id {
            return;
        }
        if let Some(tab) = self.tabs.iter().find(|t| t.workspace_id == id) {
            self.active_tab_id = tab.id.clone();
        }
    }

    /// Forgets a workspace with its open and archived sessions. Always keeps one workspace.
    /// Returns the pane ids to kill.
    pub fn unlink_workspace(&mut self, id: &str) -> Vec<String> {
        if !self.workspaces.iter().any(|w| w.id == id) {
            return Vec::new();
        }
        let pane_ids: Vec<String> = self
            .tabs
            .iter()
            .filter(|t| t.workspace_id == id)
            .flat_map(|t| t.pane_ids.clone())
            .collect();
        self.workspaces.retain(|w| w.id != id);
        if self.workspaces.is_empty() {
            self.workspaces.push(new_workspace("", None));
        }
        self.tabs.retain(|t| t.workspace_id != id);
        if self.tabs.is_empty() {
            let ws = self.workspaces[0].id.clone();
            self.tabs.push(new_tab("Session 1", &ws, None));
        }
        self.panes.retain(|pid, _| !pane_ids.contains(pid));
        self.archived.retain(|s| s.workspace_id != id);
        self.active_tab_id = self.pick_active(None);
        if self.active_pane_id.as_ref().is_some_and(|p| pane_ids.contains(p)) {
            self.active_pane_id = None;
        }
        pane_ids
    }

    /// Adds a toast (a burst keeps the last 5).
    pub fn toast(&mut self, message: impl Into<String>, now: i64) -> u64 {
        self.next_toast += 1;
        self.toasts.push(Toast {
            id: self.next_toast,
            message: message.into(),
            at: now,
        });
        let extra = self.toasts.len().saturating_sub(MAX_TOASTS);
        self.toasts.drain(..extra);
        self.next_toast
    }

    /// Drops toasts older than [`TOAST_MS`]; returns true when one was removed.
    pub fn prune_toasts(&mut self, now: i64) -> bool {
        let before = self.toasts.len();
        self.toasts.retain(|t| now - t.at < TOAST_MS);
        before != self.toasts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane() -> Pane {
        let mut spec = PaneSpec::new(AgentKind::Claude, "t", "/");
        spec.id = "p1".into();
        new_pane(spec, 0)
    }

    fn state() -> DeckState {
        let w1 = Workspace {
            id: "w1".into(),
            name: "Default".into(),
            path: String::new(),
            git: None,
        };
        let tab = |id: &str| Tab {
            id: id.into(),
            name: "S".into(),
            workspace_id: "w1".into(),
            pane_ids: vec![],
            layout: None,
        };
        DeckState {
            workspaces: vec![w1],
            tabs: vec![tab("t1"), tab("t2")],
            active_tab_id: "t1".into(),
            ..DeckState::default()
        }
    }

    #[test]
    fn apply_status_timer_and_count() {
        let working = apply_status(&pane(), PaneStatus::Working, Some("Edit a.ts".into()), 100);
        assert_eq!(working.task_started_at, Some(100));
        let waiting = apply_status(&working, PaneStatus::Waiting, None, 500);
        assert_eq!(waiting.task_started_at, Some(100));
        assert_eq!(waiting.last_action.as_deref(), Some("Edit a.ts"));
        let done = apply_status(&waiting, PaneStatus::Done, None, 9);
        assert_eq!(done.completed_count, 1);
        assert_eq!(done.task_started_at, None);
        assert_eq!(apply_status(&done, PaneStatus::Done, None, 10).completed_count, 1);
        let mut w = pane();
        w.status = PaneStatus::Working;
        assert_eq!(apply_status(&w, PaneStatus::Error, None, 1).completed_count, 0);
    }

    #[test]
    fn panes_and_tabs() {
        let mut s = state();
        s.add_pane(pane(), "t1");
        s.add_pane(pane(), "t1");
        assert_eq!(s.tabs[0].pane_ids, ["p1"]);
        s.set_status("p1", PaneStatus::Working, None, 5);
        assert_eq!(s.panes["p1"].task_started_at, Some(5));
        s.remove_pane("p1");
        assert!(s.panes.is_empty() && s.tabs[0].pane_ids.is_empty());
        s.remove_tab("t1");
        s.remove_tab("t2");
        assert_eq!(s.tabs.len(), 1);
        assert_eq!(s.active_tab_id, s.tabs[0].id);
    }

    #[test]
    fn archive_and_reopen() {
        let mut s = state();
        let mut p = pane();
        p.spec.font_size = 18.0;
        s.add_pane(p, "t1");
        assert_eq!(s.archive_tab("t1", 42), ["p1"]);
        assert!(s.panes.is_empty());
        assert_eq!(s.archived[0].saved_at, 42);
        assert_eq!(s.archived[0].panes[0].font_size, 18.0);
        assert!(s.archive_tab("t2", 1).is_empty(), "last session of a workspace stays");
        let (tab_id, panes) = s.unarchive("t1").unwrap();
        assert_eq!(panes.len(), 1);
        assert_eq!(s.active_tab_id, tab_id);
        assert!(s.unarchive("t1").is_none());
    }

    #[test]
    fn zoom_and_font() {
        assert_eq!(clamp_font_size(2.0), FONT_SIZE_MIN);
        assert_eq!(clamp_font_size(99.0), FONT_SIZE_MAX);
        assert_eq!(clamp_font_size(14.6), 15.0);
        assert_eq!(clamp_font_size(f32::NAN), FONT_SIZE_DEFAULT);
        let mut s = state();
        let mut p = pane();
        p.spec.font_size = 32.0;
        s.add_pane(p, "t1");
        s.zoom("p1", Some(1.0));
        assert_eq!(s.panes["p1"].font_size, 32.0);
        s.zoom("p1", None);
        assert_eq!(s.panes["p1"].font_size, FONT_SIZE_DEFAULT);
    }

    #[test]
    fn workspaces() {
        let mut s = state();
        let ws = s.link_workspace("D:\\Proj\\", None, None);
        assert_eq!(ws.name, "Proj");
        assert_eq!(s.active_workspace().id, ws.id);
        assert_eq!(s.workspace_for_path("d:/proj").map(|w| &w.id), Some(&ws.id));
        assert_eq!(folder_name("/a/b/"), "b");
        s.unlink_workspace(&ws.id);
        assert_eq!(s.workspaces.len(), 1);
        assert!(s.tabs.iter().all(|t| t.workspace_id == "w1"));
    }

    #[test]
    fn statuses_and_activity() {
        let mut s = state();
        let mut a = pane();
        a.status = PaneStatus::Done;
        s.add_pane(a, "t1");
        let mut b = pane();
        b.spec.id = "p2".into();
        b.status = PaneStatus::Waiting;
        s.add_pane(b, "t1");
        assert_eq!(session_status(&s.tabs[0].pane_ids, &s.panes), Some(PaneStatus::Waiting));
        let groups = s.activity_groups();
        assert_eq!(groups[0].0, ActivityKey::Status(PaneStatus::Waiting));
        assert_eq!(groups[1].0, ActivityKey::Status(PaneStatus::Done));
    }

    #[test]
    fn toasts_cap_and_prune() {
        let mut s = state();
        for i in 0..7 {
            s.toast(format!("m{i}"), 0);
        }
        assert_eq!(s.toasts.len(), 5);
        assert!(s.prune_toasts(TOAST_MS));
        assert!(s.toasts.is_empty());
    }
}
