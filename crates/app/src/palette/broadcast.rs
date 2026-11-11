//! "Broadcast prompt": one prompt sent to several running agents of the current workspace
//! (components/Broadcast/Broadcast.tsx). Race mode: the prompt goes to N fresh agents instead,
//! each in its own git worktree, to compare their results in the Git panel.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use agentdeck_core::agents::{AgentKind, detect_agents};
use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::PaneSpec;
use agentdeck_core::logic::queue::{ADVANCE_DELAY_MS, race_title};
use agentdeck_core::logic::text::{AGENT_KINDS, agent_name, now_ms, split_args};
use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::ui;
use crate::panes::{status_color, status_label};
use crate::state::Deck;
use crate::theme::{hsla, palette};

/// A running pane of the active workspace, with its session name.
struct Target {
    id: String,
    agent: AgentKind,
    title: String,
    status: PaneStatus,
    tab_id: String,
    tab_name: String,
}

fn targets(deck: &Deck) -> Vec<Target> {
    let s = &deck.state;
    let ws = &s.active_workspace().id;
    s.tabs
        .iter()
        .filter(|t| &t.workspace_id == ws)
        .flat_map(|tab| {
            tab.pane_ids.iter().filter_map(|id| {
                let pane = s.panes.get(id).filter(|p| !p.exited)?;
                Some(Target {
                    id: id.clone(),
                    agent: pane.agent,
                    title: if pane.title.is_empty() { agent_name(pane.agent).to_owned() } else { pane.title.clone() },
                    status: pane.status,
                    tab_id: tab.id.clone(),
                    tab_name: tab.name.clone(),
                })
            })
        })
        .collect()
}

pub struct Broadcast {
    deck: Entity<Deck>,
    text: Entity<TextareaState>,
    selected: HashSet<String>,
    results: HashMap<String, Result<(), String>>,
    sending: bool,
    show_recent: bool,
    race: bool,
    race_agent: AgentKind,
    race_count: usize,
    /// Installed agent CLIs; None until detected (every agent is offered meanwhile).
    installed: Option<Vec<AgentKind>>,
    _detect: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

const RACE_POLL: Duration = Duration::from_millis(200);
/// A worktree setup command (dependency install) may run before the agent starts.
const RACE_START_TIMEOUT: Duration = Duration::from_secs(600);

/// Starts one race agent and submits the prompt once its start screen stopped drawing.
/// False when it failed to start, was closed, or never got ready.
pub(crate) async fn race_one(deck: Entity<Deck>, spec: PaneSpec, tab_id: String, text: String, cx: &mut AsyncApp) -> bool {
    use crate::orchestrator::runtime;
    let id = spec.id.clone();
    // False = still queued or setting up its worktree: the loop below keeps waiting.
    runtime::create_pane(&deck, spec, tab_id, cx).await;
    let mut waited = Duration::ZERO;
    while waited < RACE_START_TIMEOUT {
        if deck.read_with(cx, |d, _| d.state.panes.get(&id).is_none_or(|p| p.exited)) {
            return false;
        }
        let last_output = cx.update(|cx| crate::panes::runtime::pane_last_output_at(&id, cx));
        if last_output != 0 && now_ms() - last_output >= ADVANCE_DELAY_MS as i64 {
            cx.update(|cx| runtime::send_prompt(&id, &text, true, cx));
            return true;
        }
        cx.background_executor().timer(RACE_POLL).await;
        waited += RACE_POLL;
    }
    false
}

/// Starts the last used agent on the GitHub issue or pull request whose link is in the clipboard.
pub fn agent_from_issue(deck: &Entity<Deck>, cx: &mut App) {
    let url = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
    let agent = crate::panes::last_agent(cx);
    let d = deck.read(cx);
    let path = d.state.active_workspace().path.clone();
    let cwd = if path.is_empty() { d.state.last_cwd.clone() } else { path };
    let defaults = d.settings.agent(agent);
    let font_size = d.settings.font_size;
    let Some(tab_id) = d.state.active_tab().map(|t| t.id.clone()).filter(|_| !cwd.is_empty()) else {
        deck.update(cx, |d, cx| d.toast("Link a project first: the agent works in its folder.", cx));
        return;
    };
    let deck = deck.clone();
    cx.spawn(async move |cx| {
        let dir = cwd.clone();
        let fetched = cx.background_executor().spawn(async move { agentdeck_core::git::gh_issue_prompt(dir, url) }).await;
        let failure = match fetched {
            Err(e) => Some(format!("GitHub issue: {e}. Copy the issue's link first.")),
            Ok((title, prompt)) => {
                let mut spec = PaneSpec::new(agent, title, cwd);
                spec.font_size = font_size;
                spec.args = split_args(&defaults.args);
                spec.worktree = defaults.worktree;
                let started = race_one(deck.clone(), spec, tab_id, prompt, cx).await;
                (!started).then(|| "The agent did not start: the issue was not sent.".to_owned())
            }
        };
        if let Some(message) = failure {
            cx.update(|cx| deck.update(cx, |d, cx| d.toast(message, cx)));
        }
    })
    .detach();
}

/// Opens the broadcast composer (TitleBar button, Ctrl+Shift+B, command palette).
pub fn open_broadcast(deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    open(deck, false, window, cx);
}

/// Opens the composer with Race preselected (command palette).
pub fn open_race(deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    open(deck, true, window, cx);
}

fn open(deck: Entity<Deck>, race: bool, window: &mut Window, cx: &mut App) {
    let global = cx.global::<super::PaletteGlobal>();
    if global.broadcast.as_ref().and_then(WeakEntity::upgrade).is_some() {
        return;
    }
    let view = cx.new(|cx| {
        let mut view = Broadcast::new(deck, window, cx);
        if race {
            view.toggle_race(cx);
        }
        view
    });
    cx.global_mut::<super::PaletteGlobal>().broadcast = Some(view.downgrade());
    let focus = view.read(cx).text.focus_handle(cx);
    super::open_modal(view, 640.0, None, focus, window, cx);
}

impl Broadcast {
    fn new(deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = cx.new(|cx| TextareaState::new(window, cx).placeholder("Prompt sent to every selected agent…"));
        let subscription = cx.subscribe(&text, |this, _, event: &InputEvent, cx| {
            if let InputEvent::PressEnter { secondary: true, .. } = event {
                this.submit(cx);
            }
            cx.notify();
        });
        let mut this = Self {
            deck,
            text,
            selected: HashSet::new(),
            results: HashMap::new(),
            sending: false,
            show_recent: false,
            race: false,
            race_agent: AgentKind::Claude,
            race_count: 2,
            installed: None,
            _detect: None,
            _subscriptions: vec![subscription],
        };
        this.selected = this.session_ids(cx);
        this
    }

