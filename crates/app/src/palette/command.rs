//! Ctrl+K / Ctrl+Shift+P: jump to any agent, session or workspace, or run any action
//! (components/CommandPalette/CommandPalette.tsx).

use agentdeck_core::agents::AgentKind;
use agentdeck_core::logic::deck::{ActivityKey, activity_key};
use agentdeck_core::logic::text::agent_name;
use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::fuzzy::fuzzy_score;
use super::ui;
use crate::panes::{status_color, status_label};
use crate::state::Deck;
use crate::theme::{hsla, palette};

actions!(
    agentdeck_palette,
    [SelectNext, SelectPrev, SelectFirst, SelectLast, NextSection, PrevSection, RunSelected]
);

const CONTEXT: &str = "CommandPalette";
const MAX_RECENT: usize = 5;

/// Bindings scoped to the palette: they win over the dialog's Enter (confirm) and Root's Tab.
pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        KeyBinding::new("up", SelectPrev, Some(CONTEXT)),
        KeyBinding::new("ctrl-home", SelectFirst, Some(CONTEXT)),
        KeyBinding::new("ctrl-end", SelectLast, Some(CONTEXT)),
        KeyBinding::new("tab", NextSection, Some(CONTEXT)),
        KeyBinding::new("shift-tab", PrevSection, Some(CONTEXT)),
        KeyBinding::new("enter", RunSelected, Some(CONTEXT)),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Section {
    Recent,
    Agents,
    Sessions,
    Workspaces,
    Actions,
}

impl Section {
    const LISTED: [Section; 4] = [Section::Agents, Section::Sessions, Section::Workspaces, Section::Actions];

