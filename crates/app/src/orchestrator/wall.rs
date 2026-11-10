//! Wall mode: every pane of the workspace (or of all workspaces) as compact tiles, most urgent first.
//! Drawn in place of the session grid: the terminal views stay alive but are not laid out or painted.
//! Original: src/components/Wall/{Wall,wallState,jump}.tsx.

use std::collections::HashMap;
use std::time::Duration;

use agentdeck_core::logic::deck::{ActivityKey, DeckState, activity_key};
use agentdeck_core::logic::text::{agent_name, format_since, last_lines, now_ms};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::base::Selectable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::panes::{agent_logo, status_dot, status_label};
use crate::state::Deck;
use crate::theme::{hsla, palette, ui};

/// Raw output kept per tile: enough for ~6 lines even with escape codes.
const TAIL_CHARS: usize = 8000;
const TILE_LINES: usize = 6;
const REFRESH: Duration = Duration::from_millis(250);

#[derive(Default)]
struct WallState {
    open: bool,
    all_workspaces: bool,
    /// Alive only while open: its refresh loop reads every pane's output.
    view: Option<Entity<Wall>>,
}

impl Global for WallState {}

pub fn wall_open(cx: &App) -> bool {
    cx.try_global::<WallState>().is_some_and(|s| s.open)
}

/// Ctrl+Shift+W and the title bar button.
pub fn toggle_wall(cx: &mut App) {
    let open = !wall_open(cx);
    set_open(open, cx);
}

fn set_open(open: bool, cx: &mut App) {
    let state = cx.default_global::<WallState>();
    state.open = open;
    if !open {
        state.view = None;
    }
    cx.refresh_windows();
}

/// The wall view while it is open, to render on top of the pane area (absolute, inset 0).
pub fn wall_overlay(deck: &Entity<Deck>, cx: &mut App) -> Option<Entity<Wall>> {
    if !wall_open(cx) {
        return None;
    }
    if let Some(view) = cx.global::<WallState>().view.clone() {
        return Some(view);
    }
    let view = cx.new(|cx| Wall::new(deck.clone(), cx));
    cx.global_mut::<WallState>().view = Some(view.clone());
    Some(view)
}

/// Title bar toggle for the wall.
pub fn wall_toggle_button(cx: &App) -> Button {
    let open = wall_open(cx);
    Button::new("wall-toggle")
        .ghost()
        .small()
        .icon(Icon::new(IconName::LayoutGrid))
        .selected(open)
        .tooltip(format!(
            "{} wall: every agent at a glance (Ctrl+Shift+W)",
            if open { "Exit" } else { "Show" }
        ))
        .on_click(|_, _, cx| toggle_wall(cx))
}

/// Leaves the wall, shows the pane's session and makes it the active pane.
pub fn jump_to_pane(deck: &Entity<Deck>, tab_id: &str, pane_id: &str, cx: &mut App) {
    set_open(false, cx);
    deck.update(cx, |d, cx| {
        d.state.active_tab_id = tab_id.to_owned();
        cx.notify();
    });
    // Active pane, acknowledgement and voice target, like the activity panel; also reveals it in its dock.
    super::runtime::focus_pane(pane_id, cx);
    crate::layout::jump_to_pane(pane_id, cx);
}

/// Pane ids on the wall, most urgent first (waiting, error, done, working, idle, stopped), oldest
/// status first; `workspace_id: None` takes every workspace.
fn wall_items(state: &DeckState, workspace_id: Option<&str>) -> Vec<(String, String)> {
    state
        .activity_groups()
        .into_iter()
        .flat_map(|(_, items)| items)
        .filter(|(_, _, ws)| workspace_id.is_none_or(|id| ws.id == id))
        .map(|(p, t, _)| (p.id.clone(), t.id.clone()))
        .collect()
}

pub struct Wall {
    deck: Entity<Deck>,
    /// Last output lines per pane, refreshed every 250 ms.
    lines: HashMap<String, Vec<String>>,
    focus: FocusHandle,
    focused: bool,
    _refresh: Task<()>,
    _observe: Subscription,
}

