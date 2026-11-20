//! Activity panel (app/Activity.tsx) and its task detail slide-over (app/TaskDetail.tsx): every agent
//! of every workspace grouped by state, permission cards inline, and what a finished agent did.

use std::collections::{HashMap, HashSet};

use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::hooks_server::permissions::PermissionRequest;
use agentdeck_core::logic::deck::{ACTIVITY_ORDER, ActivityKey, Pane, activity_key};
use agentdeck_core::logic::queue::prompt_text;
use agentdeck_core::logic::tasks::{HandoffKind, HandoffSource, Task, handoff_draft, swap_instruction};
use agentdeck_core::logic::text::{agent_name, format_elapsed, format_since, now_ms};
use agentdeck_core::logic::theme::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::DeckView;
use super::chrome::{icon_button, key_button};
use crate::panes::{PermissionCard, agent_logo, runtime, status_color};
use crate::theme::{hsla, hsla_a, palette, ui};
use agentdeck_core::logic::theme::{RADIUS_MD};

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
    /// Handoff being prepared from the open task detail.
    handoff: Option<Handoff>,
}

/// Editable handoff of a finished task to another agent ("Prepare handoff").
pub(super) struct Handoff {
    /// Pane whose task is handed off.
    source: String,
    kind: HandoffKind,
    /// Destination pane, and its folder once resolved.
    target: Option<String>,
    target_dir: Option<String>,
    /// The draft: survives the destination closing, so another one can be picked.
    text: Entity<TextareaState>,
}

