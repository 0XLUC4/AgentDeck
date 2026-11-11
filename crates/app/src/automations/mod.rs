//! Automations: prompts that run on a schedule while AgentDeck is open. A due automation starts
//! a fresh agent in its folder and sends it the prompt. Model: agentdeck_core::logic::automations.
//!
//! The list lives in a global (saved in local.json). The view is a page in place of the panes
//! (`skills::show_page`), opened from the rail: the automations in a side column, the form beside.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use agentdeck_core::agents::AgentKind;
use agentdeck_core::logic::automations::{
    Automation, MAX_HOURS, Schedule, is_due, next_run_label, ran_this_minute, schedule_label, validate,
};
use agentdeck_core::logic::deck::{DeckState, PaneSpec, folder_name, new_id};
use agentdeck_core::logic::text::{AGENT_KINDS, agent_name, now_ms, split_args};
use agentdeck_core::logic::theme::{FONT_SANS, Palette};
use agentdeck_core::persist;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::layout::DeckView;
use crate::panes::{agent_logo, icon_button};
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

actions!(agentdeck, [CloseAutomations]);

/// Key context of the view (Esc closes).
const CONTEXT: &str = "Automations";
const TICK: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Automations {
    list: Vec<Automation>,
}

impl Global for Automations {}

/// Registers the key binding and, once the main `DeckView` exists, starts the scheduler.
pub fn init(cx: &mut App) {
    cx.set_global(Automations::default());
    cx.bind_keys([KeyBinding::new("escape", CloseAutomations, Some(CONTEXT))]);
    static STARTED: AtomicBool = AtomicBool::new(false);
    cx.observe_new(|view: &mut DeckView, _, cx| {
        if !STARTED.swap(true, Ordering::SeqCst) {
            let deck = view.deck().clone();
            start(deck, cx);
        }
    })
    .detach();
}

/// Loads the automations and checks every minute which ones are due.
fn start(deck: Entity<Deck>, cx: &mut App) {
    let mut list = persist::load_automations(&deck.read(cx).data_dir);
    let now = now_ms();
    // A hand-written entry without dates would be due at once: it waits one interval instead.
    for automation in list.iter_mut().filter(|a| a.created_at == 0 && a.last_run == 0) {
        automation.created_at = now;
    }
    cx.global_mut::<Automations>().list = list;
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            cx.update(|cx| run_due(&deck, cx));
        }
    })
    .detach();
}

fn run_due(deck: &Entity<Deck>, cx: &mut App) {
    let now = now_ms();
    // global_mut also redraws the open view, whose "next run" texts follow the clock.
    let due: Vec<String> = cx.global_mut::<Automations>().list.iter().filter(|a| is_due(a, now)).map(|a| a.id.clone()).collect();
    for id in due {
        // No session open: `last_run` is unchanged, the next minute tries again.
        let _ = run(deck, &id, cx);
    }
}

/// Changes the list and saves it; the open view redraws (it observes the global).
fn edit(deck: &Entity<Deck>, cx: &mut App, change: impl FnOnce(&mut Vec<Automation>)) {
    let state = cx.global_mut::<Automations>();
    change(&mut state.list);
    let list = state.list.clone();
    if let Err(e) = persist::save_automations(&deck.read(cx).data_dir, &list) {
        deck.update(cx, |d, cx| d.toast(format!("Automations not saved: {e}"), cx));
    }
}

/// The session a run opens its agent in: one of the workspace linked to the folder (the visible
/// session when it is there), else the visible session.
fn session_for(state: &DeckState, cwd: &str) -> Option<String> {
    let active = state.active_tab();
    state
        .workspace_for_path(cwd)
        .and_then(|ws| {
            active
                .filter(|tab| tab.workspace_id == ws.id)
                .or_else(|| state.tabs.iter().find(|tab| tab.workspace_id == ws.id))
        })
        .or(active)
        .map(|tab| tab.id.clone())
}

