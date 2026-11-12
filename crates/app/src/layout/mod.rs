//! Window shell: title bar, sidebar (workspaces + sessions), session tab bar, pane area with
//! splits/tabs (dock), activity panel, toasts, detached pane windows.
//! Original: src/app/{TitleBar,Sidebar,TabBar,Deck,Activity,Toasts}.tsx, src/App.tsx (shell +
//! shortcuts), src/components/Detached/{DetachedPane.tsx,host.ts}, src/lib/{gridLayout,detach}.ts.
//!
//! Other modules reach the dock through the free functions below (`reveal_pane`, `jump_to_pane`,
//! `toggle_maximize_pane`, `arrange_in_grid`, `toggle_detached`, `is_detached`), the entry points
//! the React code exported from Deck.tsx and host.ts. Pane lifecycle (spawn, close, restart,
//! archive) stays in `panes::runtime`.

mod activity;
mod chrome;
mod detached;
pub mod dock;
mod dock_view;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::{ActivityKey, activity_key};
use agentdeck_core::logic::text::{agent_name, now_ms};
use agentdeck_core::persist::SAVE_DEBOUNCE_MS;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::browser::BrowserPanel;
use crate::panes::runtime;
use crate::panes::{AgentPicker, PaneHeader, PaneHeaderEvent, PermissionBanner};
use crate::state::Deck;
use crate::system::UpdateButton;
use crate::terminal::TerminalView;
use crate::theme::{hsla, palette, ui};
use dock::Dock;

actions!(
    agentdeck,
    [
        FocusPane1,
        FocusPane2,
        FocusPane3,
        FocusPane4,
        FocusPane5,
        FocusPane6,
        FocusPane7,
        FocusPane8,
        FocusPane9,
        ToggleActivity,
        NextWaiting,
        RenameSession,
        ToggleSidebar,
        ArrangeInGrid,
        ToggleMaximize,
        AddAgent
    ]
);

/// Marks the key bindings as registered (`init` may run from `main` and from `DeckView::new`).
struct LayoutReady;
impl Global for LayoutReady {}

/// The main window's shell, for the free functions other modules call.
struct Shell(WeakEntity<DeckView>);
impl Global for Shell {}

/// Registers the layout shortcuts (App.tsx): Ctrl+1..9, Ctrl+Shift+A, Ctrl+Shift+J, F2 on the tab bar.
pub fn init(cx: &mut App) {
    if cx.has_global::<LayoutReady>() {
        return;
    }
    cx.set_global(LayoutReady);
    cx.bind_keys([
        KeyBinding::new("ctrl-1", FocusPane1, None),
        KeyBinding::new("ctrl-2", FocusPane2, None),
        KeyBinding::new("ctrl-3", FocusPane3, None),
        KeyBinding::new("ctrl-4", FocusPane4, None),
        KeyBinding::new("ctrl-5", FocusPane5, None),
        KeyBinding::new("ctrl-6", FocusPane6, None),
        KeyBinding::new("ctrl-7", FocusPane7, None),
        KeyBinding::new("ctrl-8", FocusPane8, None),
        KeyBinding::new("ctrl-9", FocusPane9, None),
        KeyBinding::new("ctrl-shift-a", ToggleActivity, None),
        KeyBinding::new("ctrl-shift-j", NextWaiting, None),
        KeyBinding::new("f2", RenameSession, Some("SessionTabs")),
    ]);
    // Global fallbacks: actions dispatched from dialogs (palette) or with nothing focused never reach
    // DeckView's own listeners, which sit below gpui-kit's Root.
    cx.on_action(|_: &ToggleSidebar, cx| {
        with_shell(cx, |this, cx| this.toggle_sidebar(cx))
    });
    cx.on_action(|_: &ToggleActivity, cx| {
        with_shell(cx, |this, cx| this.toggle_activity(cx))
    });
    cx.on_action(|_: &NextWaiting, cx| with_shell(cx, |this, cx| this.next_waiting(cx)));
    cx.on_action(|_: &AddAgent, cx| with_shell_window(cx, |this, window, cx| this.open_picker(window, cx)));
}