impl Wall {
    pub fn new(deck: Entity<Deck>, cx: &mut Context<Self>) -> Self {
        let refresh = cx.spawn(async move |this, cx| {
            while this.update(cx, |wall, cx| wall.refresh(cx)).is_ok() {
                cx.background_executor().timer(REFRESH).await;
            }
        });
        Self {
            _observe: cx.observe(&deck, |_, _, cx| cx.notify()),
            deck,
            lines: HashMap::new(),
            focus: cx.focus_handle(),
            focused: false,
            _refresh: refresh,
        }
    }

    fn items(&self, cx: &App) -> Vec<(String, String)> {
        let d = self.deck.read(cx);
        let all = cx.try_global::<WallState>().is_some_and(|s| s.all_workspaces);
        let ws = d.state.active_workspace().id.clone();
        wall_items(&d.state, (!all).then_some(ws.as_str()))
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let mut lines = HashMap::new();
        for (id, _) in self.items(cx) {
            let tail = super::runtime::pane_output_tail(&id, TAIL_CHARS, cx);
            lines.insert(id, last_lines(&tail, TILE_LINES));
        }
        // The "since" timers tick with the deck clock (`_observe`).
        if lines != self.lines {
            self.lines = lines;
            cx.notify();
        }
    }

    fn tile(&self, pane_id: &str, tab_id: &str, now: i64, cx: &App) -> Option<Stateful<Div>> {
        let p = palette(cx);
        let d = self.deck.read(cx);
        let pane = d.state.panes.get(pane_id)?;
        let tab = d.state.tabs.iter().find(|t| t.id == tab_id)?;
        let ws = d.state.workspaces.iter().find(|w| w.id == tab.workspace_id)?;
        let key = activity_key(pane);
        let status = match key {
            ActivityKey::Status(s) => Some(s),
            ActivityKey::Exited => None,
        };
        let name = if pane.title.is_empty() { agent_name(pane.agent).to_owned() } else { pane.title.clone() };
        let lines = self.lines.get(pane_id).filter(|l| !l.is_empty());
        let mono = agentdeck_core::logic::theme::FONT_MONO;
        let dot = match status {
            Some(s) => status_dot(&format!("wall-{}", pane.id), s, &p),
            None => div()
                .size_2()
                .flex_shrink_0()
                .rounded_full()
                .border_1()
                .border_color(hsla(p.line_strong))
                .into_any_element(),
        };
        let (deck, tab_id, id) = (self.deck.clone(), tab.id.clone(), pane.id.clone());
        Some(
            div()
                .id(SharedString::from(format!("wall-{}", pane.id)))
                .flex()
                .flex_col()
                .gap(ui(6.0))
                .flex_grow(1.0)
                .flex_basis(ui(300.0))
                .min_w(ui(300.0))
                .p_3()
                .rounded(ui(8.0))
                .border_1()
                .border_color(hsla(p.line))
                .when(status == Some(agentdeck_core::hooks_server::PaneStatus::Waiting), |el| {
                    // Waiting for the user: amber line on top, like the pane header.
                    el.border_t_2().border_color(hsla(p.wait))
                })
                .bg(hsla(p.base))
                .cursor_pointer()
                .hover(|s| s.bg(hsla(p.panel)).border_color(hsla(p.line_strong)))
                .on_click(move |_, _, cx| jump_to_pane(&deck, &tab_id, &id, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(agent_logo(pane.agent, 14.0, None, &p))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(hsla(p.fg))
                                .child(name),
                        )
                        .child(dot)
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(hsla(p.muted))
                                .child(status.map_or("Stopped", status_label)),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(hsla(p.subtle))
                                .child(format_since(now - pane.status_since)),
                        ),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(hsla(p.subtle))
                        .child(format!("{} \u{203a} {}", ws.name, tab.name)),
                )
                .child(
                    div()
                        .h_4()
                        .truncate()
                        .text_xs()
                        .text_color(hsla(p.muted))
                        .child(pane.last_action.clone().unwrap_or_default()),
                )
                .child(
                    div()
                        .h_24()
                        .overflow_hidden()
                        .rounded(ui(6.0))
                        .bg(hsla(p.gutter))
                        .px_2()
                        .py_1()
                        .font_family(mono)
                        .text_size(ui(11.0))
                        .line_height(ui(16.0))
                        .text_color(hsla(p.muted))
                        .children(match lines {
                            Some(lines) => lines.iter().map(|l| div().whitespace_nowrap().child(l.clone())).collect(),
                            None => vec![div().text_color(hsla(p.subtle)).child("No output")],
                        }),
                ),
        )
    }
}

