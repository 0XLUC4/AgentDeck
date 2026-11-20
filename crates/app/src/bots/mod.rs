//! Local bot profiles, reusable instructions and task history.

mod avatar;
mod data;
mod schedule;

pub(crate) use data::record_result;
pub(crate) use schedule::run_due;

use agentdeck_core::agents::AgentKind;
use agentdeck_core::logic::automations::{Schedule, schedule_label};
use agentdeck_core::logic::deck::new_id;
use agentdeck_core::logic::text::{AGENT_KINDS, agent_name, format_since, now_ms};
use agentdeck_core::logic::theme::{FONT_SANS, Palette};
use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable as _;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use self::data::Bot;
use crate::panes::{agent_logo, icon_button, status_label};
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

actions!(agentdeck, [CloseBots]);

pub(crate) fn init(cx: &mut App) {
    data::init(cx);
    cx.bind_keys([KeyBinding::new("escape", CloseBots, Some("Bots"))]);
}

pub fn open(deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| BotsView::new(deck.clone(), window, cx));
    let focus = view.read(cx).focus.clone();
    crate::skills::show_page(&deck, "bots", view.into(), focus, window, cx);
}

struct BotsView {
    deck: Entity<Deck>,
    focus: FocusHandle,
    bots: Vec<Bot>,
    selected: Option<String>,
    expanded_run: Option<String>,
    draft: Option<Bot>,
    name: Entity<InputState>,
    folder: Entity<InputState>,
    mission: Entity<TextareaState>,
    memory: Entity<TextareaState>,
    task: Entity<TextareaState>,
    error: Option<String>,
    _bots_changed: Subscription,
    _deck_changed: Subscription,
}

impl BotsView {
    fn new(deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Bot name"));
        let folder = cx.new(|cx| InputState::new(window, cx).placeholder("Working folder"));
        let mut textarea = |placeholder: &'static str| cx.new(|cx| TextareaState::new(window, cx).rows(3).placeholder(placeholder));
        let mission = textarea("What is this bot responsible for?");
        let memory = textarea("Project context and preferences this bot should remember");
        let task = textarea("What would you like this bot to do?");
        let mut view = Self {
            _bots_changed: cx.observe_global::<data::Bots>(|this, cx| {
                this.reload(cx);
                cx.notify();
            }),
            _deck_changed: cx.observe(&deck, |_, _, cx| cx.notify()),
            deck,
            focus: cx.focus_handle(),
            bots: vec![],
            selected: None,
            expanded_run: None,
            draft: None,
            name,
            folder,
            mission,
            memory,
            task,
            error: None,
        };
        view.reload(cx);
        if view.bots.is_empty() && view.error.is_none() {
            view.new_profile(window, cx);
        }
        view
    }

    fn reload(&mut self, cx: &App) {
        match data::list(&self.deck, cx) {
            Ok(bots) => {
                if !bots.iter().any(|bot| Some(&bot.id) == self.selected.as_ref()) {
                    self.selected = bots.first().map(|bot| bot.id.clone());
                }
                self.bots = bots;
            }
            Err(message) => self.error = Some(message),
        }
    }

    fn select(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_ref() != Some(&id) {
            self.task.update(cx, |state, cx| state.set_value("", window, cx));
            self.expanded_run = None;
        }
        self.selected = Some(id);
        self.draft = None;
        self.error = None;
        cx.notify();
    }