/// Starts a fresh agent in the automation's folder and sends it the prompt.
/// Err says why nothing started; `last_run` is then unchanged.
fn run(deck: &Entity<Deck>, id: &str, cx: &mut App) -> Result<(), String> {
    let now = now_ms();
    let Some(automation) = cx.global::<Automations>().list.iter().find(|a| a.id == id).cloned() else {
        return Err("This automation no longer exists.".to_owned());
    };
    // A prompt typed into a plain shell would run as a command.
    if automation.agent == AgentKind::Shell || automation.prompt.trim().is_empty() {
        return Err("This automation has no agent or no prompt.".to_owned());
    }
    if ran_this_minute(&automation, now) {
        return Err("This automation started less than a minute ago.".to_owned());
    }
    let d = deck.read(cx);
    let Some(tab_id) = session_for(&d.state, &automation.cwd) else {
        return Err("Open a session first: the agent needs one to appear in.".to_owned());
    };
    let defaults = d.settings.agent(automation.agent);
    let mut spec = PaneSpec::new(automation.agent, automation.name.clone(), automation.cwd.clone());
    spec.id = new_id();
    spec.font_size = d.settings.font_size;
    spec.args = split_args(&defaults.args);
    spec.worktree = defaults.worktree;
    // Recorded before the agent starts: a slow start cannot run it twice.
    edit(deck, cx, |list| {
        if let Some(a) = list.iter_mut().find(|a| a.id == automation.id) {
            a.last_run = now;
        }
    });
    let name = automation.name;
    deck.update(cx, |d, cx| d.toast(format!("Automation started: {name}"), cx));
    let deck = deck.clone();
    cx.spawn(async move |cx| {
        if !crate::palette::race_one(deck.clone(), spec, tab_id, automation.prompt, cx).await {
            deck.update(cx, |d, cx| d.toast(format!("Automation {name}: the agent did not start, the prompt was not sent."), cx));
        }
    })
    .detach();
    Ok(())
}

/// Opens the Automations view.
pub fn open(deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| AutomationsView::new(deck.clone(), window, cx));
    let focus = view.read(cx).name.read(cx).focus_handle(cx);
    crate::skills::show_page(&deck, "automations", view.into(), focus, window, cx);
}

fn close(window: &mut Window, cx: &mut App) {
    crate::skills::close(window, cx);
}

/// The schedule typed in the form. `daily` picks which fields count.
fn parse_schedule(daily: bool, hours: &str, hour: &str, minute: &str) -> Result<Schedule, String> {
    let number = |raw: &str| raw.trim().parse::<u32>().ok();
    if daily {
        match (number(hour), number(minute)) {
            (Some(hour @ 0..=23), Some(minute @ 0..=59)) => Ok(Schedule::DailyAt { hour: hour as u8, minute: minute as u8 }),
            _ => Err("The time must be between 00:00 and 23:59.".to_owned()),
        }
    } else {
        match number(hours) {
            Some(hours) if (1..=MAX_HOURS).contains(&hours) => Ok(Schedule::EveryHours(hours)),
            _ => Err(format!("The interval must be a number of hours between 1 and {MAX_HOURS}.")),
        }
    }
}

pub struct AutomationsView {
    deck: Entity<Deck>,
    focus: FocusHandle,
    name: Entity<InputState>,
    folder: Entity<InputState>,
    hours: Entity<InputState>,
    hour: Entity<InputState>,
    minute: Entity<InputState>,
    prompt: Entity<TextareaState>,
    agent: AgentKind,
    daily: bool,
    error: Option<String>,
    _redraw: Subscription,
}