/// Like [`with_shell`], with the main window, deferred: the action may arrive during a window update.
fn with_shell_window(cx: &mut App, f: impl FnOnce(&mut DeckView, &mut Window, &mut Context<DeckView>) + 'static) {
    let Some(shell) = cx.try_global::<Shell>().map(|s| s.0.clone()) else { return };
    let Some(handle) = crate::system::main_window(cx) else { return };
    cx.defer(move |cx| {
        let result = handle.update(cx, |_, window, cx| shell.update(cx, |this, cx| f(this, window, cx)));
        if let Err(e) = result.and_then(|r| r) {
            eprintln!("layout: main window gone: {e}");
        }
    });
}

/// Runs `f` on the main window's shell, if it exists.
fn with_shell(cx: &mut App, f: impl FnOnce(&mut DeckView, &mut Context<DeckView>)) {
    let Some(shell) = cx.try_global::<Shell>().map(|s| s.0.clone()) else { return };
    if let Err(e) = shell.update(cx, f) {
        eprintln!("layout: shell gone: {e}");
    }
}

/// Makes a pane visible in its session: leaves another pane's maximized state, selects it if stacked.
/// Also takes a browser panel id (`browser:<pane id>`).
pub fn reveal_pane(pane_id: &str, cx: &mut App) {
    with_shell(cx, |this, cx| this.reveal(pane_id, cx));
}

/// Shows the pane's session, reveals the pane and makes it the active one (Wall, Activity, Ctrl+N).
pub fn jump_to_pane(pane_id: &str, cx: &mut App) {
    with_shell(cx, |this, cx| this.jump(pane_id, cx));
}

/// Maximizes a pane in its session, or restores it if already maximized.
pub fn toggle_maximize_pane(pane_id: &str, cx: &mut App) {
    with_shell(cx, |this, cx| this.toggle_maximize(pane_id, cx));
}

/// Re-lays the session's panes on the auto grid (store order) and saves the layout.
pub fn arrange_in_grid(tab_id: &str, cx: &mut App) {
    with_shell(cx, |this, cx| this.arrange(tab_id, cx));
}

/// Opens the pane in its own window, or reattaches it when already detached.
pub fn toggle_detached(pane_id: &str, cx: &mut App) {
    with_shell(cx, |this, cx| this.toggle_detached(pane_id, cx));
}

pub fn is_detached(pane_id: &str, cx: &App) -> bool {
    cx.try_global::<Shell>()
        .and_then(|s| s.0.upgrade())
        .is_some_and(|shell| shell.read(cx).detached.contains_key(pane_id))
}

/// Root view of the main window.
pub struct DeckView {
    deck: Entity<Deck>,
    terminals: HashMap<String, Entity<TerminalView>>,
    headers: HashMap<String, Entity<PaneHeader>>,
    browsers: HashMap<String, Entity<BrowserPanel>>,
    /// Header and browser event subscriptions, by view id.
    view_subs: HashMap<String, Subscription>,
    /// One dock per session tab, created from the saved layout on first sight.
    docks: HashMap<String, Dock>,
    /// What the last `sync_docks` applied: (tab id, wanted panel ids).
    last_sync: Vec<(String, Vec<String>)>,
    /// Panes shown in their own window. Not persisted: panes reopen attached.
    detached: HashMap<String, AnyWindowHandle>,
    /// Tabs whose layout changed since the last save.
    dirty: HashSet<String>,
    save_task: Option<Task<()>>,
    sidebar_open: bool,
    activity_open: bool,
    activity: activity::ActivityState,
    /// Session tab being renamed, with its text field.
    renaming: Option<(String, Entity<gpui_kit::component::input::InputState>, Subscription)>,
    /// Last title set on the main window ("AgentDeck (2)" while agents wait).
    window_title: String,
    /// Which add-agent button ("tabbar" or "empty") has its menu open.
    add_menu_open: Option<&'static str>,
    /// Which add-agent arrow the mouse is over.
    add_arrow_hover: Option<&'static str>,
    collapsed_workspaces: HashSet<String>,
    picker: Option<(Entity<AgentPicker>, Subscription)>,
    /// Git panel opened from a pane header (it draws itself as a window overlay).
    git_panel: Option<(Entity<crate::git::GitPanel>, Subscription)>,
    permission_banner: Entity<PermissionBanner>,
    update_button: Entity<UpdateButton>,
    focus: FocusHandle,
    tabs_focus: FocusHandle,
    /// Today's usage, total and per agent (status bar), refreshed every minute. None until the first read.
    usage_today: Option<agentdeck_core::usage::UsageSummary>,
    _subs: Vec<Subscription>,
    _ticker: Task<()>,
    _usage: Task<()>,
}

