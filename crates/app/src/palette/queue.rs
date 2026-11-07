//! Per-pane prompt queue: header button, editor dialog, and the runner that sends the next prompt
//! 1.5 s after a hooked agent finishes (components/Queue/QueueButton.tsx, store/queue.ts).

use std::collections::HashMap;
use std::time::Duration;

use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::Pane;
use agentdeck_core::logic::queue::{ADVANCE_DELAY_MS, QUEUE_AGENTS, ready_for_next, review_prompt, should_advance};
use agentdeck_core::logic::text::agent_name;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::ui;
use crate::state::Deck;
use crate::theme::{hsla, palette};

/// Group name the pane container should carry (`.group(QUEUE_HOVER_GROUP)`): the queue button is
/// hidden until the pane is hovered, like the other header actions.
pub const QUEUE_HOVER_GROUP: &str = "pane";

/// Last seen status of every pane, to detect "just finished" transitions.
#[derive(Default)]
pub(super) struct Runner {
    statuses: HashMap<String, PaneStatus>,
}

/// Watches pane statuses; called once when the deck is known.
pub(super) fn start_runner(deck: &Entity<Deck>, cx: &mut App) {
    cx.observe(deck, |deck, cx| on_deck_changed(deck, cx)).detach();
}

fn on_deck_changed(deck: Entity<Deck>, cx: &mut App) {
    let (gone, advance) = {
        let d = deck.read(cx);
        let runner = &cx.global::<super::PaletteGlobal>().queue_runner;
        let gone: Vec<String> = d.queues.0.keys().filter(|id| !d.state.panes.contains_key(*id)).cloned().collect();
        let advance: Vec<String> = d
            .state
            .panes
            .values()
            .filter(|p| should_advance(runner.statuses.get(&p.id).copied(), p.status, d.queues.get(&p.id)))
            .map(|p| p.id.clone())
            .collect();
        (gone, advance)
    };
    let (dropped, reviews) = review_transitions(deck.read(cx), &cx.global::<super::PaletteGlobal>().queue_runner);
    let statuses = deck.read(cx).state.panes.iter().map(|(id, p)| (id.clone(), p.status)).collect();
    cx.global_mut::<super::PaletteGlobal>().queue_runner.statuses = statuses;
    if !gone.is_empty() || !dropped.is_empty() {
        deck.update(cx, |d, _| {
            gone.iter().for_each(|id| d.queues.forget(id));
            dropped.iter().for_each(|id| {
                d.reviews.remove(id);
            });
        });
    }
    for (author, reviewer) in reviews {
        let deck = deck.downgrade();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(ADVANCE_DELAY_MS)).await;
            cx.update(|cx| {
                if let Some(deck) = deck.upgrade() {
                    request_review(&deck, &author, &reviewer, cx);
                }
            });
        })
        .detach();
    }
    for pane_id in advance {
        let deck = deck.downgrade();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(ADVANCE_DELAY_MS)).await;
            cx.update(|cx| {
                let Some(deck) = deck.upgrade() else { return };
                let d = deck.read(cx);
                // Still alive and not busy again (looking at a done pane turns it idle).
                let ready = d.queues.get(&pane_id).is_some_and(|q| !q.paused)
                    && ready_for_next(d.state.panes.get(&pane_id));
                if ready {
                    send_next(&deck, &pane_id, cx);
                }
            });
        })
        .detach();
    }
}

/// Code then review (store/review.ts `startReviewRunner`): authors whose reviewer pane is gone, and
/// (author, reviewer) pairs whose author just turned "done".
fn review_transitions(d: &Deck, runner: &Runner) -> (Vec<String>, Vec<(String, String)>) {
    let mut dropped = Vec::new();
    let mut due = Vec::new();
    for (author, reviewer) in &d.reviews {
        let Some(pane) = d.state.panes.get(author) else {
            dropped.push(author.clone());
            continue;
        };
        if !d.state.panes.contains_key(reviewer) {
            dropped.push(author.clone());
            continue;
        }
        let was_done = runner.statuses.get(author) == Some(&PaneStatus::Done);
        if !was_done && pane.status == PaneStatus::Done {
            due.push((author.clone(), reviewer.clone()));
        }
    }
    (dropped, due)
}