impl AutomationsView {
    fn new(deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let folder = {
            let state = &deck.read(cx).state;
            let path = state.active_workspace().path.clone();
            if path.is_empty() { state.last_cwd.clone() } else { path }
        };
        let mut input = |value: &str, placeholder: &'static str| {
            cx.new(|cx| {
                let mut state = InputState::new(window, cx).placeholder(placeholder);
                state.set_value(value.to_owned(), window, cx);
                state
            })
        };
        Self {
            name: input("", "Nightly dependency bump"),
            folder: input(&folder, "Folder the agent works in"),
            hours: input("24", "24"),
            hour: input("06", "06"),
            minute: input("00", "00"),
            prompt: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder("Bump the dependencies and open a pull request.").rows(5)
            }),
            deck,
            focus: cx.focus_handle(),
            agent: AgentKind::Claude,
            daily: true,
            error: None,
            _redraw: cx.observe_global::<Automations>(|_, cx| cx.notify()),
        }
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = |state: &Entity<InputState>| state.read(cx).value().trim().to_owned();
        let schedule = parse_schedule(self.daily, &text(&self.hours), &text(&self.hour), &text(&self.minute));
        let automation = schedule.and_then(|schedule| {
            let automation = Automation {
                id: new_id(),
                name: text(&self.name),
                agent: self.agent,
                cwd: text(&self.folder),
                prompt: self.prompt.read(cx).value().trim().to_owned(),
                schedule,
                enabled: true,
                last_run: 0,
                created_at: now_ms(),
            };
            validate(&automation).map(|()| automation)
        });
        match automation {
            Err(message) => self.error = Some(message),
            Ok(automation) => {
                self.error = None;
                edit(&self.deck, cx, |list| list.push(automation));
                self.name.update(cx, |state, cx| state.set_value("", window, cx));
                self.prompt.update(cx, |state, cx| state.set_value("", window, cx));
            }
        }
        cx.notify();
    }

    fn row(&self, automation: &Automation, now: i64, p: &Palette) -> Div {
        let id = automation.id.clone();
        let enabled = automation.enabled;
        let due = is_due(automation, now);
        let (toggle_deck, toggle_id) = (self.deck.clone(), id.clone());
        let (run_deck, run_id) = (self.deck.clone(), id.clone());
        let (delete_deck, delete_id, delete_name) = (self.deck.clone(), id.clone(), automation.name.clone());
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(ui(8.0))
            .border_1()
            .border_color(hsla(p.line))
            .bg(hsla(p.base))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(agent_logo(automation.agent, 16.0, None, p))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(hsla(if enabled { p.fg } else { p.muted }))
                            .child(automation.name.clone()),
                    )
                    .child(toggle(SharedString::from(format!("automation-enabled-{id}")), enabled, p).on_click(move |_, _, cx| {
                        edit(&toggle_deck, cx, |list| {
                            if let Some(a) = list.iter_mut().find(|a| a.id == toggle_id) {
                                a.enabled = !a.enabled;
                                // Switched back on: the schedule restarts from now instead of catching up.
                                a.created_at = now_ms();
                            }
                        });
                    })),
            )
            .child(div().truncate().text_xs().text_color(hsla(p.subtle)).child(format!(
                "{} in {} \u{b7} {}",
                agent_name(automation.agent),
                folder_name(&automation.cwd),
                schedule_label(automation.schedule)
            )))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(hsla(if due { p.wait } else { p.muted }))
                            .child(next_run_label(automation, now)),
                    )
                    .child(icon_button(SharedString::from(format!("automation-run-{id}")), IconName::Play, "Run now").on_click(
                        move |_, cx| {
                            if let Err(message) = run(&run_deck, &run_id, cx) {
                                run_deck.update(cx, |d, cx| d.toast(message, cx));
                            }
                        },
                    ))
                    .child(
                        icon_button(SharedString::from(format!("automation-delete-{id}")), IconName::Trash, "Delete")
                            .danger()
                            .on_click(move |_, cx| {
                                let confirmed = rfd::MessageDialog::new()
                                    .set_title("AgentDeck")
                                    .set_description(format!("Delete automation \"{delete_name}\"?"))
                                    .set_buttons(rfd::MessageButtons::YesNo)
                                    .set_level(rfd::MessageLevel::Warning)
                                    .show()
                                    == rfd::MessageDialogResult::Yes;
                                if confirmed {
                                    edit(&delete_deck, cx, |list| list.retain(|a| a.id != delete_id));
                                }
                            }),
                    ),
            )
    }

    fn form(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let agents = AGENT_KINDS.into_iter().filter(|kind| *kind != AgentKind::Shell).map(|kind| {
            chip(SharedString::from(format!("automation-agent-{}", agent_name(kind))), self.agent == kind, p)
                .child(agent_logo(kind, 12.0, None, p))
                .child(agent_name(kind))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.agent = kind;
                    cx.notify();
                }))
        });
        let schedule_chip = |id: &'static str, label: &'static str, daily: bool, cx: &Context<Self>| {
            chip(id, self.daily == daily, p).child(label).on_click(cx.listener(move |this, _, _, cx| {
                this.daily = daily;
                cx.notify();
            }))
        };
        let number = |state: &Entity<InputState>| div().w(ui(56.0)).child(Input::new(state).h(ui(32.0)));
        let schedule = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(schedule_chip("automation-daily", "Daily at a time", true, cx))
            .child(schedule_chip("automation-every", "Every N hours", false, cx))
            .map(|row| {
                if self.daily {
                    row.child(div().ml_2().text_color(hsla(p.muted)).child("at"))
                        .child(number(&self.hour))
                        .child(":")
                        .child(number(&self.minute))
                } else {
                    row.child(div().ml_2().text_color(hsla(p.muted)).child("every"))
                        .child(number(&self.hours))
                        .child(div().text_color(hsla(p.muted)).child("hours"))
                }
            });
        let accent_hover = hsla(p.accent_hover);
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_size(ui(15.0)).font_weight(FontWeight::MEDIUM).child("New automation"))
            .child(field("Name", Input::new(&self.name).h(ui(32.0)), p))
            .child(field("Agent", div().flex().flex_wrap().gap_1p5().children(agents), p))
            .child(field("Folder", Input::new(&self.folder).h(ui(32.0)), p))
            .child(field("Schedule", schedule, p))
            .child(field("Prompt", Textarea::new(&self.prompt).h(ui(180.0)), p))
            .children(self.error.clone().map(|message| {
                div().rounded_md().bg(hsla_a(p.bad, 0.1)).px_3().py_2().text_xs().text_color(hsla(p.bad)).child(message)
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().text_xs().text_color(hsla(p.subtle)).child(
                        "Runs while AgentDeck is open. Each run starts a fresh agent in the folder and sends it the prompt.",
                    ))
                    .child(
                        div()
                            .id("automation-add")
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_2()
                            .h(ui(32.0))
                            .px_3()
                            .rounded(ui(6.0))
                            .bg(hsla(p.accent))
                            .text_color(hsla(p.on_accent))
                            .font_weight(FontWeight::MEDIUM)
                            .cursor_pointer()
                            .hover(move |s| s.bg(accent_hover))
                            .child(IconName::Plus)
                            .child("Add automation")
                            .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                    ),
            )
    }
}