impl DeckView {
    pub fn new(deck: Entity<Deck>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        init(cx);
        cx.set_global(Shell(cx.entity().downgrade()));
        let observe = cx.observe(&deck, |this, _, cx| {
            this.sync_docks(cx);
            cx.notify();
        });
        let settings = &deck.read(cx).settings;
        let (sidebar_open, activity_open) = (settings.sidebar_open, settings.activity_open);
        // The rail shows one panel at a time.
        let activity_open = activity_open && !sidebar_open;
        let permission_banner = cx.new(|cx| PermissionBanner::new(deck.clone(), cx));
        let update_button = cx.new(UpdateButton::new);
        let mut this = Self {
            deck,
            terminals: HashMap::new(),
            headers: HashMap::new(),
            browsers: HashMap::new(),
            view_subs: HashMap::new(),
            docks: HashMap::new(),
            last_sync: Vec::new(),
            detached: HashMap::new(),
            dirty: HashSet::new(),
            save_task: None,
            sidebar_open,
            activity_open,
            activity: Default::default(),
            renaming: None,
            window_title: String::new(),
            add_menu_open: None,
            add_arrow_hover: None,
            collapsed_workspaces: HashSet::new(),
            picker: None,
            git_panel: None,
            permission_banner,
            update_button,
            focus: cx.focus_handle(),
            tabs_focus: cx.focus_handle(),
            usage_today: None,
            _subs: vec![observe],
            _ticker: ticker(cx),
            _usage: usage_ticker(cx),
        };
        this.sync_docks(cx);
        this
    }

    pub fn deck(&self) -> &Entity<Deck> {
        &self.deck
    }

    /// The terminal view of a pane, created on first use. It gets the deck now: the main window's
    /// root is gpui-kit's `Root`, and a detached window's root is not a `DeckView` either.
    pub fn terminal(&mut self, pane_id: &str, cx: &mut Context<Self>) -> Entity<TerminalView> {
        let deck = self.deck.clone();
        self.terminals
            .entry(pane_id.to_owned())
            .or_insert_with(|| {
                cx.new(|cx| {
                    let mut view = TerminalView::new(pane_id);
                    view.deck = Some(deck);
                    // Parse output now: a pane never shown must still answer its program's terminal queries.
                    view.start_output(cx);
                    view
                })
            })
            .clone()
    }