    fn new_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = &self.deck.read(cx).state;
        let cwd = &state.active_workspace().path;
        self.edit(
            Bot::new(if cwd.is_empty() { state.last_cwd.clone() } else { cwd.clone() }),
            window,
            cx,
        );
    }

    fn edit(&mut self, bot: Bot, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |s, cx| s.set_value(bot.name.clone(), window, cx));
        self.folder.update(cx, |s, cx| s.set_value(bot.cwd.clone(), window, cx));
        self.mission.update(cx, |s, cx| s.set_value(bot.mission.clone(), window, cx));
        self.memory.update(cx, |s, cx| s.set_value(bot.memory.clone(), window, cx));
        self.draft = Some(bot);
        self.error = None;
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut bot) = self.draft.clone() else {
            return;
        };
        bot.name = self.name.read(cx).value().trim().to_owned();
        bot.cwd = self.folder.read(cx).value().trim().to_owned();
        bot.mission = self.mission.read(cx).value().trim().to_owned();
        bot.memory = self.memory.read(cx).value().trim().to_owned();
        let id = bot.id.clone();
        match data::save(&self.deck, bot, cx) {
            Ok(()) => {
                self.reload(cx);
                self.select(id, window, cx);
            }
            Err(message) => self.error = Some(message),
        }
        cx.notify();
    }

    fn send_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected.as_deref() else {
            return;
        };
        let task = self.task.read(cx).value().trim().to_owned();
        match data::run(&self.deck, id, &task, cx) {
            Ok(pane_id) => {
                self.task.update(cx, |state, cx| state.set_value("", window, cx));
                crate::skills::close_page(cx);
                crate::panes::jump_to(&pane_id, cx);
                window.refresh();
            }
            Err(message) => self.error = Some(message),
        }
        cx.notify();
    }

    fn remove(&self, id: String, name: String, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |view, cx| {
            if crate::skills::confirm(format!("Delete {name} and its saved task history?"), "Delete bot", cx).await {
                let _ = view.update_in(cx, |this, window, cx| {
                    match data::remove(&this.deck, &id, cx) {
                        Ok(()) => {
                            this.error = None;
                            this.reload(cx);
                            this.task.update(cx, |s, cx| s.set_value("", window, cx));
                            if this.bots.is_empty() {
                                this.new_profile(window, cx);
                            }
                        }
                        Err(message) => this.error = Some(message),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn profile_form(&self, bot: &Bot, p: &Palette, cx: &Context<Self>) -> Div {
        let existing = self.bots.iter().any(|saved| saved.id == bot.id);
        let agent_choices = AGENT_KINDS.into_iter().filter(|k| *k != AgentKind::Shell).map(|kind| {
            Button::new(SharedString::from(format!("bot-agent-{}", agent_name(kind))))
                .small()
                .label(agent_name(kind))
                .selected(bot.agent == kind)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(draft) = &mut this.draft {
                        draft.agent = kind;
                    }
                    cx.notify();
                }))
        });
        let schedules = [None, Some(Schedule::EveryHours(1)), Some(Schedule::DailyAt { hour: 9, minute: 0 })];
        let schedule_choices = schedules.into_iter().enumerate().map(|(i, schedule)| {
            Button::new(("bot-schedule", i))
                .small()
                .label(schedule.map(schedule_label).unwrap_or_else(|| "Off".into()))
                .selected(bot.schedule == schedule)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(draft) = &mut this.draft {
                        draft.schedule = schedule;
                    }
                    cx.notify();
                }))
        });
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(title(if existing { "Edit bot" } else { "Create a bot" }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(avatar::avatar(&bot.avatar_seed, 60.0))
                    .child(
                        Button::new("bot-avatar")
                            .small()
                            .label("Change avatar")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(bot) = &mut this.draft {
                                    bot.avatar_seed = new_id();
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(field("Name", Input::new(&self.name), p))
            .child(field("Agent", div().flex().flex_wrap().gap_1().children(agent_choices), p))
            .child(field("Working folder", Input::new(&self.folder), p))
            .child(field("Mission", Textarea::new(&self.mission).h(ui(92.0)), p))
            .child(field("Memory shared across tasks", Textarea::new(&self.memory).h(ui(92.0)), p))
            .child(field(
                "Run the mission while AgentDeck is open",
                div().flex().flex_wrap().gap_1().children(schedule_choices),
                p,
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("bot-save")
                            .primary()
                            .small()
                            .label(if existing { "Save changes" } else { "Create bot" })
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    )
                    .when(self.selected.is_some(), |row| {
                        row.child(
                            Button::new("bot-cancel")
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.draft = None;
                                    this.error = None;
                                    cx.notify();
                                })),
                        )
                    }),
            )
    }

    fn detail(&self, bot: &Bot, p: &Palette, cx: &Context<Self>) -> Div {
        let edit_id = bot.id.clone();
        let (delete_id, delete_name) = (bot.id.clone(), bot.name.clone());
        let rows = bot.history.iter().rev().map(|run| {
            let pane = self.deck.read(cx).state.panes.get(&run.pane_id);
            let status = pane.map_or("Closed", |pane| if pane.exited { "Stopped" } else { status_label(pane.status) });
            let pane_id = run.pane_id.clone();
            let expanded = self.expanded_run.as_deref() == Some(pane_id.as_str());
            let summary = run.result.as_deref().filter(|s| !s.trim().is_empty());
            let expand_id = pane_id.clone();
            let row = div()
                .flex()
                .items_center()
                .gap_3()
                .py_2()
                .child(
                    div().flex_1().min_w_0().child(div().truncate().child(run.title.clone())).child(
                        div()
                            .text_xs()
                            .text_color(hsla(p.muted))
                            .child(format!("{status} · {} ago", format_since(now_ms() - run.started_at))),
                    ),
                )
                .when(summary.is_some(), |row| {
                    row.child(
                        Button::new(SharedString::from(format!("bot-result-{expand_id}")))
                            .small()
                            .label(if expanded { "Hide result" } else { "Result" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.expanded_run = (!expanded).then(|| expand_id.clone());
                                cx.notify();
                            })),
                    )
                })
                .when(pane.is_some(), |row| {
                    row.child(
                        Button::new(SharedString::from(format!("bot-open-{pane_id}")))
                            .small()
                            .label("Open")
                            .on_click(move |_, window, cx| {
                                crate::skills::close_page(cx);
                                crate::panes::jump_to(&pane_id, cx);
                                window.refresh();
                            }),
                    )
                });
            div()
                .flex()
                .flex_col()
                .border_b_1()
                .border_color(hsla(p.line))
                .child(row)
                .when_some(summary.filter(|_| expanded), |row, summary| {
                    let id = bot.id.clone();
                    let result = summary.to_owned();
                    row.child(
                        div().pb_3().flex().flex_col().gap_2().child(summary.to_owned()).child(
                            div().flex().child(
                                Button::new(SharedString::from(format!("bot-remember-{}", run.pane_id)))
                                    .small()
                                    .label("Add to memory…")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let Some(mut bot) = this.bots.iter().find(|b| b.id == id).cloned() {
                                            if !bot.memory.is_empty() {
                                                bot.memory.push_str("\n\n");
                                            }
                                            bot.memory.push_str(&result);
                                            this.edit(bot, window, cx);
                                        }
                                    })),
                            ),
                        ),
                    )
                })
        });
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(avatar::avatar(&bot.avatar_seed, 60.0))
                    .child(
                        div().flex_1().min_w_0().child(title(bot.name.clone())).child(
                            div()
                                .flex()
                                .gap_1p5()
                                .items_center()
                                .text_xs()
                                .text_color(hsla(p.muted))
                                .child(agent_logo(bot.agent, 14.0, None, p))
                                .child(agent_name(bot.agent)),
                        ),
                    )
                    .child(
                        Button::new("bot-edit")
                            .small()
                            .label("Edit bot")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(bot) = this.bots.iter().find(|bot| bot.id == edit_id).cloned() {
                                    this.edit(bot, window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("bot-delete")
                            .ghost()
                            .small()
                            .icon(IconName::Trash)
                            .tooltip("Delete bot")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.remove(delete_id.clone(), delete_name.clone(), window, cx);
                            })),
                    ),
            )
            .child(field("Mission", div().child(bot.mission.clone()), p))
            .when(!bot.memory.is_empty(), |d| {
                d.child(field("Memory", div().child(bot.memory.clone()), p))
            })
            .child(field("Working folder", div().child(bot.cwd.clone()), p))
            .when_some(bot.schedule, |d, schedule| {
                d.child(field("Schedule", div().child(schedule_label(schedule)), p))
            })
            .child(field("New task", Textarea::new(&self.task).h(ui(94.0)), p))
            .child(
                div().flex().child(
                    Button::new("bot-run")
                        .small()
                        .primary()
                        .label("Start task")
                        .on_click(cx.listener(|this, _, window, cx| this.send_task(window, cx))),
                ),
            )
            .child(field(
                "Task history",
                div().flex().flex_col().children(rows).when(bot.history.is_empty(), |list| {
                    list.child(div().text_color(hsla(p.subtle)).child("No tasks yet."))
                }),
                p,
            ))
    }
}