/// Asks `reviewer` to review `author`'s last task: sent now when it is idle, queued otherwise.
fn request_review(deck: &Entity<Deck>, author_id: &str, reviewer_id: &str, cx: &mut App) {
    let d = deck.read(cx);
    let (Some(author), Some(reviewer)) = (d.state.panes.get(author_id), d.state.panes.get(reviewer_id)) else {
        return;
    };
    if reviewer.exited {
        return;
    }
    let task = d.tasks.get(author_id).and_then(|log| log.history.first()).and_then(|t| t.prompt.clone());
    let dir = crate::git::pane_git_dir(d, author_id).unwrap_or_else(|| author.cwd.clone());
    let text = review_prompt(&author.title, author.agent, &dir, task.as_deref());
    let idle = ready_for_next(Some(reviewer)) && d.queues.get(reviewer_id).is_none_or(|q| q.items.is_empty());
    let toast = format!("{} is reviewing {}'s changes.", reviewer.title, author.title);
    let reviewer_id = reviewer_id.to_owned();
    deck.update(cx, |d, cx| {
        d.queues.add(&reviewer_id, &text);
        d.toast(toast, cx);
    });
    // The queue sends right away when the reviewer is idle; otherwise when it finishes its current task.
    if idle {
        send_next(deck, &reviewer_id, cx);
    }
}

/// Sends the next queued prompt now. Puts it back and toasts if the write fails.
pub fn send_next(deck: &Entity<Deck>, pane_id: &str, cx: &mut App) {
    let Some(item) = deck.update(cx, |d, cx| {
        cx.notify();
        d.queues.pop_next(pane_id)
    }) else {
        return;
    };
    let backend = deck.read(cx).backend.clone();
    let executor = cx.background_executor().clone();
    let send = super::send_prompt(backend, executor.clone(), pane_id.to_owned(), item.text.clone());
    let task = executor.spawn(send);
    let weak = deck.downgrade();
    let pane_id = pane_id.to_owned();
    cx.spawn(async move |cx| {
        if let Err(e) = task.await {
            weak.update(cx, |d, cx| {
                d.queues.requeue_front(&pane_id, item);
                d.toast(format!("Queue: could not send ({e})"), cx);
            })
            .ok();
        }
    })
    .detach();
}