    /// The browser panel of a pane, created on first use.
    fn browser(&mut self, pane_id: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<BrowserPanel> {
        if let Some(browser) = self.browsers.get(pane_id) {
            return browser.clone();
        }
        let deck = self.deck.clone();
        let browser = cx.new(|cx| BrowserPanel::new(deck, pane_id, window, cx));
        self.browsers.insert(pane_id.to_owned(), browser.clone());
        browser
    }

    /// Pane headers (panes module) for every pane, wired to the dock and the feature dialogs.
    fn ensure_headers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<String> = self
            .deck
            .read(cx)
            .state
            .panes
            .keys()
            .filter(|id| !self.headers.contains_key(*id))
            .cloned()
            .collect();
        for id in ids {
            let deck = self.deck.clone();
            let diff = cx.new(|cx| crate::git::DiffChip::new(deck.clone(), id.clone(), cx));
            let usage = cx.new(|cx| crate::history::UsageChip::new(deck.clone(), id.clone(), cx));
            let header = cx.new(|cx| {
                let mut header = PaneHeader::new(deck, id.clone(), cx);
                header.extras = vec![diff.into(), usage.into()];
                header
            });
            let pane_id = id.clone();
            let sub = cx.subscribe_in(&header, window, move |this, _, event: &PaneHeaderEvent, window, cx| {
                this.on_header_event(&pane_id, event, window, cx);
            });
            self.view_subs.insert(id.clone(), sub);
            self.headers.insert(id, header);
        }
    }

