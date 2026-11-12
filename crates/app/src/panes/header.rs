//! Pane header (PaneHeader.tsx + PaneMenu.tsx): agent logo, editable title, last action, chips from
//! other modules, task timer, done count, reviewer, and the action icons (Git, Interrupt, Skills,
//! Notes, Browser, More, Maximize, Close). Actions stay hidden until the pane is hovered or active.
//! Mouse down focuses the pane, double click toggles maximize.

use agentdeck_core::agents::AgentKind;
use agentdeck_core::hooks_server::{PaneStatus, Subagent};
use agentdeck_core::logic::deck::{FONT_SIZE_MAX, FONT_SIZE_MIN, Pane, folder_name};
use agentdeck_core::logic::text::{agent_name, format_elapsed, now_ms, short_action};
use agentdeck_core::logic::theme::{FONT_MONO, Palette, RADIUS_LG, RADIUS_MD, RADIUS_XL, parse_hex};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::logo::agent_logo;
use super::picker::spawn_agent;
use super::runtime::{self, is_busy};
use super::widgets::icon_button;
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

/// Tags to tell same-agent panes apart at a glance ("" = none).
pub const PANE_COLORS: [&str; 7] = ["", "#ef4444", "#f59e0b", "#22c55e", "#3b82f6", "#a855f7", "#ec4899"];

/// What the header asks its host (layout and the feature modules) to do.
#[derive(Debug, Clone, PartialEq)]
pub enum PaneHeaderEvent {
    ToggleMaximize,
    /// Detach to a window, or reattach when detached.
    ToggleDetach,
    OpenGit,
    OpenSkills,
    OpenNotes,
    /// Reveal the pane's browser panel.
    ShowBrowser,
    /// Right click on the browser button: close the browser panel.
    CloseBrowser,
}

/// Title shown for a pane: older panes were named after their folder, show the agent instead.
pub fn display_title(pane: &Pane) -> String {
    if pane.title.is_empty() || pane.title == folder_name(&pane.cwd) {
        agent_name(pane.agent).to_owned()
    } else {
        pane.title.clone()
    }
}

/// Native yes/no prompt; runs `then` on the first answer.
fn confirm(message: String, ok: &'static str, window: &mut Window, cx: &mut App, then: impl FnOnce(&mut App) + 'static) {
    let answer = window.prompt(PromptLevel::Warning, &message, None, &[ok, "Cancel"], cx);
    cx.spawn(async move |cx| {
        if answer.await.ok() == Some(0) {
            cx.update(then);
        }
    })
    .detach();
}

/// Closes the pane, asking first if the agent is still working.
pub fn confirm_close(pane_id: &str, window: &mut Window, cx: &mut App) {
    let id = pane_id.to_owned();
    let Some(agent) = runtime_pane_agent(&id, cx) else { return };
    if !is_busy(&id, now_ms(), cx) {
        runtime::close_pane(&id, cx);
        return;
    }
    let message = format!("{} is still working. Close anyway?", agent_name(agent));
    confirm(message, "Close", window, cx, move |cx| runtime::close_pane(&id, cx));
}

fn confirm_restart(pane_id: &str, window: &mut Window, cx: &mut App) {
    let id = pane_id.to_owned();
    let Some(agent) = runtime_pane_agent(&id, cx) else { return };
    if !is_busy(&id, now_ms(), cx) {
        runtime::restart_pane(&id, cx);
        return;
    }
    let message = format!("{} is still working. Restart anyway?", agent_name(agent));
    confirm(message, "Restart", window, cx, move |cx| runtime::restart_pane(&id, cx));
}

fn runtime_pane_agent(id: &str, cx: &App) -> Option<AgentKind> {
    super::runtime::deck(cx)?.read(cx).state.panes.get(id).map(|p| p.agent)
}

