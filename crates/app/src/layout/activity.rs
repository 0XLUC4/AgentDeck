//! Activity panel (app/Activity.tsx) and its task detail slide-over (app/TaskDetail.tsx): every agent
//! of every workspace grouped by state, permission cards inline, and what a finished agent did.

use std::collections::{HashMap, HashSet};

use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::hooks_server::permissions::PermissionRequest;
use agentdeck_core::logic::deck::{ACTIVITY_ORDER, ActivityKey, Pane, activity_key};
use agentdeck_core::logic::queue::prompt_text;
use agentdeck_core::logic::tasks::Task;
use agentdeck_core::logic::text::{agent_name, format_elapsed, format_since, now_ms};
use agentdeck_core::logic::theme::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::DeckView;
use super::chrome::icon_button;
use crate::panes::{PermissionCard, PermissionCardEvent, agent_logo, runtime, status_color};
use crate::theme::{hsla, palette, ui};

const LONG_PROMPT: usize = 400;
const COPIED_MS: i64 = 1500;

/// Per-view state of the panel.
#[derive(Default)]
pub(super) struct ActivityState {
    /// Pane whose task detail is open.
    detail: Option<String>,
    /// Expanded long prompts and previous tasks, by key.
    expanded: HashSet<String>,
    copied_at: i64,
    cards: HashMap<String, (Entity<PermissionCard>, Subscription)>,
}

fn activity_label(key: ActivityKey) -> &'static str {
    match key {
        ActivityKey::Status(PaneStatus::Waiting) => "Waiting",
        ActivityKey::Status(PaneStatus::Error) => "Error",
        ActivityKey::Status(PaneStatus::Done) => "Done",
        ActivityKey::Status(PaneStatus::Working) => "Working",
        ActivityKey::Status(PaneStatus::Idle) => "Idle",
        ActivityKey::Exited => "Stopped",
    }
}

fn pane_name(pane: &Pane) -> String {
    if pane.title.is_empty() { agent_name(pane.agent).to_owned() } else { pane.title.clone() }
}

fn action_icon(text: &str) -> IconName {
    let tool = text.split(|c: char| !c.is_alphanumeric()).next().unwrap_or("");
    match tool {
        "Bash" => IconName::Terminal,
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => IconName::FilePen,
        "Read" | "Grep" | "Glob" => IconName::Search,
        _ => IconName::Dot,
    }
}

fn small_button(id: impl Into<ElementId>, icon: IconName, label: impl Into<SharedString>, p: &Palette) -> Stateful<Div> {
    let (hover, fg) = (hsla(p.hover), hsla(p.fg));
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(ui(6.0))
        .h(ui(28.0))
        .px_2()
        .rounded(ui(6.0))
        .border_1()
        .border_color(hsla(p.line))
        .text_size(ui(12.0))
        .text_color(hsla(p.muted))
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(fg))
        .child(Icon::new(icon).size(ui(13.0)))
        .child(label.into())
}

fn section(title: String, p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(ui(6.0))
        .border_b_1()
        .border_color(hsla(p.line))
        .px_3()
        .py(ui(10.0))
        .child(div().text_size(ui(12.0)).text_color(hsla(p.subtle)).child(title))
}

fn note(text: impl Into<SharedString>, p: &Palette) -> Div {
    div().text_size(ui(12.0)).text_color(hsla(p.subtle)).child(text.into())
}

impl DeckView {
    fn permission_card(&mut self, request: &PermissionRequest, cx: &mut Context<Self>) -> Entity<PermissionCard> {
        if let Some((card, _)) = self.activity.cards.get(&request.request_id) {
            return card.clone();
        }
        let card = cx.new(|_| PermissionCard::new(request.clone()));
        let pane_id = request.pane_id.clone();
        let sub = cx.subscribe(&card, move |this, _, _: &PermissionCardEvent, cx| this.jump(&pane_id, cx));
        self.activity.cards.insert(request.request_id.clone(), (card.clone(), sub));
        card
    }