    fn on_header_event(&mut self, pane_id: &str, event: &PaneHeaderEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PaneHeaderEvent::ToggleMaximize => self.toggle_maximize(pane_id, cx),
            PaneHeaderEvent::ToggleDetach => self.toggle_detached(pane_id, cx),
            PaneHeaderEvent::ShowBrowser => self.reveal(&format!("browser:{pane_id}"), cx),
            PaneHeaderEvent::CloseBrowser => {
                let id = pane_id.to_owned();
                self.deck.update(cx, |deck, cx| {
                    deck.state.update_pane(&id, |p| p.spec.browser = false);
                    deck.save(cx);
                    cx.notify();
                });
                crate::browser::close(pane_id, cx);
            }
            PaneHeaderEvent::OpenSkills => crate::skills::open_skills(self.deck.clone(), pane_id, window, cx),
            PaneHeaderEvent::OpenNotes => crate::skills::open_notes(self.deck.clone(), pane_id, window, cx),
            PaneHeaderEvent::OpenGit => self.open_git(pane_id, window, cx),
        }
    }

    /// Opens the Git panel of a pane (its header button, or the rail for the active pane).
    pub(super) fn open_git(&mut self, pane_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let deck = self.deck.clone();
        let pane_id = pane_id.to_owned();
        let panel = cx.new(|cx| crate::git::GitPanel::new(deck, pane_id, window, cx));
        let sub = cx.subscribe(&panel, |this, _, _: &DismissEvent, cx| {
            this.git_panel = None;
            cx.notify();
        });
        self.git_panel = Some((panel, sub));
        cx.notify();
    }

    /// Projects and Activity share the panel next to the rail: opening one closes the other.
    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        // Behind a page the panel is hidden: asking for it closes the page and shows it.
        self.sidebar_open = crate::skills::close_page(cx) || !self.sidebar_open;
        self.activity_open &= !self.sidebar_open;
        cx.notify();
    }

    pub(super) fn toggle_activity(&mut self, cx: &mut Context<Self>) {
        self.activity_open = crate::skills::close_page(cx) || !self.activity_open;
        self.sidebar_open &= !self.activity_open;
        cx.notify();
    }

    /// Deck.tsx sync: every tab gets a dock holding its panes (and their browsers); panels of
    /// closed panes go away. With auto grid on, a change re-grids the session.
    fn sync_docks(&mut self, cx: &mut Context<Self>) {
        let deck = self.deck.read(cx);
        let auto_grid = deck.settings.auto_grid;
        let mut wanted_by_tab = Vec::new();
        for tab in &deck.state.tabs {
            let wanted: Vec<String> = tab
                .pane_ids
                .iter()
                .filter(|id| deck.state.panes.contains_key(*id))
                .flat_map(|id| {
                    let browser = deck.state.panes.get(id).is_some_and(|p| p.browser);
                    std::iter::once(id.clone()).chain(browser.then(|| format!("browser:{id}")))
                })
                .collect();
            wanted_by_tab.push((tab.id.clone(), wanted));
        }
        // Runs on every deck notify (clock, hooks, toasts): nothing to do unless tabs or panes changed.
        // The pane count covers a pane outside every tab: after a sync there is one terminal per pane.
        if wanted_by_tab == self.last_sync && self.terminals.len() == deck.state.panes.len() {
            return;
        }
        // The saved layout is only read to create a dock.
        let layouts: HashMap<String, serde_json::Value> = deck
            .state
            .tabs
            .iter()
            .filter(|tab| !self.docks.contains_key(&tab.id))
            .filter_map(|tab| Some((tab.id.clone(), tab.layout.clone()?)))
            .collect();
        let alive: HashSet<String> = deck.state.panes.keys().cloned().collect();
        let with_browser: HashSet<String> = deck
            .state
            .panes
            .values()
            .filter(|p| p.browser)
            .map(|p| p.id.clone())
            .collect();

        let tab_ids: HashSet<String> = wanted_by_tab.iter().map(|t| t.0.clone()).collect();
        self.docks.retain(|id, _| tab_ids.contains(id));
        for (tab_id, wanted) in &wanted_by_tab {
            let dock = self
                .docks
                .entry(tab_id.clone())
                .or_insert_with(|| layouts.get(tab_id).and_then(Dock::from_json).unwrap_or_default());
            if dock.sync(wanted) {
                // Only when a pane was added or removed: a restored layout keeps the sizes the user dragged.
                if auto_grid {
                    dock.grid(wanted);
                }
                self.dirty.insert(tab_id.clone());
            }
        }
        self.last_sync = wanted_by_tab;

        // Views of closed panes go; detached windows of closed panes close.
        self.terminals.retain(|id, _| alive.contains(id));
        self.headers.retain(|id, _| alive.contains(id));
        self.browsers.retain(|id, _| with_browser.contains(id));
        self.view_subs.retain(|id, _| alive.contains(id));
        let gone: Vec<String> = self.detached.keys().filter(|id| !alive.contains(*id)).cloned().collect();
        for id in gone {
            self.reattach(&id, cx);
        }
        // Every pane gets its terminal now, so background sessions keep their screen state.
        for id in alive {
            self.terminal(&id, cx);
        }
        if !self.dirty.is_empty() {
            self.schedule_save(cx);
        }
    }

    /// Debounced (SAVE_DEBOUNCE_MS like persist.ts): writes changed layouts into their tabs, then store.json.
    fn layout_changed(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        self.dirty.insert(tab_id.to_owned());
        self.schedule_save(cx);
        cx.notify();
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(SAVE_DEBOUNCE_MS))
                .await;
            if let Err(e) = this.update(cx, |this, cx| this.write_layouts(cx)) {
                eprintln!("layout save skipped: {e}");
            }
        }));
    }

    fn write_layouts(&mut self, cx: &mut Context<Self>) {
        let dirty: Vec<String> = self.dirty.drain().collect();
        let layouts: Vec<(String, serde_json::Value)> = {
            let deck = self.deck.read(cx);
            dirty
                .iter()
                .filter_map(|tab_id| {
                    let dock = self.docks.get(tab_id)?;
                    // Sizes are proportions; dockview rescales them to its container on restore.
                    let json = dock.to_json(1200.0, 800.0, |id| panel_title(deck, id));
                    Some((tab_id.clone(), json))
                })
                .collect()
        };
        self.deck.update(cx, |deck, cx| {
            for (tab_id, json) in layouts {
                deck.state.set_tab_layout(&tab_id, json);
            }
            deck.save(cx);
        });
    }

    fn tab_of(&self, pane_id: &str, cx: &App) -> Option<String> {
        let panel = pane_id.strip_prefix("browser:").unwrap_or(pane_id);
        self.deck
            .read(cx)
            .state
            .tabs
            .iter()
            .find(|t| t.pane_ids.iter().any(|p| p == panel))
            .map(|t| t.id.clone())
    }

    fn reveal(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_of(pane_id, cx) else { return };
        if self.docks.get_mut(&tab).is_some_and(|d| d.reveal(pane_id)) {
            self.layout_changed(&tab, cx);
        }
    }

    fn jump(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_of(pane_id, cx) else { return };
        self.deck.update(cx, |deck, cx| {
            deck.state.active_tab_id = tab.clone();
            cx.notify();
        });
        self.reveal(pane_id, cx);
        self.focus_pane(pane_id, cx);
        // Keyboard focus follows, so typing goes to the agent (not for detached panes: other window).
        let terminal = self.terminals.get(pane_id).cloned().filter(|_| !self.detached.contains_key(pane_id));
        if let (Some(terminal), Some(handle)) = (terminal, crate::system::main_window(cx)) {
            cx.defer(move |cx| {
                let result = handle.update(cx, |_, window, cx| window.focus(&terminal.focus_handle(cx), cx));
                if let Err(e) = result {
                    eprintln!("layout: focus terminal: {e}");
                }
            });
        }
    }

    /// The pane becomes the active one (voice, palette targets) and is "seen".
    fn focus_pane(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        runtime::focus_pane(pane_id.strip_prefix("browser:").unwrap_or(pane_id), cx);
    }

    fn toggle_maximize(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_of(pane_id, cx) else { return };
        if let Some(dock) = self.docks.get_mut(&tab) {
            dock.toggle_maximize(pane_id);
            cx.notify();
        }
    }

    fn arrange(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        let Some(dock) = self.docks.get_mut(tab_id) else { return };
        let views = {
            let deck = self.deck.read(cx);
            let Some(tab) = deck.state.tabs.iter().find(|t| t.id == tab_id) else { return };
            let present = dock.views();
            tab.pane_ids
                .iter()
                .flat_map(|id| [id.clone(), format!("browser:{id}")])
                .filter(|id| present.contains(id))
                .collect::<Vec<_>>()
        };
        if views.is_empty() {
            return;
        }
        let active = dock
            .active_group
            .clone()
            .and_then(|g| dock.groups().into_iter().find(|x| x.id == g).map(|x| x.active.clone()));
        dock.grid(&views);
        if let Some(view) = active {
            dock.set_active(&view);
        }
        self.layout_changed(tab_id, cx);
    }

    /// Pane the pane-level actions target: the active one if in the visible session, else its first.
    fn current_pane(&self, cx: &App) -> Option<String> {
        let state = &self.deck.read(cx).state;
        let tab = state.active_tab()?;
        state
            .active_pane_id
            .clone()
            .filter(|id| tab.pane_ids.contains(id))
            .or_else(|| tab.pane_ids.first().cloned())
    }

    /// Ctrl+1..9: the Nth agent of the session.
    fn focus_nth(&mut self, index: usize, cx: &mut Context<Self>) {
        let pane = self
            .deck
            .read(cx)
            .state
            .active_tab()
            .and_then(|t| t.pane_ids.get(index).cloned());
        if let Some(pane) = pane {
            self.jump(&pane, cx);
        }
    }

    /// Ctrl+Shift+J: jumps to the agent waiting longest for an answer, in any workspace.
    fn next_waiting(&mut self, cx: &mut Context<Self>) {
        let pane = self
            .deck
            .read(cx)
            .state
            .panes
            .values()
            .filter(|p| activity_key(p) == ActivityKey::Status(PaneStatus::Waiting))
            .min_by_key(|p| p.status_since)
            .map(|p| p.id.clone());
        match pane.filter(|id| self.tab_of(id, cx).is_some()) {
            Some(id) => self.jump(&id, cx),
            None => self.deck.update(cx, |deck, cx| deck.toast("No agent is waiting.", cx)),
        }
    }

    fn toggle_detached(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        if self.detached.contains_key(pane_id) {
            self.reattach(pane_id, cx);
        } else {
            self.detach(pane_id, cx);
        }
    }

    /// Opens the pane in its own window (or focuses it if already detached).
    fn detach(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        if let Some(handle) = self.detached.get(pane_id) {
            if let Err(e) = handle.update(cx, |_, window, _| window.activate_window()) {
                eprintln!("activate pane window failed: {e}");
            }
            return;
        }
        if !self.deck.read(cx).state.panes.contains_key(pane_id) {
            return;
        }
        let terminal = self.terminal(pane_id, cx);
        let shell = cx.entity().downgrade();
        match detached::open(self.deck.clone(), pane_id, terminal, shell, cx) {
            Ok(handle) => {
                self.detached.insert(pane_id.to_owned(), handle);
            }
            Err(e) => self
                .deck
                .update(cx, |deck, cx| deck.toast(format!("Could not detach the pane: {e}"), cx)),
        }
        cx.notify();
    }

    /// Closes the pane's window; the terminal goes back into the grid.
    fn reattach(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        if let Some(handle) = self.detached.remove(pane_id) {
            detached::close(pane_id, handle, &self.deck, cx);
        }
        cx.notify();
    }

    /// Called by the detached window when it closes.
    fn mark_attached(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        self.detached.remove(pane_id);
        cx.notify();
    }

    /// "Add agent" with every option (AgentPicker), shown over the window.
    fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let deck = self.deck.clone();
        let picker = cx.new(|cx| AgentPicker::new(deck, window, cx));
        let sub = cx.subscribe(&picker, |this, _, _: &DismissEvent, cx| {
            this.picker = None;
            cx.notify();
        });
        self.picker = Some((picker, sub));
        cx.notify();
    }
}