/// Status of an agent as a quiet pill in its header: blue "Working", green "Done", amber "Waiting",
/// red "Error". Nothing for an idle agent or a plain shell.
fn status_pill(pane: &Pane, p: &Palette) -> Option<Div> {
    if pane.agent == AgentKind::Shell || pane.exited || pane.status == PaneStatus::Idle {
        return None;
    }
    let color = super::status_color(pane.status, p);
    Some(
        div()
            .flex_shrink_0()
            .rounded_full()
            .px(ui(7.0))
            .py(ui(1.0))
            .text_size(ui(11.0))
            .font_weight(FontWeight::MEDIUM)
            .bg(hsla_a(color, 0.15))
            .text_color(hsla(color))
            // "Working", "Waiting"...: the first word of the status label.
            .child(super::status_label(pane.status).split(' ').next().unwrap_or_default()),
    )
}

/// "N agents" chip: the Claude subagents running inside the pane, each one detailed in the tooltip.
fn subagents_chip(subagents: Vec<Subagent>, now: i64, p: &Palette) -> Option<Stateful<Div>> {
    let count = subagents.len();
    if count == 0 {
        return None;
    }
    let rows: Vec<(String, String)> = subagents
        .into_iter()
        .map(|sub| {
            let kind = if sub.label == sub.agent_type { String::new() } else { format!(" · {}", sub.agent_type) };
            let title = format!("{}{kind} · {}", sub.label, format_elapsed(now - sub.started_at));
            (title, sub.last_action.unwrap_or_else(|| "Starting…".to_owned()))
        })
        .collect();
    Some(
        div()
            .id("subagents")
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .text_xs()
            .text_color(hsla(p.muted))
            .tooltip(move |window, cx| {
                let rows = rows.clone();
                Tooltip::element(move |_, _| {
                    div().flex().flex_col().gap_1().children(rows.iter().map(|(title, action)| {
                        div()
                            .flex()
                            .flex_col()
                            .child(div().font_weight(FontWeight::MEDIUM).child(title.clone()))
                            .child(div().opacity(0.7).child(action.clone()))
                    }))
                })
                .build(window, cx)
            })
            .child(Icon::new(IconName::Bot).size(ui(12.0)))
            .child(format!("{count} agent{}", if count == 1 { "" } else { "s" })),
    )
}

#[derive(Clone, Copy)]
enum MenuAction {
    Rename,
    Duplicate,
    Restart,
    ZoomIn,
    ZoomOut,
    Detach,
    OpenPort,
    Sleep,
}

pub struct PaneHeader {
    deck: Entity<Deck>,
    pub pane_id: String,
    pub maximized: bool,
    /// Several panes stacked in one group: show only identity, status and close.
    pub compact: bool,
    pub detached: bool,
    /// The Git panel of this pane is open (the Git button shows pressed).
    pub git_open: bool,
    /// Chips other modules place after the last action (diff chip, usage chip, queue button).
    pub extras: Vec<AnyView>,
    rename: Option<Entity<InputState>>,
    _rename_sub: Option<Subscription>,
    menu_open: bool,
    _observe: Subscription,
}

impl EventEmitter<PaneHeaderEvent> for PaneHeader {}

