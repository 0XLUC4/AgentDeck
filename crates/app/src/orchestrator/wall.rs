//! Wall mode: every pane of the workspace (or of all workspaces) as compact tiles, most urgent first.
//! Drawn in place of the session grid: the terminal views stay alive but are not laid out or painted.
//! Original: src/components/Wall/{Wall,wallState,jump}.tsx.

use std::collections::HashMap;
use std::time::Duration;

use agentdeck_core::hooks_server::{PaneStatus, Step, StepState};
use agentdeck_core::logic::deck::{ActivityKey, DeckState, Pane, activity_key};
use agentdeck_core::logic::text::{agent_name, format_cost, format_elapsed, format_since, format_tokens, now_ms};
use agentdeck_core::usage::{PaneUsage, pane_usage};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::base::Selectable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::panes::{agent_logo, status_color, status_label};
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

/// How often the tiles' token and cost figures are read again from the transcripts.
const REFRESH: Duration = Duration::from_secs(10);
/// Same for every tile: who, what, plan, then the figures.
const TILE_HEIGHT: f32 = 268.0;

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

/// Status filter of the wall: `None` shows everything, a status hides stopped agents too.
fn shown(pane: &Pane, filter: Option<PaneStatus>) -> bool {
    filter.is_none_or(|f| matches!(activity_key(pane), ActivityKey::Status(s) if s == f))
}

/// "Plan 2/5 · Next: label" for a pane with a task list (subagents are not plan entries).
fn plan(steps: &[Step]) -> Option<(usize, usize, Option<String>)> {
    let plan: Vec<&Step> = steps.iter().filter(|s| !s.agent).collect();
    if plan.is_empty() {
        return None;
    }
    let done = plan.iter().filter(|s| s.state == StepState::Done).count();
    let next = [StepState::Active, StepState::Pending]
        .iter()
        .find_map(|state| plan.iter().find(|s| s.state == *state));
    Some((done, plan.len(), next.map(|step| step.label.clone())))
}

pub struct Wall {
    deck: Entity<Deck>,
    /// Status chip selected in the top bar; `None` = All.
    filter: Option<PaneStatus>,
    /// Tokens, cost and conversation size per pane (Claude and Codex), read every 10 s.
    usage: HashMap<String, PaneUsage>,
    focus: FocusHandle,
    focused: bool,
    _refresh: Task<()>,
    _observe: Subscription,
}