    fn label(self) -> &'static str {
        match self {
            Section::Recent => "Recent",
            Section::Agents => "Agents",
            Section::Sessions => "Sessions",
            Section::Workspaces => "Projects",
            Section::Actions => "Actions",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Run {
    Pane(String),
    Tab(String),
    Archived(String),
    Workspace(String),
    Action(&'static str),
}

#[derive(Clone, Copy)]
enum Lead {
    Agent(AgentKind),
    Icon(IconName),
}

#[derive(Clone, Copy)]
enum Trailing {
    None,
    Status(ActivityKey),
    Kbd(&'static str),
}

#[derive(Clone)]
pub(super) struct Item {
    pub id: String,
    pub section: Section,
    pub label: String,
    pub detail: Option<String>,
    lead: Lead,
    trailing: Trailing,
    pub run: Run,
}

/// (id, label, icon, shortcut hint). Same order and labels as the original.
const ACTIONS: [(&str, &str, IconName, Option<&str>); 25] = [
    ("add-agent", "Add agent", IconName::Plus, None),
    ("new-session", "New session", IconName::SquarePlus, None),
    ("link-workspace", "Link project", IconName::FolderPlus, None),
    ("interrupt-all", "Interrupt all", IconName::Pause, None),
    ("stop-all", "Stop all", IconName::SquareStop, None),
    ("restart-all", "Restart all", IconName::RotateCcw, None),
    ("settings", "Open settings", IconName::Settings, Some("Ctrl+,")),
    ("sidebar", "Toggle sidebar", IconName::PanelLeft, None),
    ("activity", "Toggle activity", IconName::Activity, Some("Ctrl+Shift+A")),
    ("wall", "Wall mode", IconName::LayoutGrid, Some("Ctrl+Shift+W")),
    ("maximize", "Maximize/restore active pane", IconName::Maximize2, None),
    ("grid", "Arrange panes in a grid", IconName::Grid2x2, None),
    ("zoom-in", "Zoom +", IconName::ZoomIn, Some("Ctrl+=")),
    ("zoom-out", "Zoom -", IconName::ZoomOut, Some("Ctrl+-")),
    ("broadcast", "Broadcast prompt", IconName::Megaphone, Some("Ctrl+Shift+B")),
    ("race", "Race a prompt (N agents, one worktree each)", IconName::Flag, None),
    ("issue", "New agent from a GitHub issue (link in the clipboard)", IconName::GitPullRequest, None),
    ("templates", "New session from template", IconName::LayoutTemplate, None),
    ("checkpoint", "Create restore point (active pane)", IconName::RotateCcwClock, None),
    ("detach", "Detach active pane", IconName::ExternalLink, None),
    ("usage", "Usage", IconName::Gauge, None),
    ("history", "History", IconName::RotateCcwClock, Some("Ctrl+Shift+F")),
    ("journal", "Today's journal", IconName::NotebookText, None),
    ("orchestrator", "New orchestrator", IconName::Crown, None),
    ("orchestrator-pause", "Pause/resume orchestrator", IconName::Pause, None),
];

/// Every jump target and action, in section order. Read once when the palette opens.
pub(super) fn build_items(deck: &Deck) -> Vec<Item> {
    let s = &deck.state;
    let ws_name = |id: &str| {
        s.workspaces
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.name.clone())
            .unwrap_or_default()
    };
    let mut items = Vec::new();
    for tab in &s.tabs {
        for id in &tab.pane_ids {
            let Some(pane) = s.panes.get(id) else { continue };
            let label = if pane.title.is_empty() { agent_name(pane.agent).to_owned() } else { pane.title.clone() };
            items.push(Item {
                id: format!("pane:{id}"),
                section: Section::Agents,
                label,
                detail: Some(format!("{} › {}", ws_name(&tab.workspace_id), tab.name)),
                lead: Lead::Agent(pane.agent),
                trailing: Trailing::Status(activity_key(pane)),
                run: Run::Pane(id.clone()),
            });
        }
    }
    for tab in &s.tabs {
        items.push(Item {
            id: format!("tab:{}", tab.id),
            section: Section::Sessions,
            label: tab.name.clone(),
            detail: Some(ws_name(&tab.workspace_id)),
            lead: Lead::Icon(IconName::Layers),
            trailing: Trailing::None,
            run: Run::Tab(tab.id.clone()),
        });
    }
    for session in &s.archived {
        items.push(Item {
            id: format!("archived:{}", session.id),
            section: Section::Sessions,
            label: session.name.clone(),
            detail: Some(format!("{} · archived", ws_name(&session.workspace_id))),
            lead: Lead::Icon(IconName::Archive),
            trailing: Trailing::None,
            run: Run::Archived(session.id.clone()),
        });
    }
    for ws in &s.workspaces {
        items.push(Item {
            id: format!("ws:{}", ws.id),
            section: Section::Workspaces,
            label: ws.name.clone(),
            detail: (!ws.path.is_empty()).then(|| ws.path.clone()),
            lead: Lead::Icon(IconName::Folder),
            trailing: Trailing::None,
            run: Run::Workspace(ws.id.clone()),
        });
    }
    for (id, label, icon, shortcut) in ACTIONS {
        items.push(Item {
            id: format!("action:{id}"),
            section: Section::Actions,
            label: label.to_owned(),
            detail: id.ends_with("-all").then(|| "Active session".to_owned()),
            lead: Lead::Icon(icon),
            trailing: shortcut.map_or(Trailing::None, Trailing::Kbd),
            run: Run::Action(id),
        });
    }
    items
}

/// Items grouped for display (indices into `items`): recents first when the query is empty,
/// fuzzy-ranked within sections otherwise.
pub(super) fn filter_items(items: &[Item], query: &str, recent: &[String]) -> Vec<(Section, Vec<usize>)> {
    let in_section = |section: Section, ix: &usize| items[*ix].section == section;
    if query.trim().is_empty() {
        let recents: Vec<usize> = recent
            .iter()
            .flat_map(|id| (0..items.len()).filter(move |&i| &items[i].id == id))
            .collect();
        let rest: Vec<usize> = (0..items.len()).filter(|i| !recent.contains(&items[*i].id)).collect();
        return std::iter::once((Section::Recent, recents))
            .chain(
                Section::LISTED
                    .into_iter()
                    .map(|section| (section, rest.iter().copied().filter(|i| in_section(section, i)).collect())),
            )
            .filter(|(_, list): &(Section, Vec<usize>)| !list.is_empty())
            .collect();
    }
    let scored: Vec<(usize, f64)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let score = fuzzy_score(query, &item.label).or_else(|| {
                let full = format!("{} {}", item.label, item.detail.as_deref().unwrap_or(""));
                fuzzy_score(query, &full).map(|s| s - 5.0)
            })?;
            Some((i, score))
        })
        .collect();
    Section::LISTED
        .into_iter()
        .map(|section| {
            let mut list: Vec<(usize, f64)> = scored.iter().copied().filter(|(i, _)| in_section(section, i)).collect();
            list.sort_by(|a, b| b.1.total_cmp(&a.1));
            (section, list.into_iter().map(|(i, _)| i).collect::<Vec<_>>())
        })
        .filter(|(_, list)| !list.is_empty())
        .collect()
}

pub struct CommandPalette {
    deck: Entity<Deck>,
    items: Vec<Item>,
    groups: Vec<(Section, Vec<usize>)>,
    query: String,
    active: usize,
    focus: FocusHandle,
    scroll: ScrollHandle,
}

/// Ctrl+K: opens the palette, or closes it when it is already open.
pub(super) fn toggle(deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    let open = cx
        .global::<super::PaletteGlobal>()
        .palette
        .as_ref()
        .and_then(WeakEntity::upgrade);
    if open.is_some() {
        cx.global_mut::<super::PaletteGlobal>().palette = None;
        window.close_dialog(cx);
        return;
    }
    let view = cx.new(|cx| CommandPalette::new(deck, cx));
    cx.global_mut::<super::PaletteGlobal>().palette = Some(view.downgrade());
    let focus = view.read(cx).focus.clone();
    let top = window.viewport_size().height * 0.12;
    super::open_modal(view, 608.0, Some(top), focus, window, cx);
}

impl CommandPalette {
    pub fn new(deck: Entity<Deck>, cx: &mut Context<Self>) -> Self {
        let items = build_items(deck.read(cx));
        let mut this = Self {
            deck,
            items,
            groups: Vec::new(),
            query: String::new(),
            active: 0,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        };
        this.refilter(cx);
        this
    }