impl PaneHeader {
    pub fn new(deck: Entity<Deck>, pane_id: impl Into<String>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&deck, |_, _, cx| cx.notify());
        Self {
            deck,
            pane_id: pane_id.into(),
            maximized: false,
            compact: false,
            detached: false,
            git_open: false,
            extras: Vec::new(),
            rename: None,
            _rename_sub: None,
            menu_open: false,
            _observe: observe,
        }
    }

    /// Opens the title field (menu "Rename", or F2 from layout).
    pub fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.deck.read(cx).state.panes.get(&self.pane_id).map(|p| p.title.clone()) else {
            return;
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title));
        input.update(cx, |state, cx| state.focus(window, cx));
        let sub = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.finish_rename(true, cx);
            }
        });
        self.rename = Some(input);
        self._rename_sub = Some(sub);
        self.menu_open = false;
        cx.notify();
    }

    fn finish_rename(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(input) = self.rename.take() else { return };
        self._rename_sub = None;
        if commit {
            let value = input.read(cx).value().trim().to_owned();
            let id = self.pane_id.clone();
            self.deck.update(cx, |d, cx| {
                if !value.is_empty() {
                    d.state.rename(&id, &value);
                }
                cx.notify();
            });
        }
        cx.notify();
    }

    fn run(&mut self, action: MenuAction, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_open = false;
        cx.notify();
        let id = self.pane_id.clone();
        let Some(pane) = self.deck.read(cx).state.panes.get(&id).cloned() else { return };
        match action {
            MenuAction::Rename => self.start_rename(window, cx),
            MenuAction::Duplicate => spawn_agent(pane.agent, &pane.cwd, pane.spec.args.clone(), pane.worktree, pane.ssh.clone(), cx),
            MenuAction::Restart => confirm_restart(&id, window, cx),
            MenuAction::ZoomIn | MenuAction::ZoomOut => {
                let delta = if matches!(action, MenuAction::ZoomIn) { 1.0 } else { -1.0 };
                self.deck.update(cx, |d, cx| {
                    d.state.zoom(&id, Some(delta));
                    cx.notify();
                });
            }
            MenuAction::Detach => cx.emit(PaneHeaderEvent::ToggleDetach),
            MenuAction::Sleep => runtime::sleep_pane(&id, cx),
            MenuAction::OpenPort => {
                let Some(port) = pane.port else { return };
                let url = format!("http://localhost:{port}");
                // In the agent's browser panel when it has one, else in the system browser.
                if pane.browser && crate::browser::go(&id, &url, cx).is_ok() {
                    cx.emit(PaneHeaderEvent::ShowBrowser);
                } else {
                    cx.open_url(&url);
                }
            }
        }
    }

    fn set_color(&mut self, color: &str, cx: &mut Context<Self>) {
        self.menu_open = false;
        let id = self.pane_id.clone();
        let color = (!color.is_empty()).then(|| color.to_owned());
        self.deck.update(cx, |d, cx| {
            d.state.update_pane(&id, |p| p.spec.color = color);
            cx.notify();
        });
    }

    /// Off: turns the browser on (the agent has its MCP server since its spawn, so it keeps running). On: reveals it.
    fn browser_button(&mut self, pane: &Pane, cx: &mut Context<Self>) {
        if pane.browser {
            cx.emit(PaneHeaderEvent::ShowBrowser);
            return;
        }
        self.deck.update(cx, |d, cx| {
            d.state.update_pane(&pane.id, |p| p.spec.browser = true);
            cx.notify();
        });
        runtime::show_browser(&pane.id, cx);
    }

    fn title(&self, pane: &Pane, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        if let Some(input) = &self.rename {
            return div()
                .w(ui(160.0))
                .min_w_0()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.finish_rename(false, cx);
                    }
                }))
                .child(Input::new(input).h(ui(24.0)).aria_label("Task title"))
                .into_any_element();
        }
        let tip: SharedString = format!("{} · {}", agent_name(pane.agent), pane.cwd).into();
        div()
            .id("pane-title")
            .flex_shrink_0()
            .font_weight(FontWeight::MEDIUM)
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .child(display_title(pane))
            .text_color(hsla(p.fg))
            .into_any_element()
    }

    fn menu(&self, pane: &Pane, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let mut items: Vec<(IconName, SharedString, MenuAction, bool)> = [
            (IconName::Pencil, "Rename", MenuAction::Rename, false),
            (IconName::Copy, "Duplicate agent", MenuAction::Duplicate, false),
            (IconName::RotateCcw, "Restart in a new conversation", MenuAction::Restart, false),
            (IconName::ZoomIn, "Zoom in (Ctrl+=)", MenuAction::ZoomIn, pane.font_size >= FONT_SIZE_MAX),
            (IconName::ZoomOut, "Zoom out (Ctrl+-)", MenuAction::ZoomOut, pane.font_size <= FONT_SIZE_MIN),
            if self.detached {
                (IconName::SquareArrowDownLeft, "Reattach", MenuAction::Detach, false)
            } else {
                (IconName::ExternalLink, "Detach to a window", MenuAction::Detach, false)
            },
        ]
        .map(|(icon, label, action, disabled)| (icon, label.into(), action, disabled))
        .into();
        // Frees the agent's memory; it wakes on the same conversation. Needs a conversation to resume.
        if pane.agent == AgentKind::Claude && !pane.exited {
            let label = "Sleep: free its memory until you wake it";
            items.push((IconName::Clock, label.into(), MenuAction::Sleep, pane.resume_id.is_none()));
        }
        // The agent's own PORT (Settings > Agents > Port per agent): its dev server, one click away.
        if let Some(port) = pane.port.filter(|_| self.deck.read(cx).settings.base_port != 0) {
            items.push((IconName::Globe, format!("Open localhost:{port}").into(), MenuAction::OpenPort, false));
        }
        let rows = items.into_iter().enumerate().map(|(i, (icon, label, action, disabled))| {
            let row = div()
                .id(("pane-menu-item", i))
                .flex()
                .items_center()
                .gap(ui(10.0))
                .rounded(ui(RADIUS_MD - 2.0))
                .px(ui(10.0))
                .py(ui(6.0))
                .text_color(hsla(p.muted))
                .child(Icon::new(icon).size(ui(14.0)))
                .child(label);
            if disabled {
                row.opacity(0.4)
            } else {
                row.cursor_pointer()
                    .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                    .on_click(cx.listener(move |this, _, window, cx| this.run(action, window, cx)))
            }
        });
        let current = pane.color.clone().unwrap_or_default();
        let swatches = PANE_COLORS.iter().enumerate().map(|(i, color)| {
            let pressed = current == *color;
            let tip: SharedString = if color.is_empty() {
                "No color".into()
            } else {
                format!("Tag with {color}").into()
            };
            div()
                .id(("pane-color", i))
                .size(ui(16.0))
                .rounded_full()
                .border_1()
                .border_color(hsla(if pressed { p.fg } else { p.line_strong }))
                .when(pressed, |s| s.border_2())
                .when_some(parse_hex(color), |s, hex| s.bg(hsla(hex)))
                .cursor_pointer()
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .on_click(cx.listener(move |this, _, _, cx| this.set_color(color, cx)))
        });
        div()
            .absolute()
            .top_full()
            .right_0()
            .child(deferred(
                anchored().anchor(Anchor::TopRight).snap_to_window().child(
                    div()
                        .id("pane-menu")
                        .occlude()
                        .mt_1()
                        .flex()
                        .flex_col()
                        .min_w(ui(224.0))
                        .rounded(ui(RADIUS_LG - 4.0))
                        .border_1()
                        .border_color(hsla(p.line_strong))
                        .bg(hsla(p.raised))
                        .p_1()
                        .text_size(ui(13.0))
                        .shadow_lg()
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.menu_open = false;
                            cx.notify();
                        }))
                        .children(rows)
                        .child(
                            div()
                                .mt_1()
                                .flex()
                                .items_center()
                                .gap(ui(6.0))
                                .border_t_1()
                                .border_color(hsla(p.line))
                                .px(ui(10.0))
                                .pt_2()
                                .pb(ui(6.0))
                                .child(div().mr_auto().text_xs().text_color(hsla(p.subtle)).child("Color"))
                                .children(swatches),
                        ),
                ),
            ))
            .into_any_element()
    }
}