/// Header button with the queue size badge. Disabled for agents without hooks (no "done" event).
/// Like the other header actions it shows on pane hover ([`QUEUE_HOVER_GROUP`]), but always while
/// the queue has items or the pane is active.
pub fn queue_button(deck: &Entity<Deck>, pane: &Pane, active: bool, cx: &App) -> AnyElement {
    let p = palette(cx);
    let queue = deck.read(cx).queues.get(&pane.id).cloned().unwrap_or_default();
    let count = queue.items.len();
    let always = count > 0 || active;
    let supported = QUEUE_AGENTS.contains(&pane.agent);
    let id = SharedString::from(format!("queue-{}", pane.id));
    let base = div()
        .relative()
        .flex()
        .when(!always, |s| s.opacity(0.0).group_hover(QUEUE_HOVER_GROUP, |s| s.opacity(1.0)));
    if !supported {
        return base
            .child(
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(crate::theme::ui(24.0))
                    .text_size(crate::theme::ui(14.0))
                    .text_color(hsla(p.muted))
                    .opacity(0.4)
                    .child(IconName::ListPlus)
                    .tooltip(|window, cx| {
                        Tooltip::new("Queue: Claude Code and Codex only (task end is detected by hooks)").build(window, cx)
                    }),
            )
            .into_any_element();
    }
    let hint = if count > 0 {
        format!("Queue: {count} prompt(s){}", if queue.paused { " (paused)" } else { "" })
    } else {
        "Queue".to_owned()
    };
    let deck = deck.clone();
    let pane_id = pane.id.clone();
    let hover = hsla(p.hover);
    let fg = hsla(p.fg);
    base.child(
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .size(crate::theme::ui(24.0))
            .rounded(crate::theme::ui(6.0))
            .text_size(crate::theme::ui(14.0))
            .text_color(hsla(p.muted))
            .cursor_pointer()
            .hover(move |s| s.bg(hover).text_color(fg))
            .child(IconName::ListPlus)
            .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
            .on_click(move |_, window, cx| open_queue(deck.clone(), pane_id.clone(), window, cx)),
    )
    .when(count > 0, |s| {
        s.child(
            div()
                .absolute()
                .top(crate::theme::ui(-2.0))
                .right(crate::theme::ui(-2.0))
                .flex()
                .items_center()
                .justify_center()
                .h(crate::theme::ui(14.0))
                .min_w(crate::theme::ui(14.0))
                .px(crate::theme::ui(2.0))
                .rounded_full()
                .font_family(agentdeck_core::logic::theme::FONT_MONO)
                .text_size(crate::theme::ui(10.0))
                .when(queue.paused, |s| s.bg(hsla(p.raised)).text_color(hsla(p.muted)))
                .when(!queue.paused, |s| s.bg(hsla(p.accent)).text_color(hsla(p.on_accent)))
                .child(count.to_string()),
        )
    })
    .into_any_element()
}

pub struct QueueDialog {
    deck: Entity<Deck>,
    pane_id: String,
    text: Entity<TextareaState>,
    _subscriptions: Vec<Subscription>,
}

/// Opens the queue editor of a pane.
pub fn open_queue(deck: Entity<Deck>, pane_id: String, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| QueueDialog::new(deck, pane_id, window, cx));
    let focus = view.read(cx).text.focus_handle(cx);
    super::open_modal(view, 544.0, None, focus, window, cx);
}

impl QueueDialog {
    fn new(deck: Entity<Deck>, pane_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = cx.new(|cx| TextareaState::new(window, cx).placeholder("Prompt to queue (Ctrl+Enter)"));
        let subscription = cx.subscribe_in(&text, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { secondary: true, .. } = event {
                this.add(window, cx);
            }
            cx.notify();
        });
        // Re-render when the queue or the pane status changes elsewhere (runner, hooks).
        let observe = cx.observe(&deck, |_, _, cx| cx.notify());
        Self { deck, pane_id, text, _subscriptions: vec![subscription, observe] }
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let pane_id = self.pane_id.clone();
        self.deck.update(cx, |d, cx| {
            d.queues.add(&pane_id, &text);
            cx.notify();
        });
        self.text.update(cx, |t, cx| t.set_value("", window, cx));
    }
}