    fn refilter(&mut self, cx: &App) {
        let recent = &cx.global::<super::PaletteGlobal>().recent_items;
        self.groups = filter_items(&self.items, &self.query, recent);
        self.active = 0;
    }

    fn flat(&self) -> Vec<usize> {
        self.groups.iter().flat_map(|(_, list)| list.iter().copied()).collect()
    }

    fn section_starts(&self) -> Vec<usize> {
        let mut start = 0;
        self.groups
            .iter()
            .map(|(_, list)| {
                let s = start;
                start += list.len();
                s
            })
            .collect()
    }

    /// Row index in the scroll list (section headers count as rows) of flat item `index`.
    fn row_of(&self, index: usize) -> usize {
        let starts = self.section_starts();
        let group = starts.iter().filter(|&&s| s <= index).count();
        index + group
    }

    fn select(&mut self, next: usize, cx: &mut Context<Self>) {
        self.active = next;
        self.scroll.scroll_to_item(self.row_of(next));
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.flat().len() as isize;
        if count > 0 {
            self.select((self.active as isize + delta).rem_euclid(count) as usize, cx);
        }
    }

    fn jump_section(&mut self, delta: isize, cx: &mut Context<Self>) {
        let starts = self.section_starts();
        if starts.is_empty() {
            return;
        }
        let current = starts.iter().filter(|&&s| s <= self.active).count() as isize - 1;
        let target = (current + delta).rem_euclid(starts.len() as isize) as usize;
        self.select(starts[target], cx);
    }