impl Render for PaneHeader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let deck = self.deck.read(cx);
        let Some(pane) = deck.state.panes.get(&self.pane_id).cloned() else {
            // Fallback while the pane is gone from the deck (closing).
            return div()
                .id("pane-header-empty")
                .flex()
                .h_full()
                .items_center()
                .gap_2()
                .px_2()
                .text_color(hsla(p.subtle))
                .child(Icon::new(IconName::SquareTerminal).size(ui(15.0)))
                .child("Agent");
        };
        let active = deck.state.active_pane_id.as_deref() == Some(pane.id.as_str());
        let reviewer = deck
            .reviews
            .get(&pane.id)
            .and_then(|r| deck.state.panes.get(r))
            .map(|r| r.title.clone());
        let voice_target = crate::voice::voice_target(cx).as_deref() == Some(pane.id.as_str());
        let now = now_ms();
        let group: SharedString = format!("pane-header-{}", pane.id).into();
        let weak = cx.entity().downgrade();
        let emit = move |event: PaneHeaderEvent| {
            let weak = weak.clone();
            move |_: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |_, cx| cx.emit(event.clone()));
            }
        };
        let id = pane.id.clone();
        let focus_id = pane.id.clone();

        let mut header = div()
            .id(SharedString::from(format!("pane-header-{}", pane.id)))
            .group(group.clone())
            .relative()
            .flex()
            .size_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .px(ui(10.0))
            .text_size(ui(13.0))
            // GPUI does not clip children to the pane's rounded corners: round the fill itself.
            .rounded_t(ui(RADIUS_XL - 1.0))
            .bg(hsla(if active { p.panel } else { p.base }))
            .text_color(hsla(if active { p.fg } else { p.muted }))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| runtime::focus_pane(&focus_id, cx))
            .on_click(cx.listener(|_, event: &ClickEvent, _, cx| {
                if event.click_count() == 2 {
                    cx.emit(PaneHeaderEvent::ToggleMaximize);
                }
            }))
            .child(agent_logo(pane.agent, 14.0, None, &p))
            .children(crate::orchestrator::orchestrator_badge(&self.deck, &pane, cx))
            .child(self.title(&pane, cx))
            .children(status_pill(&pane, &p))
            // Subject of the conversation, after the agent's name and status.
            // Capped at half the header (unless compact) so a long subject cannot push every chip out.
            .children(pane.topic.clone().map(|topic| {
                div()
                    .min_w_0()
                    .when(!self.compact, |d| d.max_w(relative(0.5)))
                    .truncate()
                    .text_color(hsla(p.fg))
                    .child(topic)
            }))
            .when(voice_target, |h| {
                h.child(
                    div()
                        .id("voice-target")
                        .tooltip(|window, cx| Tooltip::new("Next dictation goes here").build(window, cx))
                        .child(Icon::new(IconName::Mic).size(ui(12.0)).text_color(hsla(p.subtle))),
                )
            });

        if !self.compact {
            let full_action: Option<SharedString> = pane.last_action.clone().map(Into::into);
            // Secondary details: they take what the name, status and subject leave (zero basis) and
            // clip from the right, so a narrow pane drops them first.
            let details = div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_2()
                .overflow_hidden()
                .child(
                    div()
                        .id("last-action")
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(hsla(p.subtle))
                        .when_some(full_action, |el, full| {
                            el.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
                        })
                        .child(short_action(pane.last_action.as_deref())),
                )
                .children(subagents_chip(runtime::pane_subagents(&pane.id, cx), now, &p))
                .children(self.extras.iter().cloned())
                .child(crate::palette::queue_button(&self.deck, &pane, active, cx))
                .when_some(pane.task_started_at, |h, since| {
                    h.child(
                        div()
                            .id("elapsed")
                            .flex_shrink_0()
                            .font_family(FONT_MONO)
                            .text_xs()
                            .text_color(hsla(p.subtle))
                            .tooltip(|window, cx| Tooltip::new("Time spent on the task").build(window, cx))
                            .child(format_elapsed(now - since)),
                    )
                })
                .when(pane.completed_count > 0, |h| {
                    let tip: SharedString = format!("{} task(s) done", pane.completed_count).into();
                    h.child(
                        div()
                            .id("done-count")
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .text_color(hsla(p.subtle))
                            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                            .child(Icon::new(IconName::CheckCheck).size(ui(12.0)))
                            .child(pane.completed_count.to_string()),
                    )
                })
                .when_some(reviewer, |h, reviewer| {
                    let tip: SharedString = format!("Reviewed by {reviewer}").into();
                    h.child(
                        div()
                            .id("reviewer")
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .text_color(hsla(p.subtle))
                            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                            .child(Icon::new(IconName::ScanEye).size(ui(12.0)))
                            .child(reviewer),
                    )
                });
            header = header.child(details).child(self.toolbar(&pane, active, group, &emit, cx));
        }

        let close_id = id.clone();
        header
            .child(
                div().when(self.compact, |d| d.ml_auto()).child(
                    icon_button("close", IconName::X, "Close")
                        .danger()
                        .on_click(move |window, cx| confirm_close(&close_id, window, cx)),
                ),
            )
            // Color tag: a stripe on the left (waiting = the pane frame's amber border).
            .when_some(pane.color.as_deref().and_then(parse_hex), |h, hex| {
                // Left border of a box rounded like the pane: the stripe follows the pane's corner.
                let mut tag = div().absolute().inset_0().rounded_tl(ui(RADIUS_XL - 1.0)).border_color(hsla(hex));
                tag.style().border_widths.left = Some(ui(3.0).into());
                h.child(tag)
            })
    }
}

