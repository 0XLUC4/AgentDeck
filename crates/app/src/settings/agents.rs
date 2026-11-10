//! Settings > Agents: folder trust, resume, ports, rate-limit retry, budget, auto-allowed
//! permissions and the defaults prefilled in "Add agent".

use agentdeck_core::agents::AgentKind;
use agentdeck_core::logic::settings::{AutoAllowRule, BudgetSettings, Settings};
use agentdeck_core::logic::text::{AGENT_KINDS, agent_name};
use agentdeck_core::logic::theme::FONT_MONO;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use super::SettingsDialog;
use super::controls::{button, field, hint_text, icon_button, row, section, toggle};
use crate::theme::{hsla, ui};

pub(super) struct Agents {
    base_port: Entity<InputState>,
    worktree_setup: Entity<InputState>,
    retry: Entity<InputState>,
    daily_usd: Entity<InputState>,
    pane_usd: Entity<InputState>,
    rule: Entity<InputState>,
    folder: Entity<InputState>,
    args: Vec<(AgentKind, Entity<InputState>)>,
}

/// USD field saved on blur; empty or 0 means off.
fn usd_input(
    window: &mut Window,
    cx: &mut Context<SettingsDialog>,
    subs: &mut Vec<Subscription>,
    value: f64,
    field: fn(&mut BudgetSettings) -> &mut f64,
) -> Entity<InputState> {
    let shown = if value > 0.0 { value.to_string() } else { String::new() };
    let state = super::input(window, cx, "Off", &shown);
    subs.push(cx.subscribe(&state, move |this, state, event: &InputEvent, cx| {
        if !matches!(event, InputEvent::Blur) {
            return;
        }
        let value = state.read(cx).value().trim().parse::<f64>().unwrap_or(0.0);
        let value = if value.is_finite() { value.max(0.0) } else { 0.0 };
        let mut budget = this.settings(cx).budget.clone();
        if *field(&mut budget) != value {
            this.set(cx, |s| *field(&mut s.budget) = value);
        }
    }));
    state
}

impl Agents {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsDialog>, s: &Settings, subs: &mut Vec<Subscription>) -> Self {
        let base_port = super::number_input(
            window,
            cx,
            subs,
            u32::from(s.base_port),
            (1024, 65000),
            |s| u32::from(s.base_port),
            |s, v| s.base_port = u16::try_from(v).unwrap_or(0),
        );
        let worktree_setup = super::input(window, cx, "pnpm install", &s.worktree_setup);
        // Saved on blur: a half-typed command must never be the one a spawn runs.
        subs.push(cx.subscribe(&worktree_setup, |this, state, event: &InputEvent, cx| {
            let command = state.read(cx).value().trim().to_owned();
            if matches!(event, InputEvent::Blur) && command != this.settings(cx).worktree_setup {
                this.set(cx, |s| s.worktree_setup = command);
            }
        }));
        let retry = super::number_input(
            window,
            cx,
            subs,
            s.retry_rate_limit_minutes,
            (0, 600),
            |s| s.retry_rate_limit_minutes,
            |s, v| s.retry_rate_limit_minutes = v,
        );
        let daily_usd = usd_input(window, cx, subs, s.budget.daily_usd, |b| &mut b.daily_usd);
        let pane_usd = usd_input(window, cx, subs, s.budget.pane_usd, |b| &mut b.pane_usd);

        let rule = super::input(window, cx, "Bash(pnpm test *)", "");
        subs.push(cx.subscribe_in(&rule, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => cx.notify(), // enables "Add"
            InputEvent::PressEnter { .. } => this.add_rule(window, cx),
            _ => {}
        }));
        let folder = super::input(window, cx, "Folder (empty: all)", "");
        subs.push(cx.subscribe_in(&folder, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.add_rule(window, cx);
            }
        }));

        let args = AGENT_KINDS
            .into_iter()
            .map(|kind| {
                let state = super::input(window, cx, "Default arguments", &s.agent(kind).args);
                subs.push(cx.subscribe(&state, move |this, state, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let args = state.read(cx).value().to_string();
                        this.set(cx, |s| {
                            let mut defaults = s.agent(kind);
                            defaults.args = args;
                            s.agents.insert(kind, defaults);
                        });
                    }
                }));
                (kind, state)
            })
            .collect();
        Self {
            base_port,
            worktree_setup,
            retry,
            daily_usd,
            pane_usd,
            rule,
            folder,
            args,
        }
    }
}