/// A form field: its label above the control.
fn field(label: &'static str, control: impl IntoElement, p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(ui(6.0))
        .child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(hsla(p.muted)).child(label))
        .child(control)
}

/// Small bordered choice button; the selected one has the accent border.
fn chip(id: impl Into<ElementId>, selected: bool, p: &Palette) -> Stateful<Div> {
    let (hover, fg) = (hsla(p.hover), hsla(p.fg));
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_1p5()
        .h(ui(28.0))
        .px_2p5()
        .rounded(ui(6.0))
        .border_1()
        .border_color(hsla(if selected { p.accent } else { p.line }))
        .text_xs()
        .text_color(hsla(if selected { p.fg } else { p.muted }))
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(fg))
}

/// On/off switch (same look as the Settings one). The caller adds `.on_click`.
fn toggle(id: impl Into<ElementId>, checked: bool, p: &Palette) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .h(ui(20.0))
        .w(ui(36.0))
        .px(ui(2.0))
        .rounded_full()
        .border_1()
        .cursor_pointer()
        .when(checked, |d| d.justify_end().border_color(hsla(p.accent)).bg(hsla(p.accent)))
        .when(!checked, |d| d.border_color(hsla(p.line_strong)).bg(hsla(p.base)))
        .child(div().size(ui(14.0)).rounded_full().bg(hsla(if checked { p.on_accent } else { p.subtle })))
}