    fn run(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.flat().get(index).map(|&i| self.items[i].clone()) else {
            return;
        };
        let recent = &mut cx.global_mut::<super::PaletteGlobal>().recent_items;
        recent.retain(|id| id != &item.id);
        recent.insert(0, item.id.clone());
        recent.truncate(MAX_RECENT);
        cx.global_mut::<super::PaletteGlobal>().palette = None;
        window.close_dialog(cx);
        run_item(&self.deck, &item, window, cx);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        let m = &ks.modifiers;
        match ks.key.as_str() {
            "escape" => {
                cx.global_mut::<super::PaletteGlobal>().palette = None;
                window.close_dialog(cx);
            }
            "backspace" if m.control => self.query.clear(),
            "backspace" => {
                self.query.pop();
            }
            "v" if m.control => {
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
        let groups_before = self.groups.len();
        self.refilter(cx);
        if groups_before != self.groups.len() || self.active == 0 {
            self.scroll.scroll_to_item(0);
        }
        cx.notify();
    }

    fn render_item(&self, index: usize, item: &Item, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette(cx);
        let selected = index == self.active;
        let lead = match item.lead {
            Lead::Agent(kind) => ui::agent_mark(kind, 15.0, &p).into_any_element(),
            Lead::Icon(icon) => div().text_color(hsla(p.subtle)).child(icon).into_any_element(),
        };
        let trailing = match item.trailing {
            Trailing::None => None,
            Trailing::Kbd(keys) => Some(ui::kbd(keys, &p).into_any_element()),
            Trailing::Status(ActivityKey::Exited) => Some(
                div()
                    .size(crate::theme::ui(8.0))
                    .rounded_full()
                    .border_1()
                    .border_color(hsla(p.line_strong))
                    .into_any_element(),
            ),
            Trailing::Status(ActivityKey::Status(status)) => Some(
                div()
                    .id(SharedString::from(format!("status-{}", item.id)))
                    .size(crate::theme::ui(8.0))
                    .rounded_full()
                    .bg(hsla(status_color(status, &p)))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(status_label(status)).build(window, cx)
                    })
                    .into_any_element(),
            ),
        };
        div()
            .id(index)
            .mx_1()
            .flex()
            .items_center()
            .gap_2p5()
            .h(crate::theme::ui(36.0))
            .px_2()
            .rounded(crate::theme::ui(6.0))
            .text_size(crate::theme::ui(14.0))
            .when(selected, |s| s.bg(hsla(p.raised)).text_color(hsla(p.fg)))
            .when(!selected, |s| s.text_color(hsla(p.muted)))
            .child(div().flex().flex_shrink_0().size_4().items_center().justify_center().text_size(crate::theme::ui(15.0)).child(lead))
            .child(div().flex_shrink_0().max_w(crate::theme::ui(300.0)).truncate().child(item.label.clone()))
            .children(
                item.detail
                    .clone()
                    .map(|d| div().min_w_0().flex_1().truncate().text_xs().text_color(hsla(p.subtle)).child(d)),
            )
            .child(div().ml_auto().flex().flex_shrink_0().items_center().children(trailing))
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.active != index {
                    this.active = index;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.run(index, window, cx)))
    }
}

/// Runs a palette item. Actions owned by other modules go through their GPUI actions.
fn run_item(deck: &Entity<Deck>, item: &Item, window: &mut Window, cx: &mut App) {
    match &item.run {
        Run::Pane(pane) => {
            close_wall(cx);
            crate::layout::jump_to_pane(pane, cx);
        }
        Run::Tab(tab) => {
            close_wall(cx);
            deck.update(cx, |d, cx| {
                d.state.active_tab_id = tab.clone();
                d.save(cx);
                cx.notify();
            });
        }
        Run::Archived(id) => {
            close_wall(cx);
            crate::panes::runtime::reopen_session(id, cx);
            deck.update(cx, |d, cx| d.save(cx));
        }
        Run::Workspace(id) => deck.update(cx, |d, cx| {
            d.state.select_workspace(id);
            d.save(cx);
            cx.notify();
        }),
        Run::Action(id) => run_action(deck, id, &item.label, window, cx),
    }
}

fn close_wall(cx: &mut App) {
    if crate::orchestrator::wall_open(cx) {
        crate::orchestrator::toggle_wall(cx);
    }
}

/// Pane-level actions target the current pane (`onCurrentPane`).
fn on_current_pane(deck: &Entity<Deck>, cx: &mut App, f: impl FnOnce(&str, &str, &mut App)) {
    if let Some((tab, pane)) = super::current_pane(deck.read(cx)) {
        f(&tab, &pane, cx);
    }
}