impl SettingsDialog {
    fn add_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rule = self.agents.rule.read(cx).value().trim().to_owned();
        if rule.is_empty() {
            return;
        }
        let folder = self.agents.folder.read(cx).value().trim().to_owned();
        self.set(cx, |s| s.auto_allow.push(AutoAllowRule { rule, folder }));
        self.agents.rule.update(cx, |s, cx| s.set_value("", window, cx));
    }

    fn auto_allow_rows(&self, s: &Settings, cx: &mut Context<Self>) -> AnyElement {
        let p = crate::theme::palette(cx);
        let a = &self.agents;
        let rule_empty = a.rule.read(cx).value().trim().is_empty();
        let mut add = button("set-rule-add", None, "Add", rule_empty, &p);
        if !rule_empty {
            add = add.on_click(cx.listener(|this, _, window, cx| this.add_rule(window, cx)));
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .py_3()
            .child(div().text_color(hsla(p.fg)).child("Auto-allowed permissions (Claude Code)"))
            .child(hint_text(
                "AgentDeck answers these itself. \"Read\" covers a whole tool; \"Bash(pnpm test)\" one command, \
                 \"Bash(pnpm test *)\" its variants. Chained commands (&&, ;, |, $(...)) are never auto-allowed.",
                &p,
            ))
            .children(s.auto_allow.iter().enumerate().map(|(i, r)| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(FONT_MONO)
                            .text_color(hsla(p.fg))
                            .child(r.rule.clone()),
                    )
                    .child(
                        div()
                            .max_w(ui(240.0))
                            .truncate()
                            .text_color(hsla(p.subtle))
                            .child(if r.folder.is_empty() { "All folders".to_owned() } else { r.folder.clone() }),
                    )
                    .child(icon_button(("rule-remove", i), IconName::Trash, false, &p).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.set(cx, |s| {
                                if i < s.auto_allow.len() {
                                    s.auto_allow.remove(i);
                                }
                            })
                        },
                    )))
            }))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(field(&a.rule, None, true))
                    .child(field(&a.folder, Some(192.0), false))
                    .child(add),
            )
            .into_any_element()
    }

    pub(super) fn render_agents(&mut self, cx: &mut Context<Self>) -> Div {
        let p = crate::theme::palette(cx);
        let s = self.settings(cx).clone();
        let a = &self.agents;
        let mut rows = vec![
            row(
                "Trust agent folders automatically",
                Some(
                    "Skips each agent's folder trust prompt for folders you open in AgentDeck. Only enable for code \
                     you trust: project hooks and MCP servers will run without asking.",
                ),
                toggle("set-trust-folders", s.trust_agent_folders, &p)
                    .on_click(cx.listener(|this, _, _, cx| this.set(cx, |s| s.trust_agent_folders = !s.trust_agent_folders))),
                &p,
            ),
            row(
                "Resume conversations",
                Some("Claude Code agents pick up their previous conversation when AgentDeck restarts or a session is reopened"),
                toggle("set-resume", s.resume_sessions, &p)
                    .on_click(cx.listener(|this, _, _, cx| this.set(cx, |s| s.resume_sessions = !s.resume_sessions))),
                &p,
            ),
            row(
                "Port per agent",
                Some("Each agent gets PORT=this+n so dev servers started in parallel don't collide"),
                field(&a.base_port, Some(96.0), false),
                &p,
            ),
            row(
                "Worktree setup command",
                Some(
                    "Runs once in each new agent worktree, before the agent starts: install dependencies, copy \
                     git-ignored files. AGENTDECK_ROOT is the main checkout (copy \"%AGENTDECK_ROOT%\\.env\" .env)",
                ),
                field(&a.worktree_setup, Some(256.0), true),
                &p,
            ),
            row(
                "Continue after a rate limit",
                Some("Sends \"continue\" to an agent stopped by a rate limit, after this many minutes"),
                field(&a.retry, Some(96.0), false),
                &p,
            ),
            row(
                "Daily cost limit (USD)",
                Some(
                    "Estimated cost of every Claude Code and Codex session on this computer today, including ones \
                     started outside AgentDeck",
                ),
                field(&a.daily_usd, Some(96.0), false),
                &p,
            ),
            row("Cost limit per agent (USD)", Some("Since the agent was started"), field(&a.pane_usd, Some(96.0), false), &p),
            row(
                "Stop at the limit",
                Some("Interrupts the working agents and pauses the orchestrator, after the warning"),
                toggle("set-budget-stop", s.budget.interrupt, &p)
                    .on_click(cx.listener(|this, _, _, cx| this.set(cx, |s| s.budget.interrupt = !s.budget.interrupt))),
                &p,
            ),
            self.auto_allow_rows(&s, cx),
            div()
                .pb_2()
                .pt_3()
                .text_xs()
                .text_color(hsla(p.subtle))
                .child("Values prefilled in \"Add agent\".")
                .into_any_element(),
        ];
        for (kind, args) in &self.agents.args {
            let kind = *kind;
            let worktree = s.agent(kind).worktree;
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_3()
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .w(ui(144.0))
                            .items_center()
                            .gap(ui(10.0))
                            // shortcut: color dot instead of the panes module's AgentLogo.
                            .child(div().size(ui(10.0)).rounded_full().bg(hsla(p.agent(kind))))
                            .child(div().truncate().text_color(hsla(p.fg)).child(agent_name(kind))),
                    )
                    .child(field(args, None, true))
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(hsla(p.muted))
                            .child(toggle(("set-worktree", kind as usize), worktree, &p).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.set(cx, |s| {
                                        let mut defaults = s.agent(kind);
                                        defaults.worktree = !worktree;
                                        s.agents.insert(kind, defaults);
                                    })
                                },
                            )))
                            .child("Worktree"),
                    )
                    .into_any_element(),
            );
        }
        section("Agents", rows, &p)
    }
}