impl Render for AutomationsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let now = now_ms();
        let list = cx.global::<Automations>().list.clone();
        let rows: Vec<Div> = list.iter().map(|automation| self.row(automation, now, &p)).collect();
        let empty = rows.is_empty().then(|| {
            div().px_1().py_2().text_xs().text_color(hsla(p.subtle)).child("None yet. Fill the form: it then runs by itself on its schedule.")
        });
        let header = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(hsla(p.line))
            .px_5()
            .py_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(ui(15.0)).font_weight(FontWeight::MEDIUM).child("Automations"))
                    .child(div().text_xs().text_color(hsla(p.subtle)).child("Prompts that start an agent on a schedule")),
            )
            .child(icon_button("automations-close", IconName::X, "Close (Esc)").large().on_click(close));
        let count = rows.len();
        let side = div()
            .id("automations-list")
            .w(ui(320.0))
            .flex_shrink_0()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_r_1()
            .border_color(hsla(p.line))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_1()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(hsla(p.muted))
                    .child("Scheduled")
                    .child(div().text_color(hsla(p.subtle)).child(count.to_string())),
            )
            .children(empty)
            .children(rows);
        let form = div()
            .id("automations-form")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .p_6()
            .child(self.form(&p, cx).max_w(ui(640.0)));
        div()
            .id("automations")
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(|_: &CloseAutomations, window, cx| close(window, cx))
            .size_full()
            .flex()
            .flex_col()
            .occlude()
            .bg(hsla(p.panel))
            .text_color(hsla(p.fg))
            .text_size(ui(13.0))
            .font_family(FONT_SANS)
            .child(header)
            .child(div().flex().flex_1().min_h_0().child(side).child(form))
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use agentdeck_core::logic::automations::Schedule;
    use agentdeck_core::logic::deck::{DeckState, Tab, new_workspace};

    use super::{parse_schedule, session_for};

    fn state() -> DeckState {
        let tab = |id: &str, workspace_id: &str| Tab {
            id: id.into(),
            name: "S".into(),
            workspace_id: workspace_id.into(),
            pane_ids: vec![],
            layout: None,
        };
        let (mut repo, mut other) = (new_workspace("C:/code/repo", None), new_workspace("C:/code/other", None));
        repo.id = "repo".into();
        other.id = "other".into();
        let mut state = DeckState::default();
        state.workspaces = vec![repo, other];
        state.tabs = vec![tab("r1", "repo"), tab("r2", "repo"), tab("o1", "other")];
        state.active_tab_id = "o1".into();
        state
    }

    #[test]
    fn automations_session_is_in_the_folder_workspace() {
        let mut state = state();
        // Another workspace is visible: the first session of the folder's workspace.
        assert_eq!(session_for(&state, "C:/code/repo").as_deref(), Some("r1"));
        // The visible session belongs to the folder's workspace: that one.
        state.active_tab_id = "r2".into();
        assert_eq!(session_for(&state, "C:/code/repo").as_deref(), Some("r2"));
        // No workspace for the folder: the visible session.
        assert_eq!(session_for(&state, "C:/elsewhere").as_deref(), Some("r2"));
        // No session at all: nothing to run in.
        state.tabs.clear();
        assert_eq!(session_for(&state, "C:/code/repo"), None);
    }

    #[test]
    fn automations_schedule_from_the_form() {
        assert_eq!(parse_schedule(false, " 6 ", "", ""), Ok(Schedule::EveryHours(6)));
        assert_eq!(parse_schedule(true, "", "06", "5"), Ok(Schedule::DailyAt { hour: 6, minute: 5 }));
        assert!(parse_schedule(false, "0", "", "").is_err());
        assert!(parse_schedule(false, "169", "", "").is_err());
        assert!(parse_schedule(false, "soon", "", "").is_err());
        assert!(parse_schedule(true, "", "24", "00").is_err());
        assert!(parse_schedule(true, "", "6", "").is_err());
    }
}