impl PaneHeader {
    fn toolbar<F, H>(&self, pane: &Pane, active: bool, group: SharedString, emit: &F, cx: &mut Context<Self>) -> Div
    where
        F: Fn(PaneHeaderEvent) -> H,
        H: Fn(&mut Window, &mut App) + 'static,
    {
        let interrupt_id = pane.id.clone();
        let weak = cx.entity().downgrade();
        let browser_pane = pane.clone();
        let browser_weak = weak.clone();
        let menu_weak = weak.clone();
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .opacity(if active { 1.0 } else { 0.0 })
            .group_hover(group, |s| s.opacity(1.0))
            .child(
                icon_button("git", IconName::GitBranch, "Git: changes, commit, merge, restore points")
                    .pressed(self.git_open)
                    .on_click(emit(PaneHeaderEvent::OpenGit)),
            )
            .child(
                icon_button(
                    "interrupt",
                    IconName::Pause,
                    "Interrupt the current task (Esc, or Ctrl+C for a shell). The agent stays open.",
                )
                .disabled(pane.exited)
                .on_click(move |_, cx| runtime::interrupt_pane(&interrupt_id, cx)),
            )
            .when(pane.agent != AgentKind::Shell, |t| {
                t.child(
                    icon_button("skills", IconName::Puzzle, "Skills: see, edit, install (GitHub link), uninstall")
                        .on_click(emit(PaneHeaderEvent::OpenSkills)),
                )
                .child(
                    icon_button("notes", IconName::NotebookPen, "Notes: the agent's instructions and memory files")
                        .on_click(emit(PaneHeaderEvent::OpenNotes)),
                )
            })
            .when(crate::browser::supported(pane.agent), |t| {
                let hint = if pane.browser {
                    "Show this agent's browser (right-click to close it)"
                } else {
                    "Give this agent a browser it can drive, in a panel next to it"
                };
                let mut button = icon_button("browser", IconName::Globe, hint)
                    .pressed(pane.browser)
                    .on_click(move |_, cx| {
                        let pane = browser_pane.clone();
                        let _ = browser_weak.update(cx, |this, cx| this.browser_button(&pane, cx));
                    });
                if pane.browser {
                    button = button.on_right_click(emit(PaneHeaderEvent::CloseBrowser));
                }
                t.child(button)
            })
            .child(
                div()
                    .relative()
                    .child(
                        icon_button("more", IconName::Ellipsis, "More actions")
                            .pressed(self.menu_open)
                            .on_click(move |_, cx| {
                                let _ = menu_weak.update(cx, |this, cx| {
                                    this.menu_open = !this.menu_open;
                                    cx.notify();
                                });
                            }),
                    )
                    .when(self.menu_open, |d| d.child(self.menu(pane, cx))),
            )
            .child({
                let (icon, hint) = if self.maximized {
                    (IconName::Minimize2, "Restore (Esc)")
                } else {
                    (IconName::Maximize2, "Maximize (double-click the header)")
                };
                icon_button("maximize", icon, hint)
                    .pressed(self.maximized)
                    .on_click(emit(PaneHeaderEvent::ToggleMaximize))
            })
    }
}