/// Title of a panel (dockview `title`), kept in the saved layout.
fn panel_title(deck: &Deck, id: &str) -> String {
    if id.starts_with("browser:") {
        return "Browser".into();
    }
    deck.state
        .panes
        .get(id)
        .map(|p| if p.title.is_empty() { agent_name(p.agent).to_owned() } else { p.title.clone() })
        .unwrap_or_else(|| "Agent".into())
}

/// Once a second: toast expiry. Running agents repaint through the deck (`panes::runtime` clock);
/// an idle window only needs the minute-grained "x min ago" labels, so it redraws twice a minute.
fn ticker(cx: &mut Context<DeckView>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let mut seconds = 0u32;
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            seconds = seconds.wrapping_add(1);
            let alive = this.update(cx, |this, cx| {
                this.deck.update(cx, |deck, cx| {
                    if deck.state.prune_toasts(now_ms()) {
                        cx.notify();
                    }
                });
                if seconds % 30 == 0 {
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        }
    })
}

/// Every minute: today's usage for the status bar. It reads transcript files, so off the main thread.
fn usage_ticker(cx: &mut Context<DeckView>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            let summary = cx
                .background_executor()
                .spawn(async { agentdeck_core::usage::usage_summary("today".into()) })
                .await;
            let alive = this.update(cx, |this, cx| match summary {
                Ok(summary) => {
                    this.usage_today = Some(summary);
                    cx.notify();
                }
                Err(e) => eprintln!("layout: usage of today: {e}"),
            });
            if alive.is_err() {
                break;
            }
            cx.background_executor().timer(Duration::from_secs(60)).await;
        }
    })
}