impl Wall {
    pub fn new(deck: Entity<Deck>, cx: &mut Context<Self>) -> Self {
        let refresh = cx.spawn(async move |this, cx| {
            loop {
                let Ok(jobs) = this.update(cx, |wall, cx| wall.usage_jobs(cx)) else { break };
                // Transcripts are read off the UI thread.
                let loaded = cx
                    .background_spawn(async move {
                        jobs.into_iter()
                            .filter_map(|(id, cwd, started, agent)| {
                                let usage = pane_usage(id.clone(), cwd, started, agent).ok()?;
                                Some((id, usage))
                            })
                            .collect::<HashMap<_, _>>()
                    })
                    .await;
                let shown = this.update(cx, |wall, cx| {
                    wall.usage = loaded;
                    cx.notify();
                });
                if shown.is_err() {
                    break;
                }
                cx.background_executor().timer(REFRESH).await;
            }
        });
        Self {
            _observe: cx.observe(&deck, |_, _, cx| cx.notify()),
            deck,
            filter: None,
            usage: HashMap::new(),
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

    /// What reading each tile's usage needs: (pane id, folder, start time, agent).
    fn usage_jobs(&self, cx: &App) -> Vec<(String, String, i64, String)> {
        let d = self.deck.read(cx);
        self.items(cx)
            .into_iter()
            .filter_map(|(id, _)| {
                let pane = d.state.panes.get(&id)?;
                let agent = serde_json::to_value(pane.agent).ok()?.as_str()?.to_owned();
                let started = crate::panes::runtime::pane_started_at(&id, cx)?;
                Some((id, pane.cwd.clone(), started, agent))
            })
            .collect()
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
        let usage = self.usage.get(pane_id).filter(|u| u.totals.tokens > 0);
        let fill = usage.and_then(|u| crate::history::context_fill(u.context));
        let subagents = crate::panes::runtime::pane_subagents(&pane.id, cx).len();
        let none = || "-".to_owned();
        // (label, value, color): what the agent used and how far along it is.
        let stats: [(&str, String, u32); 6] = [
            ("Tokens", usage.map_or_else(none, |u| format_tokens(u.totals.tokens as f64)), p.fg),
            ("Cost", usage.filter(|u| u.totals.cost > 0.0).map_or_else(none, |u| format_cost(u.totals.cost)), p.fg),
            (
                "Context",
                fill.map_or_else(none, |(percent, _)| format!("{percent}%")),
                // Nearly full: the conversation is about to be compacted.
                if fill.is_some_and(|(percent, _)| percent >= 80) { p.wait } else { p.fg },
            ),
            ("Task time", pane.task_started_at.map_or_else(none, |start| format_elapsed(now - start)), p.fg),
            ("Tasks done", pane.completed_count.to_string(), p.fg),
            ("Subagents", subagents.to_string(), if subagents > 0 { p.busy } else { p.fg }),
        ];
        let models = usage.map(|u| u.models.join(", ")).unwrap_or_default();
        let mono = agentdeck_core::logic::theme::FONT_MONO;
        let color = status.map_or(p.subtle, |s| status_color(s, &p));
        // "Working", "Waiting"...: the first word of the status label.
        let word = status.map_or("Stopped", |s| status_label(s).split(' ').next().unwrap_or_default());
        let plan = plan(&crate::panes::runtime::pane_steps(&pane.id, cx));
        let (deck, tab_id, id) = (self.deck.clone(), tab.id.clone(), pane.id.clone());
        Some(
            div()
                .id(SharedString::from(format!("wall-{}", pane.id)))
                .flex()
                .flex_col()
                .gap(ui(10.0))
                .flex_grow(1.0)
                .flex_basis(ui(380.0))
                .min_w(ui(340.0))
                // Wider than this, a tile is mostly empty: more of them fit per row instead.
                .max_w(ui(560.0))
                .h(ui(TILE_HEIGHT))
                .overflow_hidden()
                .p(ui(14.0))
                .rounded(ui(12.0))
                .border_1()
                .border_color(hsla(if status == Some(PaneStatus::Waiting) { p.wait } else { p.line }))
                .bg(hsla(p.panel))
                .cursor_pointer()
                .hover(|s| s.border_color(hsla(p.line_strong)))
                .on_click(move |_, _, cx| jump_to_pane(&deck, &tab_id, &id, cx))
                // Who: logo, name, where it runs; its status and since when on the right.
                .child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap(ui(10.0))
                        .child(
                            div()
                                .flex()
                                .flex_shrink_0()
                                .items_center()
                                .justify_center()
                                .size(ui(32.0))
                                .rounded(ui(8.0))
                                .bg(hsla(p.raised))
                                .child(agent_logo(pane.agent, 18.0, None, &p)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(ui(14.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(hsla(p.fg))
                                        .child(name),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .text_color(hsla(p.subtle))
                                        .child(format!("{} \u{203a} {}", ws.name, tab.name)),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_shrink_0()
                                .items_end()
                                .gap(ui(2.0))
                                .child(
                                    div()
                                        .rounded_full()
                                        .px(ui(8.0))
                                        .py(ui(1.0))
                                        .text_size(ui(11.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .bg(hsla_a(color, 0.15))
                                        .text_color(hsla(color))
                                        .child(word),
                                )
                                .child(
                                    div()
                                        .text_size(ui(11.0))
                                        .text_color(hsla(p.subtle))
                                        .child(format_since(now - pane.status_since)),
                                ),
                        ),
                )
                // What: the task, then the last action. Fixed height so every tile lines up.
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_shrink_0()
                        .gap(ui(2.0))
                        .h(ui(38.0))
                        .child(match pane.topic.clone() {
                            Some(topic) => div().truncate().text_color(hsla(p.fg)).child(topic),
                            None => div().truncate().text_color(hsla(p.subtle)).child("No task yet"),
                        })
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(hsla(p.muted))
                                .child(pane.last_action.clone().unwrap_or_default()),
                        ),
                )
                // How far: the plan as a bar, with the step to do next.
                .child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_2()
                        .h(ui(16.0))
                        .text_xs()
                        .text_color(hsla(p.subtle))
                        .when_some(plan, |el, (done, total, next)| {
                            el.child(
                                div()
                                    .w(ui(72.0))
                                    .h(ui(4.0))
                                    .flex_shrink_0()
                                    .overflow_hidden()
                                    .rounded_full()
                                    .bg(hsla(p.raised))
                                    .child(div().h_full().rounded_full().bg(hsla(p.ok)).w(relative(done as f32 / total as f32))),
                            )
                            .child(div().flex_shrink_0().font_family(mono).text_color(hsla(p.muted)).child(format!("{done}/{total}")))
                            .children(next.map(|label| div().min_w_0().flex_1().truncate().child(label)))
                        }),
                )
                .child(div().h_px().flex_shrink_0().bg(hsla(p.line)))
                // The figures, three per row.
                .child(div().flex().flex_wrap().flex_shrink_0().gap_y(ui(10.0)).children(stats.into_iter().map(
                    |(label, value, color)| {
                        div()
                            .flex()
                            .flex_col()
                            .w(relative(1.0 / 3.0))
                            .min_w_0()
                            .child(div().text_size(ui(11.0)).text_color(hsla(p.subtle)).child(label))
                            .child(div().truncate().font_family(mono).text_color(hsla(color)).child(value))
                    },
                )))
                .child(div().mt_auto().flex_shrink_0().truncate().text_size(ui(11.0)).text_color(hsla(p.subtle)).child(models)),
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
        let filter = self.filter;
        let mut items = self.items(cx);
        let panes = &self.deck.read(cx).state.panes;
        items.retain(|(pane, _)| panes.get(pane).is_some_and(|pane| shown(pane, filter)));
        let count = items.len();
        let tiles: Vec<_> = items
            .iter()
            .filter_map(|(pane, tab)| self.tile(pane, tab, now, cx))
            .collect();
        let chips = [
            ("All", None),
            ("Working", Some(PaneStatus::Working)),
            ("Waiting", Some(PaneStatus::Waiting)),
            ("Done", Some(PaneStatus::Done)),
            ("Error", Some(PaneStatus::Error)),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (label, status))| {
            Button::new(("wall-filter", index))
                .ghost()
                .small()
                .label(label)
                .selected(filter == status)
                .on_click(cx.listener(move |wall, _, _, cx| {
                    wall.filter = status;
                    cx.notify();
                }))
        })
        .collect::<Vec<_>>();
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
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(hsla(p.fg))
                            .child(format!(
                                "Wall \u{b7} {}",
                                if all { "all projects" } else { ws_name.as_str() }
                            ))
                            .child(div().text_color(hsla(p.subtle)).child(count.to_string())),
                    )
                    .child(div().flex().items_center().gap_1().children(chips))
                    .child(div().flex_1())
                    .child(
                        Checkbox::new("wall-all-workspaces")
                            .checked(all)
                            .label("All projects")
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
                            .child(if filter.is_some() { "No agents with this status." } else { "No agents running." })
                    } else {
                        div().flex().flex_wrap().gap(ui(6.0)).children(tiles)
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use agentdeck_core::agents::AgentKind;
    use agentdeck_core::hooks_server::{PaneStatus, Step, StepState};
    use agentdeck_core::logic::deck::{PaneSpec, new_pane, new_tab};

    use super::{DeckState, plan, shown, wall_items};

    #[test]
    fn plan_line_counts_tasks_and_names_the_next_step() {
        let step = |label: &str, state, agent| Step { id: label.into(), label: label.into(), state, agent };
        assert_eq!(plan(&[step("sub", StepState::Active, true)]), None);
        let steps = [
            step("a", StepState::Done, false),
            step("b", StepState::Pending, false),
            step("c", StepState::Active, false),
            step("sub", StepState::Active, true),
        ];
        assert_eq!(plan(&steps), Some((1, 3, Some("c".to_owned()))));
        assert_eq!(plan(&steps[..1]), Some((1, 1, None)));
    }

    #[test]
    fn status_filter_hides_other_statuses_and_stopped_agents() {
        let mut pane = new_pane(PaneSpec::new(AgentKind::Shell, "p", ""), 0);
        pane.status = PaneStatus::Working;
        assert!(shown(&pane, None) && shown(&pane, Some(PaneStatus::Working)));
        assert!(!shown(&pane, Some(PaneStatus::Done)));
        pane.exited = true;
        assert!(shown(&pane, None) && !shown(&pane, Some(PaneStatus::Working)));
    }

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