fn title(label: impl Into<SharedString>) -> Div {
    div()
        .truncate()
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(ui(19.0))
        .child(label.into())
}

fn field(label: &'static str, control: impl IntoElement, p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(hsla(p.muted))
                .child(label),
        )
        .child(control)
}

impl Render for BotsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(hsla(p.line))
            .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child("Bots"))
            .child(
                Button::new("bot-new")
                    .small()
                    .label("New bot")
                    .icon(IconName::Plus)
                    .on_click(cx.listener(|this, _, window, cx| this.new_profile(window, cx))),
            )
            .child(icon_button("bots-close", IconName::X, "Close (Esc)").on_click(crate::skills::close));
        let list = div()
            .id("bots-list")
            .w(ui(196.0))
            .flex_shrink_0()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .border_r_1()
            .border_color(hsla(p.line))
            .children(self.bots.iter().map(|bot| {
                let id = bot.id.clone();
                let active = self.draft.as_ref().map(|v| &v.id).or(self.selected.as_ref()) == Some(&id);
                div()
                    .id(SharedString::from(format!("bot-{id}")))
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |s| s.bg(hsla(p.hover)))
                    .when(active, |row| row.bg(hsla(p.raised)))
                    .child(avatar::avatar(&bot.avatar_seed, 34.0))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .child(div().truncate().font_weight(FontWeight::MEDIUM).child(bot.name.clone()))
                            .child(div().text_xs().text_color(hsla(p.muted)).child(agent_name(bot.agent))),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.select(id.clone(), window, cx)))
            }));
        let body = self.draft.as_ref().map(|draft| self.profile_form(draft, &p, cx)).or_else(|| {
            self.bots
                .iter()
                .find(|bot| Some(&bot.id) == self.selected.as_ref())
                .map(|bot| self.detail(bot, &p, cx))
        });
        let content = div()
            .id("bot-content")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .p_5()
            .child(
                div()
                    .max_w(ui(680.0))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .children(self.error.clone().map(|message| {
                        div()
                            .rounded_md()
                            .p_2()
                            .bg(hsla_a(p.bad, 0.10))
                            .text_color(hsla(p.bad))
                            .child(message)
                    }))
                    .children(body),
            );
        div()
            .id("bots")
            .track_focus(&self.focus)
            .key_context("Bots")
            .on_action(|_: &CloseBots, window, cx| crate::skills::close(window, cx))
            .size_full()
            .flex()
            .flex_col()
            .occlude()
            .bg(hsla(p.panel))
            .text_color(hsla(p.fg))
            .font_family(FONT_SANS)
            .text_size(ui(13.0))
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(!self.bots.is_empty(), |body| body.child(list))
                    .child(content),
            )
    }
}