impl Render for QueueDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let (title, busy, queue) = {
            let d = self.deck.read(cx);
            let pane = d.state.panes.get(&self.pane_id);
            let title = pane
                .map(|p| if p.title.is_empty() { agent_name(p.agent).to_owned() } else { p.title.clone() })
                .unwrap_or_default();
            let busy = pane.is_none_or(|p| matches!(p.status, PaneStatus::Working | PaneStatus::Waiting) || p.exited);
            (title, busy, d.queues.get(&self.pane_id).cloned().unwrap_or_default())
        };
        let count = queue.items.len();
        let empty_text = self.text.read(cx).value().trim().is_empty();
        let (bad, hover_bg) = (hsla(p.bad), hsla(p.hover));
        let list = if count == 0 {
            div().py_3().text_center().text_color(hsla(p.muted)).child("Queue is empty.").into_any_element()
        } else {
            div()
                .id("queue-items")
                .flex()
                .flex_col()
                .max_h(crate::theme::ui(288.0))
                .overflow_y_scroll()
                .rounded(crate::theme::ui(6.0))
                .border_1()
                .border_color(hsla(p.line))
                .children(queue.items.iter().enumerate().map(|(i, item)| {
                    let (up, down, remove) = (item.id.clone(), item.id.clone(), item.id.clone());
                    div()
                        .flex()
                        .items_start()
                        .gap_2()
                        .border_b_1()
                        .border_color(hsla(p.line))
                        .py_1p5()
                        .pr_1()
                        .pl_3()
                        .child(
                            div()
                                .pt_1()
                                .font_family(agentdeck_core::logic::theme::FONT_MONO)
                                .text_xs()
                                .text_color(hsla(p.subtle))
                                .child((i + 1).to_string()),
                        )
                        .child(div().min_w_0().flex_1().pt_0p5().text_color(hsla(p.fg)).child(item.text.clone()))
                        .child(
                            ui::icon_button(SharedString::from(format!("up-{}", item.id)), IconName::ArrowUp, &p)
                                .when(i == 0, |s| s.opacity(0.4))
                                .on_click(cx.listener(move |this, _, _, cx| this.move_item(&up, -1, cx))),
                        )
                        .child(
                            ui::icon_button(SharedString::from(format!("down-{}", item.id)), IconName::ArrowDown, &p)
                                .when(i + 1 == count, |s| s.opacity(0.4))
                                .on_click(cx.listener(move |this, _, _, cx| this.move_item(&down, 1, cx))),
                        )
                        .child(
                            ui::icon_button(SharedString::from(format!("rm-{}", item.id)), IconName::Trash, &p)
                                .hover(move |s| s.bg(hover_bg).text_color(bad))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let pane_id = this.pane_id.clone();
                                    this.deck.update(cx, |d, cx| {
                                        d.queues.remove(&pane_id, &remove);
                                        cx.notify();
                                    });
                                })),
                        )
                }))
                .into_any_element()
        };
        let paused = queue.paused;
        let send_disabled = busy || count == 0;
        let mut send = ui::primary_button("send-next", IconName::Send, "Send next", send_disabled, &p);
        if busy {
            send = send.tooltip(|window, cx| {
                Tooltip::new("The agent is busy: the next prompt goes out when it finishes").build(window, cx)
            });
        }
        div()
            .flex()
            .flex_col()
            .w_full()
            .text_size(crate::theme::ui(13.0))
            .child(ui::header(
                "Queue",
                format!("{title} · sent automatically 1.5 s after each finished task"),
                &p,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_5()
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child(Textarea::new(&self.text).h(crate::theme::ui(64.0))))
                            .child(
                                ui::icon_button("queue-add", IconName::Plus, &p)
                                    .when(empty_text, |s| s.opacity(0.4))
                                    .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                            ),
                    )
                    .child(list),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(hsla(p.line))
                    .px_5()
                    .py_3()
                    .child(
                        div()
                            .mr_auto()
                            .text_xs()
                            .text_color(hsla(p.subtle))
                            .child(if paused { "Paused".to_owned() } else { format!("{count} queued") }),
                    )
                    .child(
                        ui::secondary_button(
                            "pause",
                            if paused { IconName::Play } else { IconName::Pause },
                            if paused { "Resume" } else { "Pause" },
                            false,
                            &p,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let pane_id = this.pane_id.clone();
                            this.deck.update(cx, |d, cx| {
                                d.queues.set_paused(&pane_id, !paused);
                                cx.notify();
                            });
                        })),
                    )
                    .child(send.on_click(cx.listener(move |this, _, _, cx| {
                        if !send_disabled {
                            send_next(&this.deck, &this.pane_id, cx);
                        }
                    }))),
            )
    }
}

impl QueueDialog {
    fn move_item(&mut self, item_id: &str, delta: isize, cx: &mut Context<Self>) {
        let pane_id = self.pane_id.clone();
        self.deck.update(cx, |d, cx| {
            d.queues.move_by(&pane_id, item_id, delta);
            cx.notify();
        });
    }
}