    /// Right column: every agent of every workspace, the ones needing attention first.
    pub(super) fn activity_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let now = now_ms();
        let pending: Vec<PermissionRequest> = self.deck.read(cx).permissions.clone();
        self.activity.cards.retain(|id, _| pending.iter().any(|r| &r.request_id == id));
        let cards: HashMap<String, Entity<PermissionCard>> =
            pending.iter().map(|r| (r.request_id.clone(), self.permission_card(r, cx))).collect();
        let detail = self.activity.detail.clone().filter(|id| self.deck.read(cx).state.panes.contains_key(id));
        let detail_view = detail.map(|id| self.task_detail(&id, now, cx));

        let deck = self.deck.read(cx);
        let groups = deck.state.activity_groups();
        let mut list = div().id("activity-list").flex().flex_col().flex_1().min_h_0().overflow_y_scroll();
        if groups.iter().all(|(_, items)| items.is_empty()) {
            list = list.child(
                note("No agents running. Add one with the + in the session bar, or Ctrl+K.", &p).px_3().py_4(),
            );
        }
        for key in ACTIVITY_ORDER {
            let Some((_, items)) = groups.iter().find(|(k, _)| *k == key) else { continue };
            if items.is_empty() {
                continue;
            }
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(hsla(p.line))
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .text_size(ui(12.0))
                    .text_color(hsla(p.subtle))
                    .child(activity_label(key))
                    .child(div().ml_auto().child(items.len().to_string())),
            );
            for (pane, tab, workspace) in items {
                let active = deck.state.active_pane_id.as_deref() == Some(pane.id.as_str());
                let key = activity_key(pane);
                let name = pane_name(pane);
                let requests: Vec<&PermissionRequest> = pending.iter().filter(|r| r.pane_id == pane.id).collect();
                let id = pane.id.clone();
                let dot = match key {
                    ActivityKey::Exited => div()
                        .size(ui(8.0))
                        .flex_shrink_0()
                        .rounded_full()
                        .border_1()
                        .border_color(hsla(p.line_strong)),
                    ActivityKey::Status(status) => {
                        div().size(ui(8.0)).flex_shrink_0().rounded_full().bg(hsla(status_color(status, &p)))
                    }
                };
                let main = div()
                    .id(ElementId::Name(format!("activity-{}", pane.id).into()))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(ui(2.0))
                    .py_2()
                    .pl_3()
                    .pr_2()
                    .cursor_pointer()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(agent_logo(pane.agent, 14.0, None, &p))
                            .when(pane.orchestrator, |el| el.child(crate::orchestrator::orchestrator_crown(12.0, cx)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(hsla(if active { p.fg } else { p.muted }))
                                    .child(name.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(ui(12.0))
                                    .text_color(hsla(p.subtle))
                                    .child(format_since(now - pane.status_since)),
                            )
                            .child(dot),
                    )
                    .child(
                        div()
                            .truncate()
                            .pl(ui(22.0))
                            .text_size(ui(12.0))
                            .text_color(hsla(p.subtle))
                            .child(format!("{} › {}", workspace.name, tab.name)),
                    )
                    .when_some(pane.last_action.clone(), |el, action| {
                        let hint: SharedString = action.clone().into();
                        el.child(
                            div()
                                .id(ElementId::Name(format!("activity-action-{}", pane.id).into()))
                                .truncate()
                                .pl(ui(22.0))
                                .text_size(ui(12.0))
                                .text_color(hsla(p.muted))
                                .child(action)
                                .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx)),
                        )
                    })
                    .on_click(cx.listener({
                        let id = id.clone();
                        move |this, _: &ClickEvent, _, cx| this.jump(&id, cx)
                    }));
                let side = match key {
                    ActivityKey::Status(PaneStatus::Done) => Some(
                        div()
                            .mt(ui(6.0))
                            .mr(ui(6.0))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(
                                icon_button(
                                    ElementId::Name(format!("activity-detail-{id}").into()),
                                    IconName::Check,
                                    "See what it finished",
                                    &p,
                                )
                                .size(ui(24.0))
                                .on_click(cx.listener({
                                    let id = id.clone();
                                    move |this, _: &ClickEvent, _, cx| {
                                        this.activity.detail = Some(id.clone());
                                        cx.notify();
                                    }
                                })),
                            )
                            .child(
                                icon_button(
                                    ElementId::Name(format!("activity-dismiss-{id}").into()),
                                    IconName::EyeOff,
                                    "Dismiss: set the agent back to idle without opening the details",
                                    &p,
                                )
                                .size(ui(24.0))
                                .on_click(cx.listener({
                                    let id = id.clone();
                                    move |this, _: &ClickEvent, _, cx| this.acknowledge(&id, cx)
                                })),
                            ),
                    ),
                    // Agents without a permission hook: answer in the terminal.
                    ActivityKey::Status(PaneStatus::Waiting) if requests.is_empty() => Some(
                        div().mt(ui(6.0)).mr(ui(6.0)).child(
                            icon_button(
                                ElementId::Name(format!("activity-term-{id}").into()),
                                IconName::SquareTerminal,
                                "Go to the terminal to answer",
                                &p,
                            )
                            .size(ui(24.0))
                            .on_click(cx.listener({
                                let id = id.clone();
                                move |this, _: &ClickEvent, _, cx| this.jump(&id, cx)
                            })),
                        ),
                    ),
                    _ => None,
                };
                let hover = hsla(p.hover);
                list = list.child(
                    div()
                        .flex()
                        .flex_col()
                        .border_b_1()
                        .border_color(hsla(p.line))
                        .when(active, |el| el.bg(hsla(p.raised)))
                        .when(!active, |el| el.hover(move |s| s.bg(hover)))
                        .child(div().flex().items_start().child(main).children(side))
                        .when(!requests.is_empty(), |el| {
                            el.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(ui(6.0))
                                    .px_2()
                                    .pb_2()
                                    .children(requests.iter().filter_map(|r| cards.get(&r.request_id).cloned())),
                            )
                        }),
                );
            }
        }
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(ui(260.0))
            .border_l_1()
            .border_color(hsla(p.line))
            .bg(hsla(p.panel))
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .h(ui(40.0))
                    .pl_3()
                    .pr_2()
                    .child(
                        div()
                            .text_size(ui(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(hsla(p.subtle))
                            .child("Activity"),
                    )
                    .child(
                        icon_button("activity-close", IconName::X, "Hide activity (Ctrl+Shift+A)", &p).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.activity_open = false;
                                cx.notify();
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(hsla(p.line))
                    .child(list)
                    .children(detail_view),
            )
            .into_any_element()
    }

    fn acknowledge(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        let id = pane_id.to_owned();
        self.deck.update(cx, |d, cx| {
            d.state.acknowledge(&id, now_ms());
            d.save(cx);
            cx.notify();
        });
    }

    fn close_detail(&mut self, cx: &mut Context<Self>) {
        self.activity.detail = None;
        self.activity.expanded.clear();
        cx.notify();
    }

    fn toggle_expanded(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.activity.expanded.remove(&key) {
            self.activity.expanded.insert(key);
        }
        cx.notify();
    }

    /// Prompt, actions and result of one task. Plain text only: agent output is never interpreted.
    fn task_body(&self, task: &Task, key: &str, now: i64, p: &Palette, cx: &mut Context<Self>) -> Vec<Div> {
        let mut prompt = section("Prompt".into(), p);
        match task.prompt.as_deref() {
            None | Some("") => prompt = prompt.child(note("No prompt received.", p)),
            Some(text) => {
                let long = text.chars().count() > LONG_PROMPT;
                let open_key = format!("prompt:{key}");
                let open = self.activity.expanded.contains(&open_key);
                let shown = if long && !open {
                    let lines: Vec<&str> = text.lines().take(6).collect();
                    let mut cut: String = lines.join("\n").chars().take(LONG_PROMPT).collect();
                    cut.push('…');
                    cut
                } else {
                    text.to_owned()
                };
                prompt = prompt
                    .child(div().font_family(agentdeck_core::logic::theme::FONT_MONO).text_size(ui(12.0)).text_color(hsla(p.fg)).child(shown))
                    .when(long, |el| {
                        el.child(
                            div()
                                .id(SharedString::from(open_key.clone()))
                                .text_size(ui(12.0))
                                .text_color(hsla(p.subtle))
                                .cursor_pointer()
                                .child(if open { "Show less" } else { "Show all" })
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_expanded(open_key.clone(), cx))),
                        )
                    });
            }
        }
        let mut actions = section(format!("Actions ({})", task.actions.len()), p);
        if task.actions.is_empty() {
            actions = actions.child(note("No tool actions.", p));
        }
        for action in &task.actions {
            actions = actions.child(
                div()
                    .flex()
                    .items_start()
                    .gap(ui(6.0))
                    .text_size(ui(12.0))
                    .child(Icon::new(action_icon(&action.text)).size(ui(13.0)).text_color(hsla(p.subtle)))
                    .child(div().flex_1().min_w_0().truncate().text_color(hsla(p.muted)).child(action.text.clone()))
                    .child(div().flex_shrink_0().text_color(hsla(p.subtle)).child(format_since(now - action.at))),
            );
        }
        let title = if task.status == PaneStatus::Error { "Error" } else { "Result" };
        let mut result = section(title.into(), p);
        result = match &task.summary {
            Some(summary) => result.child(div().text_size(ui(12.0)).text_color(hsla(p.fg)).child(summary.clone())),
            None if task.ended_at.is_some() => result.child(note("No summary received.", p)),
            None => result.child(note("Task in progress…", p)),
        };
        vec![prompt, actions, result]
    }

    /// Slide-over inside the activity panel: what a finished agent did.
    fn task_detail(&mut self, pane_id: &str, now: i64, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let deck = self.deck.read(cx);
        let Some(pane) = deck.state.panes.get(pane_id).cloned() else {
            return div().into_any_element();
        };
        let log = deck.tasks.get(pane_id).cloned().unwrap_or_default();
        let mut tasks: Vec<Task> = log.current.into_iter().chain(log.history).collect();
        let task = (!tasks.is_empty()).then(|| tasks.remove(0));
        let previous = tasks;
        let name = pane_name(&pane);
        // HandOff: other live panes, labelled "title (session)".
        let targets: Vec<(String, String)> = deck
            .state
            .tabs
            .iter()
            .flat_map(|t| {
                t.pane_ids.iter().filter_map(|id| {
                    let other = deck.state.panes.get(id)?;
                    (id != pane_id && !other.exited).then(|| (id.clone(), format!("{} ({})", pane_name(other), t.name)))
                })
            })
            .collect();
        let copied = now - self.activity.copied_at < COPIED_MS;

        let header = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .h(ui(40.0))
            .border_b_1()
            .border_color(hsla(p.line))
            .pl(ui(6.0))
            .pr_2()
            .child(
                icon_button("task-back", IconName::ArrowLeft, "Back (Esc)", &p)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_detail(cx))),
            )
            .child(agent_logo(pane.agent, 14.0, None, &p))
            .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).text_color(hsla(p.fg)).child(name.clone()))
            .when_some(task.as_ref(), |el, task| {
                el.child(
                    div()
                        .flex_shrink_0()
                        .text_size(ui(12.0))
                        .text_color(hsla(p.subtle))
                        .child(format_elapsed(task.ended_at.unwrap_or(now) - task.started_at)),
                )
            });

        let mut body = div().id("task-detail-body").flex().flex_col().flex_1().min_h_0().overflow_y_scroll();
        body = match &task {
            Some(task) => body.children(self.task_body(task, "current", now, &p, cx)),
            None => body.child(note("No details available for this agent (no hooks).", &p).px_3().py_4()),
        };
        let summary = task.as_ref().and_then(|t| t.summary.clone());
        let id = pane_id.to_owned();
        let mut buttons = div()
            .flex()
            .flex_wrap()
            .gap(ui(6.0))
            .border_b_1()
            .border_color(hsla(p.line))
            .px_3()
            .py(ui(10.0))
            .child(small_button("task-jump", IconName::SquareTerminal, "Go to terminal", &p).on_click(cx.listener({
                let id = id.clone();
                move |this, _: &ClickEvent, _, cx| this.jump(&id, cx)
            })));
        if pane.status == PaneStatus::Done {
            buttons = buttons.child(small_button("task-seen", IconName::Check, "Mark as seen", &p).on_click(cx.listener({
                let id = id.clone();
                move |this, _: &ClickEvent, _, cx| {
                    this.acknowledge(&id, cx);
                    this.close_detail(cx);
                }
            })));
        }
        if let Some(summary) = summary {
            let label = if copied { "Copied" } else { "Copy result" };
            buttons = buttons.child(small_button("task-copy", IconName::Copy, label, &p).on_click(cx.listener({
                let summary = summary.clone();
                move |this, _: &ClickEvent, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(summary.clone()));
                    this.activity.copied_at = now_ms();
                    cx.notify();
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(std::time::Duration::from_millis(COPIED_MS as u64)).await;
                        this.update(cx, |_, cx| cx.notify()).ok();
                    })
                    .detach();
                }
            })));
            if !targets.is_empty() {
                let weak = cx.entity().downgrade();
                let from = name.clone();
                buttons = buttons.child(
                    Button::new("task-handoff")
                        .outline()
                        .small()
                        .label("Send result to…")
                        .dropdown_caret(true)
                        .dropdown_menu(move |mut menu, _, _| {
                            for (target, label) in targets.clone() {
                                let (weak, from, summary) = (weak.clone(), from.clone(), summary.clone());
                                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                                    // Pasted into the prompt, not submitted: the user adds the instruction.
                                    let text = format!("Result from {from}:\n{summary}\n\n");
                                    runtime::send_input(&target, &prompt_text(&text), cx);
                                    weak.update(cx, |this, cx| this.jump(&target, cx)).ok();
                                }));
                            }
                            menu
                        }),
                );
            }
        }
        body = body.child(buttons);
        if !previous.is_empty() {
            let mut list = section(format!("Previous tasks ({})", previous.len()), &p);
            for (i, prev) in previous.iter().enumerate() {
                let key = format!("prev:{}", prev.started_at);
                let open = self.activity.expanded.contains(&key);
                let first = prev
                    .prompt
                    .as_deref()
                    .and_then(|t| t.lines().next())
                    .filter(|l| !l.is_empty())
                    .unwrap_or("Task without a prompt")
                    .to_owned();
                let hover = (hsla(p.hover), hsla(p.fg));
                list = list.child(
                    div()
                        .id(ElementId::Name(format!("task-prev-{i}").into()))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(ui(6.0))
                        .px_1()
                        .py_1()
                        .text_size(ui(12.0))
                        .text_color(hsla(p.muted))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover.0).text_color(hover.1))
                        .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(ui(12.0)))
                        .child(div().flex_1().min_w_0().truncate().child(first))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(hsla(p.subtle))
                                .child(format_since(now - prev.ended_at.unwrap_or(prev.started_at))),
                        )
                        .on_click(cx.listener({
                            let key = key.clone();
                            move |this, _: &ClickEvent, _, cx| this.toggle_expanded(key.clone(), cx)
                        })),
                );
                if open {
                    list = list.child(
                        div().border_t_1().border_color(hsla(p.line)).children(self.task_body(prev, &key, now, &p, cx)),
                    );
                }
            }
            body = body.child(list);
        }

        div()
            .id("task-detail")
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .bg(hsla(p.panel))
            .occlude()
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    this.close_detail(cx);
                    cx.stop_propagation();
                }
            }))
            .child(header)
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn action_icons() {
        assert_eq!(action_icon("Bash npm test"), IconName::Terminal);
        assert_eq!(action_icon("Edit src/a.rs"), IconName::FilePen);
        assert_eq!(action_icon("Grep foo"), IconName::Search);
        assert_eq!(action_icon("Editor"), IconName::Dot);
    }
}
