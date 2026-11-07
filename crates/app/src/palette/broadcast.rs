//! "Broadcast prompt": one prompt sent to several running agents of the current workspace
//! (components/Broadcast/Broadcast.tsx).

use std::collections::{HashMap, HashSet};

use agentdeck_core::agents::AgentKind;
use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::text::{AGENT_KINDS, agent_name};
use gpui_kit::assets::IconName;
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
    _subscriptions: Vec<Subscription>,
}

/// Opens the broadcast composer (TitleBar button, Ctrl+Shift+B, command palette).
pub fn open_broadcast(deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    let global = cx.global::<super::PaletteGlobal>();
    if global.broadcast.as_ref().and_then(WeakEntity::upgrade).is_some() {
        return;
    }
    let view = cx.new(|cx| Broadcast::new(deck, window, cx));
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

    fn submit(&mut self, cx: &mut Context<Self>) {
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
        let disabled = self.prompt(cx).is_empty() || chosen == 0 || self.sending;
        let list = if targets.is_empty() {
            div()
                .py_4()
                .text_center()
                .text_color(hsla(p.muted))
                .child("No running agents in this workspace.")
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
                    .child(quick)
                    .child(list)
                    .when(waiting > 0, |s| {
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
                        ui::primary_button("send", IconName::Send, format!("Send to {chosen}"), disabled, &p)
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
            )
    }
}