impl Render for Wall {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Take focus from the terminal underneath so typing doesn't go to a hidden agent.
        if !self.focused {
            self.focused = true;
            window.focus(&self.focus, cx);
        }
        let p = palette(cx);
        let now = now_ms();
        let all = cx.try_global::<WallState>().is_some_and(|s| s.all_workspaces);
        let ws_name = self.deck.read(cx).state.active_workspace().name.clone();
        let items = self.items(cx);
        let count = items.len();
        let tiles: Vec<_> = items
            .iter()
            .filter_map(|(pane, tab)| self.tile(pane, tab, now, cx))
            .collect();
        div()
            .id("wall")
            .track_focus(&self.focus)
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .bg(hsla(p.gutter))
            .text_size(ui(13.0))
            .on_key_down(cx.listener(|_, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    set_open(false, cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .h_10()
                    .flex_shrink_0()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(hsla(p.line))
                    .bg(hsla(p.panel))
                    .pr_2()
                    .pl_3()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(hsla(p.subtle))
                            .child(format!(
                                "Wall \u{b7} {}",
                                if all { "all workspaces" } else { ws_name.as_str() }
                            ))
                            .child(count.to_string()),
                    )
                    .child(div().flex_1())
                    .child(
                        Checkbox::new("wall-all-workspaces")
                            .checked(all)
                            .label("All workspaces")
                            .small()
                            .on_click(|checked, _, cx| {
                                cx.default_global::<WallState>().all_workspaces = *checked;
                                cx.refresh_windows();
                            }),
                    )
                    .child(
                        Button::new("wall-exit")
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::X))
                            .tooltip("Exit wall (Esc, Ctrl+Shift+W)")
                            .on_click(|_, _, cx| set_open(false, cx)),
                    ),
            )
            .child(
                div()
                    .id("wall-tiles")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(ui(6.0))
                    .child(if tiles.is_empty() {
                        div()
                            .px_2()
                            .py_4()
                            .text_xs()
                            .text_color(hsla(p.subtle))
                            .child("No agents running.")
                    } else {
                        div().flex().flex_wrap().gap(ui(6.0)).children(tiles)
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use agentdeck_core::agents::AgentKind;
    use agentdeck_core::hooks_server::PaneStatus;
    use agentdeck_core::logic::deck::{PaneSpec, new_pane, new_tab};

    use super::{DeckState, wall_items};

    #[test]
    fn sorts_by_priority_then_time_for_one_workspace() {
        let mut d = DeckState::default();
        let ws = d.workspaces[0].id.clone();
        let tab = d.tabs[0].id.clone();
        let other = d.link_workspace("C:\\b", None, None);
        let tab2 = new_tab("S2", &other.id, None);
        let tab2_id = d.add_tab(Some(tab2));
        let pane = |id: &str, status: PaneStatus, since: i64, exited: bool| {
            let mut spec = PaneSpec::new(AgentKind::Shell, id, "");
            spec.id = id.into();
            let mut p = new_pane(spec, since);
            p.status = status;
            p.exited = exited;
            p
        };
        for (id, s, since, ex) in [
            ("idle", PaneStatus::Idle, 1, false),
            ("working", PaneStatus::Working, 1, false),
            ("stopped", PaneStatus::Idle, 1, true),
            ("waitNew", PaneStatus::Waiting, 9, false),
            ("waitOld", PaneStatus::Waiting, 2, false),
            ("error", PaneStatus::Error, 1, false),
            ("done", PaneStatus::Done, 1, false),
        ] {
            d.add_pane(pane(id, s, since, ex), &tab);
        }
        d.add_pane(pane("elsewhere", PaneStatus::Waiting, 0, false), &tab2_id);
        let ids: Vec<String> = wall_items(&d, Some(&ws)).into_iter().map(|(p, _)| p).collect();
        assert_eq!(ids, ["waitOld", "waitNew", "error", "done", "working", "idle", "stopped"]);
        assert_eq!(wall_items(&d, None)[0].0, "elsewhere");
    }
}