/// Folder an agent works in (its worktree when it has one); the ssh target for a remote pane.
/// Touches the disk: call off the UI thread.
fn work_dir(cwd: String, pane_id: String, worktree: bool, ssh: Option<String>) -> String {
    ssh.unwrap_or_else(|| agentdeck_core::git::git_pane_dir(cwd, pane_id, worktree))
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

/// Agents that wait on the user: a question, an error, or a pending permission card.
fn needs_user(key: ActivityKey, has_request: bool) -> bool {
    has_request || matches!(key, ActivityKey::Status(PaneStatus::Waiting | PaneStatus::Error))
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
        .rounded(ui(RADIUS_MD))
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
    /// Right column: every agent of every workspace, the ones needing attention first.
    pub(super) fn activity_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let now = now_ms();
        // Same card entities as the banner: picks and a half-typed deny reason survive switching views.
        let pending: Vec<PermissionRequest> = self.deck.read(cx).permissions.clone();
        let cards: HashMap<String, Entity<PermissionCard>> = self.permission_banner.update(cx, |banner, cx| {
            pending.iter().map(|r| (r.request_id.clone(), banner.card(r, cx))).collect()
        });
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
        // The to-do list first, then the other states in ACTIVITY_ORDER. An agent is listed once.
        let waits = |pane: &Pane| needs_user(activity_key(pane), pending.iter().any(|r| r.pane_id == pane.id));
        let mut sections = vec![(
            "Needs you",
            true,
            groups.iter().flat_map(|(_, items)| items.iter().copied()).filter(|(pane, ..)| waits(pane)).collect::<Vec<_>>(),
        )];
        for key in ACTIVITY_ORDER {
            let Some((_, items)) = groups.iter().find(|(k, _)| *k == key) else { continue };
            sections.push((activity_label(key), false, items.iter().copied().filter(|(pane, ..)| !waits(pane)).collect()));
        }
        for (title, urgent, items) in sections {
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
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(hsla(if urgent { p.fg } else { p.muted }))
                    .child(title)
                    .child(
                        div()
                            .ml_auto()
                            .text_color(hsla(if urgent { p.wait } else { p.subtle }))
                            .child(items.len().to_string()),
                    ),
            );
            for (pane, tab, workspace) in items {
                let active = deck.state.active_pane_id.as_deref() == Some(pane.id.as_str());
                let key = activity_key(pane);
                let name = pane_name(pane);
                let task = pane.topic.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| name.clone());
                let (status_word, status_tint) = match key {
                    ActivityKey::Exited => (activity_label(key), p.subtle),
                    ActivityKey::Status(status) => (activity_label(key), status_color(status, &p)),
                };
                // Time in the current state; a working agent shows how long its task has run.
                let since = match key {
                    ActivityKey::Status(PaneStatus::Working) => pane.task_started_at.unwrap_or(pane.status_since),
                    _ => pane.status_since,
                };
                let mut meta = format!("{} · {} › {}", format_since(now - since), workspace.name, tab.name);
                if task != name {
                    meta = format!("{name} · {meta}");
                }
                let requests: Vec<&PermissionRequest> = pending.iter().filter(|r| r.pane_id == pane.id).collect();
                // What the user is expected to do, in words and in the color of the state.
                let expected: Option<(String, u32)> = match key {
                    _ if !requests.is_empty() => Some(("Permission requested".to_owned(), p.wait)),
                    ActivityKey::Status(PaneStatus::Waiting) => Some(("Waiting for your reply".to_owned(), p.wait)),
                    ActivityKey::Status(PaneStatus::Error) => {
                        let log = deck.tasks.get(&pane.id);
                        let last = log.and_then(|l| l.current.as_ref().or(l.history.first()));
                        let text = last.filter(|t| t.status == PaneStatus::Error).and_then(|t| t.summary.clone());
                        Some((text.filter(|t| !t.is_empty()).unwrap_or_else(|| "Stopped on an error".to_owned()), p.bad))
                    }
                    ActivityKey::Status(PaneStatus::Done) => Some(("Finished: review the result".to_owned(), p.ok)),
                    _ => None,
                };
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
                    .pl(ui(10.0))
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
                                    .child(task),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(ui(12.0))
                                    .text_color(hsla(status_tint))
                                    .child(status_word),
                            )
                            .child(dot),
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
                    .child(div().truncate().pl(ui(22.0)).text_size(ui(11.0)).text_color(hsla(p.subtle)).child(meta))
                    .when_some(expected, |el, (text, tint)| {
                        let hint: SharedString = text.clone().into();
                        el.child(
                            div()
                                .id(ElementId::Name(format!("activity-expected-{}", pane.id).into()))
                                .truncate()
                                .pl(ui(22.0))
                                .text_size(ui(12.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(hsla(tint))
                                .child(text)
                                .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx)),
                        )
                    })
                    // Claude subagents running inside the pane: what each one is doing.
                    .children(runtime::pane_subagents(&pane.id, cx).into_iter().map(|sub| {
                        let action = sub.last_action.unwrap_or_else(|| "Starting…".to_owned());
                        div()
                            .flex()
                            .items_center()
                            .gap(ui(6.0))
                            .pl(ui(22.0))
                            .text_size(ui(12.0))
                            .text_color(hsla(p.subtle))
                            .child(Icon::new(IconName::CornerDownRight).size(ui(12.0)))
                            .child(div().flex_1().min_w_0().truncate().child(format!("{}: {action}", sub.label)))
                            .child(div().flex_shrink_0().child(format_elapsed(now - sub.started_at)))
                    }))
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
                        // Selection: accent bar, always reserved so cards do not shift.
                        .child(
                            div()
                                .flex()
                                .items_start()
                                .border_l_2()
                                .border_color(hsla_a(p.accent, if active { 1.0 } else { 0.0 }))
                                .child(main)
                                .children(side),
                        )
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
            .border_r_1()
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
                        key_button("activity-close", IconName::X, "Back to projects", &["Ctrl", "Shift", "A"], &p)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_activity(cx))),
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
        self.activity.handoff = None;
        cx.notify();
    }

    /// Opens the handoff editor of the pane's last task: the draft names the pane's own worktree and
    /// branch (resolved off the UI thread), whatever pane is active.
    fn prepare_handoff(&mut self, pane_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let deck = self.deck.read(cx);
        let Some(pane) = deck.state.panes.get(pane_id) else { return };
        let log = deck.tasks.get(pane_id);
        let Some(task) = log.and_then(|l| l.current.as_ref().or(l.history.first())).cloned() else { return };
        let project = deck
            .state
            .tabs
            .iter()
            .find(|t| t.pane_ids.iter().any(|id| id == pane_id))
            .and_then(|t| deck.state.workspaces.iter().find(|w| w.id == t.workspace_id))
            .map(|w| w.name.clone())
            .unwrap_or_default();
        let agent = pane_name(pane);
        let (source, cwd, worktree, ssh) = (pane.id.clone(), pane.cwd.clone(), pane.worktree, pane.ssh.clone());
        cx.spawn_in(window, async move |this, cx| {
            let id = source.clone();
            let (dir, branch) = cx
                .background_executor()
                .spawn(async move {
                    let remote = ssh.is_some();
                    let dir = work_dir(cwd, id, worktree, ssh);
                    let branch = if remote {
                        None
                    } else {
                        agentdeck_core::workspace::inspect_folder(dir.clone()).ok().and_then(|info| info.branch)
                    };
                    (dir, branch)
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                let from = HandoffSource { agent: &agent, project: &project, dir: &dir, branch: branch.as_deref() };
                let Some(draft) = handoff_draft(&task, &from, HandoffKind::Review) else { return };
                let text = cx.new(|cx| TextareaState::new(window, cx));
                text.update(cx, |t, cx| t.set_value(draft, window, cx));
                this.activity.handoff =
                    Some(Handoff { source, kind: HandoffKind::Review, target: None, target_dir: None, text });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Picks the other instruction; the user's edits elsewhere in the draft are kept.
    fn set_handoff_kind(&mut self, kind: HandoffKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(handoff) = self.activity.handoff.as_mut() else { return };
        let text = swap_instruction(&handoff.text.read(cx).value(), handoff.kind, kind);
        handoff.kind = kind;
        handoff.text.update(cx, |t, cx| t.set_value(text, window, cx));
        cx.notify();
    }

    fn pick_handoff_target(&mut self, target: String, cx: &mut Context<Self>) {
        let Some(handoff) = self.activity.handoff.as_mut() else { return };
        let Some(pane) = self.deck.read(cx).state.panes.get(&target) else { return };
        let (cwd, id, worktree, ssh) = (pane.cwd.clone(), pane.id.clone(), pane.worktree, pane.ssh.clone());
        handoff.target = Some(target.clone());
        handoff.target_dir = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let dir = cx.background_executor().spawn(async move { work_dir(cwd, id, worktree, ssh) }).await;
            this.update(cx, |this, cx| {
                if let Some(handoff) = this.activity.handoff.as_mut().filter(|h| h.target.as_ref() == Some(&target)) {
                    handoff.target_dir = Some(dir);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Pastes the draft into the destination's prompt without submitting it (same paste as "Send
    /// result to…"), then shows that terminal. Nothing happens if the destination closed meanwhile.
    fn send_handoff(&mut self, cx: &mut Context<Self>) {
        let Some(handoff) = &self.activity.handoff else { return };
        let alive = |id: &String| self.deck.read(cx).state.panes.get(id).is_some_and(|p| !p.exited);
        let Some(target) = handoff.target.clone().filter(alive) else { return };
        let text = handoff.text.read(cx).value().trim_end().to_owned();
        if text.trim().is_empty() {
            return;
        }
        runtime::send_input(&target, &prompt_text(&format!("{text}\n\n")), cx);
        self.activity.handoff = None;
        self.jump(&target, cx);
    }

    /// Handoff editor, in place of the task body: instruction, destination and its folder, the
    /// editable draft, Cancel / Paste into terminal.
    fn handoff_editor(&self, handoff: &Handoff, targets: &[(String, String)], p: &Palette, cx: &mut Context<Self>) -> Div {
        let target = handoff.target.as_ref().and_then(|id| targets.iter().find(|(t, _)| t == id));
        let gone = handoff.target.is_some() && target.is_none();
        let kinds = [HandoffKind::Review, HandoffKind::Continue].map(|kind| {
            let pressed = handoff.kind == kind;
            let (hover, fg) = (hsla(p.hover), hsla(p.fg));
            div()
                .id(SharedString::from(format!("handoff-kind-{}", kind.label())))
                .flex()
                .items_center()
                .h(ui(26.0))
                .px_2()
                .rounded(ui(RADIUS_MD))
                .border_1()
                .text_size(ui(12.0))
                .cursor_pointer()
                .when(pressed, |d| d.border_color(hsla(p.accent)).bg(hsla_a(p.accent, 0.12)).text_color(hsla(p.fg)))
                .when(!pressed, |d| d.border_color(hsla(p.line)).text_color(hsla(p.muted)).hover(move |s| s.bg(hover).text_color(fg)))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.set_handoff_kind(kind, window, cx)))
                .child(kind.label())
        });
        let weak = cx.entity().downgrade();
        let menu_targets = targets.to_vec();
        let picker = Button::new("handoff-target")
            .outline()
            .small()
            .label(target.map_or_else(|| "Choose an agent…".to_owned(), |(_, label)| label.clone()))
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for (id, label) in menu_targets.clone() {
                    let weak = weak.clone();
                    menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                        weak.update(cx, |this, cx| this.pick_handoff_target(id.clone(), cx)).ok();
                    }));
                }
                menu
            });
        let destination_note = if targets.is_empty() {
            Some("No other agent is open. Start one, then pick it here: your text is kept.".to_owned())
        } else if gone {
            Some("That agent closed. Pick another one: your text is kept.".to_owned())
        } else if target.is_some() {
            Some(handoff.target_dir.as_ref().map_or_else(|| "Folder: …".to_owned(), |dir| format!("Folder: {dir}")))
        } else {
            None
        };
        let can_send = target.is_some();
        let paste = small_button("handoff-send", IconName::Send, "Paste into terminal", p);
        let paste = if can_send {
            paste.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.send_handoff(cx)))
        } else {
            paste.opacity(0.5).cursor_default()
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(section("Instruction".into(), p).child(div().flex().flex_wrap().gap(ui(6.0)).children(kinds)))
            .child(
                section("Destination".into(), p)
                    .child(div().flex().child(picker))
                    .children(destination_note.map(|text| note(text, p).whitespace_normal())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(ui(160.0))
                    .gap(ui(6.0))
                    .px_3()
                    .py(ui(10.0))
                    .child(div().text_size(ui(12.0)).text_color(hsla(p.subtle)).child("Draft"))
                    .child(div().flex_1().min_h_0().text_size(ui(12.0)).child(Textarea::new(&handoff.text).h(relative(1.0)))),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(ui(6.0))
                    .border_t_1()
                    .border_color(hsla(p.line))
                    .px_3()
                    .py(ui(10.0))
                    .child(small_button("handoff-cancel", IconName::X, "Cancel", p).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.activity.handoff = None;
                            cx.notify();
                        },
                    )))
                    .child(paste)
                    .child(note("Pasted, not sent: press Enter in the terminal when ready.", p).whitespace_normal()),
            )
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
        // A task with a summary or at least its initial request can be handed off.
        let transferable = task.as_ref().is_some_and(|t| {
            [&t.prompt, &t.summary].iter().any(|text| text.as_deref().is_some_and(|s| !s.trim().is_empty()))
        });
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
                let targets = targets.clone();
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
        if transferable {
            buttons = buttons.child(
                small_button("task-prepare-handoff", IconName::ArrowRightLeft, "Prepare handoff…", &p)
                    .tooltip(|window, cx| {
                        Tooltip::new("Hand this task to another agent for review or follow-up: edit the draft, then paste it")
                            .build(window, cx)
                    })
                    .on_click(cx.listener({
                        let id = id.clone();
                        move |this, _: &ClickEvent, window, cx| this.prepare_handoff(&id, window, cx)
                    })),
            );
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
                        .rounded(ui(RADIUS_MD))
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
        // Preparing a handoff: its editor replaces the task body until sent or cancelled.
        let content = match self.activity.handoff.as_ref().filter(|h| h.source == pane_id) {
            Some(handoff) => self.handoff_editor(handoff, &targets, &p, cx).into_any_element(),
            None => body.into_any_element(),
        };

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
                    // Esc cancels a handoff first (nothing was pasted), then closes the detail.
                    if this.activity.handoff.take().is_some() {
                        cx.notify();
                    } else {
                        this.close_detail(cx);
                    }
                    cx.stop_propagation();
                }
            }))
            .child(header)
            .child(content)
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

    #[test]
    fn needs_user_groups() {
        assert!(needs_user(ActivityKey::Status(PaneStatus::Waiting), false));
        assert!(needs_user(ActivityKey::Status(PaneStatus::Error), false));
        assert!(needs_user(ActivityKey::Status(PaneStatus::Working), true));
        assert!(!needs_user(ActivityKey::Status(PaneStatus::Done), false));
        assert!(!needs_user(ActivityKey::Exited, false));
    }
}