fn run_action(deck: &Entity<Deck>, id: &str, label: &str, window: &mut Window, cx: &mut App) {
    use crate::panes::runtime;
    match id {
        "add-agent" => super::dispatch_named(deck, "AddAgent", label, window, cx),
        "new-session" => deck.update(cx, |d, cx| {
            d.state.add_tab(None);
            d.save(cx);
            cx.notify();
        }),
        "link-workspace" => crate::orchestrator::open_link_workspace(deck.clone(), window, cx),
        "interrupt-all" => runtime::interrupt_all(cx),
        "stop-all" => confirm_bulk(deck, "Stop", runtime::stop_all, window, cx),
        "restart-all" => confirm_bulk(deck, "Restart", runtime::restart_all, window, cx),
        "settings" => window.dispatch_action(Box::new(crate::settings::OpenSettings), cx),
        "sidebar" => window.dispatch_action(Box::new(crate::layout::ToggleSidebar), cx),
        "activity" => window.dispatch_action(Box::new(crate::layout::ToggleActivity), cx),
        "wall" => crate::orchestrator::toggle_wall(cx),
        "maximize" => on_current_pane(deck, cx, |_, pane, cx| crate::layout::toggle_maximize_pane(pane, cx)),
        "grid" => {
            let tab = deck.read(cx).state.active_tab_id.clone();
            crate::layout::arrange_in_grid(&tab, cx);
        }
        "zoom-in" | "zoom-out" => {
            let delta = if id == "zoom-in" { 1.0 } else { -1.0 };
            on_current_pane(deck, cx, |_, pane, cx| {
                deck.update(cx, |d, cx| {
                    d.state.zoom(pane, Some(delta));
                    d.save(cx);
                    cx.notify();
                })
            });
        }
        "broadcast" => super::open_broadcast(deck.clone(), window, cx),
        "race" => super::open_race(deck.clone(), window, cx),
        "issue" => super::agent_from_issue(deck, cx),
        "templates" => super::open_template_picker(deck.clone(), window, cx),
        "checkpoint" => on_current_pane(deck, cx, |_, pane, cx| {
            let job = crate::git::checkpoint_pane(deck, pane, cx);
            cx.background_executor()
                .spawn(async move {
                    if let Err(e) = job.await {
                        eprintln!("checkpoint failed: {e}");
                    }
                })
                .detach();
        }),
        "detach" => on_current_pane(deck, cx, |_, pane, cx| crate::layout::toggle_detached(pane, cx)),
        "usage" => window.dispatch_action(Box::new(crate::history::OpenUsage), cx),
        "history" => window.dispatch_action(Box::new(crate::history::SearchHistory), cx),
        "journal" => window.dispatch_action(Box::new(crate::history::OpenJournal), cx),
        "orchestrator" => crate::orchestrator::create_orchestrator(deck, cx),
        "orchestrator-pause" => {
            let paused = deck.read(cx).orchestrator.paused;
            crate::orchestrator::set_paused(deck, !paused, cx);
        }
        _ => {}
    }
}

/// Runs a bulk action on the active session, asking first unless disabled in Settings
/// (TitleBar.tsx `confirmBulk`).
fn confirm_bulk(deck: &Entity<Deck>, verb: &'static str, action: fn(&mut App), window: &mut Window, cx: &mut App) {
    let (count, ask) = {
        let d = deck.read(cx);
        (d.state.active_tab().map_or(0, |t| t.pane_ids.len()), d.settings.confirm_bulk)
    };
    if count == 0 {
        return;
    }
    if !ask {
        action(cx);
        return;
    }
    let question = format!("{verb} the {count} agent(s) in this session?");
    window.open_alert_dialog(cx, move |alert, _, _| {
        alert
            .title(question.clone())
            .show_cancel(true)
            .ok_text(verb)
            .on_ok(move |_, _, cx| {
                action(cx);
                true
            })
    });
}