    /// Running panes of the visible session.
    fn session_ids(&self, cx: &App) -> HashSet<String> {
        let deck = self.deck.read(cx);
        let active = &deck.state.active_tab_id;
        targets(deck)
            .into_iter()
            .filter(|t| &t.tab_id == active)
            .map(|t| t.id)
            .collect()
    }

    fn chosen(&self, cx: &App) -> Vec<String> {
        targets(self.deck.read(cx))
            .into_iter()
            .filter(|t| self.selected.contains(&t.id))
            .map(|t| t.id)
            .collect()
    }

    fn prompt(&self, cx: &App) -> String {
        // Ctrl+Enter also typed a newline into the textarea: trimming drops it.
        self.text.read(cx).value().trim().to_owned()
    }

    /// Agents a race can start: the installed CLIs, every one while detection runs.
    fn race_kinds(&self) -> Vec<AgentKind> {
        AGENT_KINDS
            .into_iter()
            .filter(|k| *k != AgentKind::Shell && self.installed.as_ref().is_none_or(|i| i.contains(k)))
            .collect()
    }

    fn toggle_race(&mut self, cx: &mut Context<Self>) {
        self.race = !self.race;
        if self.race && self._detect.is_none() {
            self._detect = Some(cx.spawn(async move |this, cx| {
                let infos = cx.background_spawn(async { detect_agents() }).await;
                this.update(cx, |this, cx| {
                    this.installed = Some(infos.into_iter().filter(|i| i.installed).map(|i| i.kind).collect());
                    let kinds = this.race_kinds();
                    if !kinds.contains(&this.race_agent) {
                        this.race_agent = kinds.first().copied().unwrap_or(this.race_agent);
                    }
                    cx.notify();
                })
                .ok();
            }));
        }
        cx.notify();
    }