impl Render for DeckView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::browser::frame_drawn();
        let p = palette(cx);
        self.ensure_headers(window, cx);
        let active_tab = self.deck.read(cx).state.active_tab_id.clone();
        let empty = self
            .deck
            .read(cx)
            .state
            .active_tab()
            .is_none_or(|t| t.pane_ids.is_empty());
        let wall = crate::orchestrator::wall_overlay(&self.deck, cx);
        // The wall covers the whole pane area: no dock (terminals) behind it.
        let page = crate::skills::page(&active_tab, cx);
        // A page brings its own side column: the Projects or Activity panel steps aside for it.
        let page_side = crate::skills::page_side(cx);
        let page_open = crate::skills::page_kind(cx).is_some();
        let dock = (wall.is_none() && page.is_none()).then(|| self.render_dock(&active_tab, window, cx));
        let pane_area = div()
            .relative()
            .flex()
            .flex_1()
            .min_h_0()
            .p(ui(6.0))
            .children(dock)
            .when(empty && page.is_none(), |el| el.child(self.empty_session(cx)))
            .when_some(wall, |el, wall| el.child(div().absolute().inset_0().child(wall)))
            .when_some(page, |el, page| el.child(div().absolute().inset_0().child(page)));
        let main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(self.tab_bar(window, cx))
            .child(pane_area);
        let picker = self.picker.as_ref().map(|(picker, _)| picker.clone());
        let git_panel = self.git_panel.as_ref().map(|(panel, _)| panel.clone());
        div()
            .id("deck-view")
            .key_context("DeckView")
            .track_focus(&self.focus)
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(hsla(p.gutter))
            .text_color(hsla(p.fg))
            .font_family(agentdeck_core::logic::theme::FONT_SANS)
            .text_size(ui(13.0))
            .on_action(cx.listener(|this, _: &FocusPane1, _, cx| this.focus_nth(0, cx)))
            .on_action(cx.listener(|this, _: &FocusPane2, _, cx| this.focus_nth(1, cx)))
            .on_action(cx.listener(|this, _: &FocusPane3, _, cx| this.focus_nth(2, cx)))
            .on_action(cx.listener(|this, _: &FocusPane4, _, cx| this.focus_nth(3, cx)))
            .on_action(cx.listener(|this, _: &FocusPane5, _, cx| this.focus_nth(4, cx)))
            .on_action(cx.listener(|this, _: &FocusPane6, _, cx| this.focus_nth(5, cx)))
            .on_action(cx.listener(|this, _: &FocusPane7, _, cx| this.focus_nth(6, cx)))
            .on_action(cx.listener(|this, _: &FocusPane8, _, cx| this.focus_nth(7, cx)))
            .on_action(cx.listener(|this, _: &FocusPane9, _, cx| this.focus_nth(8, cx)))
            .on_action(cx.listener(|this, _: &NextWaiting, _, cx| this.next_waiting(cx)))
            .on_action(cx.listener(|this, _: &ToggleActivity, _, cx| this.toggle_activity(cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &ArrangeInGrid, _, cx| {
                let tab = this.deck.read(cx).state.active_tab_id.clone();
                this.arrange(&tab, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleMaximize, _, cx| {
                if let Some(pane) = this.current_pane(cx) {
                    this.toggle_maximize(&pane, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &RenameSession, window, cx| {
                let tab = this.deck.read(cx).state.active_tab_id.clone();
                this.start_rename(&tab, window, cx);
            }))
            // Esc restores a maximized pane, unless the key is meant for a terminal (agents use Esc):
            // only when the shell itself has focus (pane chrome, tab bar), not a terminal.
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key != "escape" || e.is_held {
                    return;
                }
                let in_shell = this.focus.is_focused(window) || this.tabs_focus.is_focused(window);
                let tab = this.deck.read(cx).state.active_tab_id.clone();
                if in_shell
                    && let Some(dock) = this.docks.get_mut(&tab)
                    && dock.maximized.is_some()
                {
                    dock.maximized = None;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(self.title_bar(window, cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.rail(cx))
                    .when(self.sidebar_open && !page_open, |el| el.child(self.sidebar(cx)))
                    .when(self.activity_open && !page_open, |el| el.child(self.activity_panel(cx)))
                    .children(page_side)
                    .child(main),
            )
            .child(self.status_bar(cx))
            .when(!self.activity_open, |el| el.child(self.permission_banner.clone()))
            .child(self.toasts(cx))
            .when_some(picker, |el, picker| el.child(div().absolute().inset_0().child(picker)))
            .when_some(git_panel, |el, panel| el.child(panel))
            .children(crate::history::overlay(cx))
            .children(crate::skills::overlay(cx))
    }
}