impl Focusable for CommandPalette {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CommandPalette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let groups = self.groups.clone();
        let starts = self.section_starts();
        let empty = groups.is_empty();
        let mut rows: Vec<AnyElement> = Vec::new();
        for ((section, list), start) in groups.iter().zip(starts) {
            rows.push(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_xs()
                    .text_color(hsla(p.subtle))
                    .child(section.label())
                    .into_any_element(),
            );
            for (offset, &ix) in list.iter().enumerate() {
                let item = self.items[ix].clone();
                rows.push(self.render_item(start + offset, &item, cx).into_any_element());
            }
        }
        let query = if self.query.is_empty() {
            div().text_color(hsla(p.subtle)).child("Agent, session, project, action…")
        } else {
            div().text_color(hsla(p.fg)).child(self.query.clone())
        };
        let caret = div().w(px(1.0)).h(crate::theme::ui(16.0)).bg(hsla(p.fg));
        let max_list = window.viewport_size().height * 0.6;
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| {
                if !this.flat().is_empty() {
                    this.select(0, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| {
                let count = this.flat().len();
                if count > 0 {
                    this.select(count - 1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextSection, _, cx| this.jump_section(1, cx)))
            .on_action(cx.listener(|this, _: &PrevSection, _, cx| this.jump_section(-1, cx)))
            .on_action(cx.listener(|this, _: &RunSelected, window, cx| this.run(this.active, window, cx)))
            .flex()
            .flex_col()
            .w_full()
            .overflow_hidden()
            .text_size(crate::theme::ui(14.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(crate::theme::ui(44.0))
                    .px_3()
                    .border_b_1()
                    .border_color(hsla(p.line))
                    .child(div().text_size(crate::theme::ui(16.0)).text_color(hsla(p.subtle)).child(IconName::Search))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .overflow_hidden()
                            // Caret after the text, or before the placeholder.
                            .map(|s| if self.query.is_empty() { s.child(caret).child(query) } else { s.child(query).child(caret) }),
                    )
                    .child(ui::kbd("Esc", &p)),
            )
            .child(
                div()
                    .id("palette-listbox")
                    .track_scroll(&self.scroll)
                    .overflow_y_scroll()
                    .max_h(max_list)
                    .py_1()
                    .when(empty, |s| {
                        s.child(div().px_3().py_4().text_xs().text_color(hsla(p.subtle)).child("No results."))
                    })
                    .children(rows),
            )
    }
}

#[cfg(test)]
mod tests {
    // Explicit imports: a glob of `super` would bring GPUI's `test` macro over Rust's #[test].
    use super::{ACTIONS, IconName, Item, Lead, Run, Section, Trailing, filter_items};

    fn item(id: String, section: Section, label: &str) -> Item {
        Item {
            id,
            section,
            label: label.to_owned(),
            detail: None,
            lead: Lead::Icon(IconName::Layers),
            trailing: Trailing::None,
            run: Run::Action("none"),
        }
    }

    fn items() -> Vec<Item> {
        let mut all = vec![item("tab:1".into(), Section::Sessions, "Main")];
        all.extend(ACTIONS.iter().map(|(id, label, _, _)| item(format!("action:{id}"), Section::Actions, label)));
        all
    }

    fn labels(items: &[Item], groups: &[(Section, Vec<usize>)]) -> Vec<(Section, Vec<String>)> {
        groups
            .iter()
            .map(|(s, list)| (*s, list.iter().map(|&i| items[i].label.clone()).collect()))
            .collect()
    }

    #[test]
    fn empty_query_puts_recents_first_and_keeps_section_order() {
        let items = items();
        let groups = filter_items(&items, "  ", &["action:wall".to_owned()]);
        let got = labels(&items, &groups);
        assert_eq!(got[0], (Section::Recent, vec!["Wall mode".to_owned()]));
        assert_eq!(got[1].0, Section::Sessions);
        assert_eq!(got[2].0, Section::Actions);
        assert!(!got[2].1.contains(&"Wall mode".to_owned()));
    }

    #[test]
    fn query_ranks_within_sections_and_drops_misses() {
        let items = items();
        let got = labels(&items, &filter_items(&items, "wall", &[]));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, Section::Actions);
        assert_eq!(got[0].1[0], "Wall mode");
        assert!(filter_items(&items, "zzzz", &[]).is_empty());
    }
}