    /// Creates `race_count` agents in the visible session, one worktree each, and sends them the prompt.
    fn start_race(&mut self, cx: &mut Context<Self>) {
        let text = self.prompt(cx);
        if text.is_empty() || self.sending || !self.race_kinds().contains(&self.race_agent) {
            return;
        }
        let deck = self.deck.clone();
        let d = deck.read(cx);
        let cwd = d.state.active_workspace().path.clone();
        let tab_id = d.state.active_tab_id.clone();
        let args = split_args(&d.settings.agent(self.race_agent).args);
        let font_size = d.settings.font_size;
        if cwd.is_empty() {
            deck.update(cx, |d, cx| d.toast("Race needs a project folder: link one first.", cx));
            return;
        }
        let (agent, count) = (self.race_agent, self.race_count);
        self.sending = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let folder = cwd.clone();
            let is_git = cx
                .background_spawn(async move {
                    agentdeck_core::workspace::inspect_folder(folder).is_ok_and(|f| f.is_git)
                })
                .await;
            if !is_git {
                deck.update(cx, |d, cx| {
                    d.toast(format!("Race needs a git repository (one worktree per agent): {cwd} is not one."), cx)
                });
                this.update(cx, |this, cx| {
                    this.sending = false;
                    cx.notify();
                })
                .ok();
                return;
            }
            cx.update(|cx| {
                super::remember_prompt(&deck, &text, cx);
                // The new panes appear behind the dialog: get out of their way.
                super::with_window(cx, |_, window, cx| window.close_dialog(cx));
            });
            let agents: Vec<Task<bool>> = (1..=count)
                .map(|n| {
                    let mut spec = PaneSpec::new(agent, race_title(n, &text), cwd.clone());
                    spec.args = args.clone();
                    spec.worktree = true;
                    spec.font_size = font_size;
                    let (deck, tab_id, text) = (deck.clone(), tab_id.clone(), text.clone());
                    cx.spawn(async move |cx| race_one(deck, spec, tab_id, text, cx).await)
                })
                .collect();
            let mut started = 0;
            for agent in agents {
                started += usize::from(agent.await);
            }
            // A failed start already got its own toast from the pane runtime.
            deck.update(cx, |d, cx| {
                d.toast(
                    format!("Race: {started} of {count} agents got the prompt. Compare and merge them from the Git panel."),
                    cx,
                )
            });
        })
        .detach();
    }

    fn race_options(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let workspace = self.deck.read(cx).state.active_workspace().name.clone();
        let picked = |chip: Stateful<Div>, on: bool| chip.when(on, |c| c.border_color(hsla(p.accent)).text_color(hsla(p.fg)));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .children(self.race_kinds().into_iter().map(|kind| {
                        picked(ui::chip(SharedString::from(format!("race-{}", agent_name(kind))), &p), self.race_agent == kind)
                            .child(ui::agent_mark(kind, 12.0, &p))
                            .child(agent_name(kind))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.race_agent = kind;
                                cx.notify();
                            }))
                    }))
                    .children((2..=4usize).map(|n| {
                        picked(ui::chip(("race-count", n), &p), self.race_count == n)
                            .when(n == 2, |c| c.ml_auto())
                            .child(format!("{n} agents"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.race_count = n;
                                cx.notify();
                            }))
                    })),
            )
            .child(div().text_xs().text_color(hsla(p.subtle)).child(format!(
                "Starts new agents in {workspace}, each in its own Git worktree. Compare and merge them from the Git panel."
            )))
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.race {
            return self.start_race(cx);
        }
        let text = self.prompt(cx);
        let chosen = self.chosen(cx);
        if text.is_empty() || chosen.is_empty() || self.sending {
            return;
        }
        self.sending = true;
        cx.notify();
        // Codex/Gemini/shell send no prompt hook: remember it here too.
        super::remember_prompt(&self.deck, &text, cx);
        let backend = self.deck.read(cx).backend.clone();
        let executor = cx.background_executor().clone();
        let sends: Vec<Task<(String, Result<(), String>)>> = chosen
            .into_iter()
            .map(|id| {
                let send = super::send_prompt(backend.clone(), executor.clone(), id.clone(), text.clone());
                executor.spawn(async move { (id, send.await) })
            })
            .collect();
        cx.spawn(async move |this, cx| {
            let mut results = HashMap::new();
            for send in sends {
                let (id, result) = send.await;
                results.insert(id, result);
            }
            this.update(cx, |this, cx| {
                this.results = results;
                this.sending = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select(&mut self, ids: HashSet<String>, cx: &mut Context<Self>) {
        self.selected = ids;
        cx.notify();
    }

    fn quick_select(&self, targets: &[Target], cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let all: HashSet<String> = targets.iter().map(|t| t.id.clone()).collect();
        let kinds = AGENT_KINDS.into_iter().filter(|k| targets.iter().any(|t| t.agent == *k));
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1p5()
            .child(
                ui::chip("all", &p)
                    .child("All")
                    .on_click(cx.listener(move |this, _, _, cx| this.select(all.clone(), cx))),
            )
            .child(ui::chip("session", &p).child("Current session").on_click(cx.listener(|this, _, _, cx| {
                let ids = this.session_ids(cx);
                this.select(ids, cx);
            })))
            .children(kinds.map(|kind| {
                let ids: HashSet<String> = targets.iter().filter(|t| t.agent == kind).map(|t| t.id.clone()).collect();
                ui::chip(SharedString::from(format!("kind-{}", agent_name(kind))), &p)
                    .child(ui::agent_mark(kind, 12.0, &p))
                    .child(agent_name(kind))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(ids.clone(), cx)))
            }))
            .child(
                ui::chip("none", &p)
                    .ml_auto()
                    .child("None")
                    .on_click(cx.listener(|this, _, _, cx| this.select(HashSet::new(), cx))),
            )
    }

    fn target_row(&self, target: &Target, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette(cx);
        let checked = self.selected.contains(&target.id);
        let id = target.id.clone();
        let status = target.status;
        let result = self.results.get(&target.id).cloned();
        let hover = hsla(p.hover);
        div()
            .id(SharedString::from(format!("target-{}", target.id)))
            .flex()
            .items_center()
            .gap_2p5()
            .h(crate::theme::ui(36.0))
            .px_3()
            .border_b_1()
            .border_color(hsla(p.line))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.selected.remove(&id) {
                    this.selected.insert(id.clone());
                }
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .size_4()
                    .rounded(crate::theme::ui(4.0))
                    .border_1()
                    .text_size(crate::theme::ui(12.0))
                    .when(checked, |s| {
                        s.bg(hsla(p.accent))
                            .border_color(hsla(p.accent))
                            .text_color(hsla(p.on_accent))
                            .child(IconName::Check)
                    })
                    .when(!checked, |s| s.border_color(hsla(p.line_strong))),
            )
            .child(ui::agent_mark(target.agent, 14.0, &p))
            .child(div().truncate().text_color(hsla(p.fg)).child(target.title.clone()))
            .child(div().truncate().text_xs().text_color(hsla(p.subtle)).child(target.tab_name.clone()))
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .when(status == PaneStatus::Waiting, |s| {
                        s.child(div().text_color(hsla(p.wait)).child("waiting for a reply"))
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("dot-{}", target.id)))
                            .size(crate::theme::ui(8.0))
                            .rounded_full()
                            .bg(hsla(status_color(status, &p)))
                            .tooltip(move |window, cx| Tooltip::new(status_label(status)).build(window, cx)),
                    )
                    .children(result.map(|r| match r {
                        Ok(()) => div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_color(hsla(p.ok))
                            .child(IconName::Check)
                            .child("Sent")
                            .into_any_element(),
                        Err(e) => div()
                            .id(SharedString::from(format!("err-{}", target.id)))
                            .flex()
                            .items_center()
                            .gap_1()
                            .max_w(crate::theme::ui(192.0))
                            .text_color(hsla(p.bad))
                            .child(IconName::CircleAlert)
                            .child(div().truncate().child(e.clone()))
                            .tooltip(move |window, cx| Tooltip::new(e.clone()).build(window, cx))
                            .into_any_element(),
                    })),
            )
    }

    fn recent_prompts(&self, cx: &mut Context<Self>) -> Option<Div> {
        let p = palette(cx);
        let recent = self.deck.read(cx).recent_prompts.clone();
        if recent.is_empty() {
            return None;
        }
        let line_strong = hsla(p.line_strong);
        let hover = hsla(p.hover);
        let fg = hsla(p.fg);
        let toggle = div()
            .id("recent-toggle")
            .flex()
            .items_center()
            .justify_between()
            .h(crate::theme::ui(32.0))
            .px_2()
            .rounded(crate::theme::ui(6.0))
            .border_1()
            .border_color(hsla(p.line))
            .bg(hsla(p.base))
            .text_xs()
            .text_color(hsla(p.muted))
            .cursor_pointer()
            .hover(move |s| s.border_color(line_strong))
            .child("Recent prompts…")
            .child(IconName::ChevronDown)
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_recent = !this.show_recent;
                cx.notify();
            }));
        let list = self.show_recent.then(|| {
            div()
                .id("recent-list")
                .flex()
                .flex_col()
                .max_h(crate::theme::ui(192.0))
                .overflow_y_scroll()
                .rounded(crate::theme::ui(6.0))
                .border_1()
                .border_color(hsla(p.line))
                .bg(hsla(p.base))
                .children(recent.into_iter().enumerate().map(|(i, prompt)| {
                    let shown = if prompt.chars().count() > 90 {
                        format!("{}…", prompt.chars().take(90).collect::<String>())
                    } else {
                        prompt.clone()
                    };
                    div()
                        .id(i)
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(hsla(p.muted))
                        .truncate()
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover).text_color(fg))
                        .child(shown)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let value = prompt.clone();
                            this.text.update(cx, |t, cx| t.set_value(value, window, cx));
                            this.show_recent = false;
                            cx.notify();
                        }))
                }))
        });
        Some(div().flex().flex_col().gap_1().child(toggle).children(list))
    }
}

impl Render for Broadcast {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let (workspace, targets) = {
            let deck = self.deck.read(cx);
            (deck.state.active_workspace().name.clone(), targets(deck))
        };
        let chosen = targets.iter().filter(|t| self.selected.contains(&t.id)).count();
        let waiting = targets
            .iter()
            .filter(|t| self.selected.contains(&t.id) && t.status == PaneStatus::Waiting)
            .count();
        let race = self.race;
        let no_target = if race { !self.race_kinds().contains(&self.race_agent) } else { chosen == 0 };
        let disabled = self.prompt(cx).is_empty() || no_target || self.sending;
        let send_label = if race { format!("Race {} agents", self.race_count) } else { format!("Send to {chosen}") };
        let race_toggle = ui::chip("race", &p)
            .when(race, |c| c.border_color(hsla(p.accent)).text_color(hsla(p.fg)))
            .child(IconName::Flag)
            .child("Race: new agents, one worktree each")
            .on_click(cx.listener(|this, _, _, cx| this.toggle_race(cx)));
        let race_options = race.then(|| self.race_options(cx));
        let list = if targets.is_empty() {
            div()
                .py_4()
                .text_center()
                .text_color(hsla(p.muted))
                .child("No running agents in this project.")
                .into_any_element()
        } else {
            div()
                .id("targets")
                .flex()
                .flex_col()
                .max_h(crate::theme::ui(256.0))
                .overflow_y_scroll()
                .rounded(crate::theme::ui(6.0))
                .border_1()
                .border_color(hsla(p.line))
                .children(targets.iter().map(|t| self.target_row(t, cx)))
                .into_any_element()
        };
        let recent = self.recent_prompts(cx);
        let quick = self.quick_select(&targets, cx);
        div()
            .flex()
            .flex_col()
            .w_full()
            .text_size(crate::theme::ui(14.0))
            .child(ui::header("Broadcast prompt", format!("Running agents in {workspace}"), &p))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_5()
                    .children(recent)
                    .child(Textarea::new(&self.text).h(crate::theme::ui(120.0)))
                    .child(div().flex().child(race_toggle))
                    .children(race_options)
                    .when(!race, |s| s.child(quick).child(list))
                    .when(!race && waiting > 0, |s| {
                        s.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_xs()
                                .text_color(hsla(p.wait))
                                .child(IconName::CircleAlert)
                                .child(format!(
                                    "{waiting} agent(s) waiting for a reply: the prompt will arrive as their answer."
                                )),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_3()
                    .border_t_1()
                    .border_color(hsla(p.line))
                    .px_5()
                    .py_3()
                    .child(div().mr_auto().text_xs().text_color(hsla(p.subtle)).child("Ctrl+Enter to send"))
                    .child(
                        ui::primary_button("send", IconName::Send, send_label, disabled, &p)
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
            )
    }
}
