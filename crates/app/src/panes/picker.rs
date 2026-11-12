//! "Add agent" dialog (AgentPicker.tsx): agent cards with install state, working folder (with Browse),
//! arguments and the dedicated worktree option, prefilled from Settings > Agents. Also `spawn_agent`
//! and `last_agent` (the agent the quick "Add agent" button starts).

use std::collections::HashMap;

use agentdeck_core::agents::{AgentInfo, AgentKind, detect_agents};
use agentdeck_core::logic::deck::PaneSpec;
use agentdeck_core::logic::text::{AGENT_KINDS, agent_name, split_args};
use agentdeck_core::logic::theme::{FONT_MONO, RADIUS_LG, RADIUS_MD, RADIUS_XL};
use agentdeck_core::persist::{JsonStore, LOCAL_FILE};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::logo::agent_logo;
use super::runtime;
use super::widgets::icon_button;
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

/// local.json key (was localStorage "add-agent-last").
const LAST_AGENT_KEY: &str = "add-agent-last";

/// Agent the quick "Add agent" button starts: the last one added, Claude at first.
/// Read on every title bar render: loaded from local.json once, then kept here.
static LAST_AGENT: std::sync::Mutex<Option<AgentKind>> = std::sync::Mutex::new(None);

pub fn last_agent(cx: &App) -> AgentKind {
    let mut cached = LAST_AGENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(agent) = *cached {
        return agent;
    }
    let Some(deck) = runtime::deck(cx) else { return AgentKind::Claude };
    let agent = JsonStore::load(&deck.read(cx).data_dir, LOCAL_FILE)
        .ok()
        .and_then(|store| serde_json::from_value(store.get(LAST_AGENT_KEY)).ok())
        .unwrap_or(AgentKind::Claude);
    *cached = Some(agent);
    agent
}

fn remember_agent(agent: AgentKind, dir: &std::path::Path) {
    *LAST_AGENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(agent);
    let saved = JsonStore::load(dir, LOCAL_FILE).and_then(|mut store| {
        store.set(LAST_AGENT_KEY, serde_json::json!(agent));
        store.save()
    });
    // The quick button just keeps its previous choice.
    if let Err(e) = saved {
        eprintln!("last agent not saved: {e}");
    }
}

/// Starts an agent in the active session tab and remembers it as the quick "Add agent" choice.
pub fn spawn_agent(agent: AgentKind, cwd: &str, args: Vec<String>, worktree: bool, ssh: Option<String>, cx: &mut App) {
    spawn_agent_with(agent, cwd, args, worktree, ssh, false, cx);
}

/// [`spawn_agent`], in chat mode when `chat` (Claude and Codex).
pub fn spawn_agent_with(agent: AgentKind, cwd: &str, args: Vec<String>, worktree: bool, ssh: Option<String>, chat: bool, cx: &mut App) {
    let Some(deck) = runtime::deck(cx) else { return };
    let (tab, font_size, dir) = deck.update(cx, |d, cx| {
        d.state.last_cwd = cwd.to_owned();
        cx.notify();
        (d.state.active_tab_id.clone(), d.settings.font_size, d.data_dir.clone())
    });
    remember_agent(agent, &dir);
    let mut spec = PaneSpec::new(agent, agent_name(agent), cwd);
    spec.args = args;
    spec.worktree = worktree;
    spec.ssh = ssh;
    spec.chat = chat;
    spec.font_size = font_size;
    runtime::create_pane(spec, &tab, false, cx);
}

/// Modal "Add agent". Emits `DismissEvent` when closed (Esc, Cancel, X or after Launch).
/// Render it as an overlay of the window root (it fills its parent with a backdrop).
pub struct AgentPicker {
    deck: Entity<Deck>,
    /// None while detecting.
    infos: Option<HashMap<AgentKind, AgentInfo>>,
    agent: AgentKind,
    cwd: Entity<InputState>,
    args: Entity<InputState>,
    /// "user@host:/folder": run the agent on that machine.
    ssh: Entity<InputState>,
    worktree: bool,
    /// Chat view instead of the terminal (Claude and Codex).
    chat: bool,
    error: Option<String>,
    focus: FocusHandle,
    _detect: Task<()>,
}

impl EventEmitter<DismissEvent> for AgentPicker {}

impl Focusable for AgentPicker {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl AgentPicker {
    pub fn new(deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let d = deck.read(cx);
        let workspace_path = d.state.active_workspace().path.clone();
        let folder = if workspace_path.is_empty() { d.state.last_cwd.clone() } else { workspace_path };
        let defaults = d.settings.agent(AgentKind::Claude);
        let cwd = cx.new(|cx| InputState::new(window, cx).placeholder("C:\\path\\to\\project").default_value(folder));
        let args = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("--model sonnet --append \"text with spaces\"")
                .default_value(defaults.args)
        });
        let ssh = cx.new(|cx| InputState::new(window, cx).placeholder("user@host:/folder/on/that/machine"));
        let detect = cx.spawn(async move |this, cx| {
            let list = cx.background_spawn(async { detect_agents() }).await;
            let infos = list.into_iter().map(|info| (info.kind, info)).collect();
            let _ = this.update(cx, |this, cx| {
                this.infos = Some(infos);
                cx.notify();
            });
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            deck,
            infos: None,
            agent: AgentKind::Claude,
            cwd,
            args,
            ssh,
            worktree: defaults.worktree,
            chat: false,
            error: None,
            focus,
            _detect: detect,
        }
    }

    /// Unknown install state (detection pending) counts as installed; spawn errors are toasted.
    fn installed(&self, kind: AgentKind) -> bool {
        self.infos
            .as_ref()
            .and_then(|i| i.get(&kind))
            .is_none_or(|info| info.installed)
    }

    /// Each agent comes prefilled with its defaults from Settings > Agents.
    fn pick_agent(&mut self, kind: AgentKind, window: &mut Window, cx: &mut Context<Self>) {
        let defaults = self.deck.read(cx).settings.agent(kind);
        self.agent = kind;
        self.worktree = defaults.worktree;
        self.args.update(cx, |s, cx| s.set_value(defaults.args, window, cx));
        cx.notify();
    }

    fn chat_mode(&self) -> bool {
        self.chat && matches!(self.agent, AgentKind::Claude | AgentKind::Codex)
    }

    /// "Terminal | Chat" switch, for the agents that have a headless mode.
    fn mode_switch(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !matches!(self.agent, AgentKind::Claude | AgentKind::Codex) {
            return None;
        }
        let p = palette(cx);
        let option = |id: &'static str, text: &'static str, chat: bool, cx: &mut Context<Self>| {
            let selected = self.chat == chat;
            div()
                .id(id)
                .px_3()
                .py_1()
                .rounded(ui(RADIUS_MD))
                .cursor_pointer()
                .when(selected, |el| el.bg(hsla(p.accent)).text_color(hsla(p.on_accent)))
                .when(!selected, |el| el.text_color(hsla(p.muted)).hover(|s| s.bg(hsla(p.hover))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.chat = chat;
                    cx.notify();
                }))
                .child(text)
        };
        Some(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(label("Mode", &p))
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .p(ui(2.0))
                        .rounded(ui(RADIUS_MD))
                        .border_1()
                        .border_color(hsla(p.line))
                        .child(option("mode-terminal", "Terminal", false, cx))
                        .child(option("mode-chat", "Chat", true, cx)),
                ),
        )
    }

    fn pick_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self.cwd.read(cx).value().to_string();
        cx.spawn_in(window, async move |this, cx| {
            let mut dialog = rfd::AsyncFileDialog::new();
            if !start.is_empty() {
                dialog = dialog.set_directory(&start);
            }
            let Some(folder) = dialog.pick_folder().await else { return };
            let path = folder.path().to_string_lossy().into_owned();
            let _ = this.update_in(cx, |this, window, cx| {
                this.cwd.update(cx, |s, cx| s.set_value(path, window, cx));
            });
        })
        .detach();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let cwd = self.cwd.read(cx).value().trim().to_owned();
        let ssh = self.ssh.read(cx).value().trim().to_owned();
        if cwd.is_empty() {
            self.error = Some("Choose a working folder.".into());
        } else if !ssh.is_empty() && agentdeck_core::pty::ssh_target(&ssh).is_none() {
            self.error = Some("SSH: user@host:/folder, without spaces or quotes.".into());
        } else if self.chat_mode() && !ssh.is_empty() {
            self.error = Some("Chat mode runs the agent on this machine: clear SSH or pick Terminal.".into());
        } else if ssh.is_empty() && !self.installed(self.agent) {
            self.error = Some(format!("{} is not installed.", agent_name(self.agent)));
        } else {
            let args = split_args(&self.args.read(cx).value());
            let chat = self.chat_mode();
            spawn_agent_with(self.agent, &cwd, args, self.worktree, (!ssh.is_empty()).then_some(ssh), chat, cx);
            cx.emit(DismissEvent);
        }
        cx.notify();
    }

    fn agent_card(&self, kind: AgentKind, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette(cx);
        let ok = self.installed(kind);
        let selected = self.agent == kind;
        let known = self.infos.is_some();
        let install_url = self
            .infos
            .as_ref()
            .and_then(|i| i.get(&kind))
            .and_then(|info| info.install_url);
        let state = if !known {
            div().text_xs().text_color(hsla(p.subtle)).child("Detecting…")
        } else if ok {
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_xs()
                .text_color(hsla(p.ok))
                .child(Icon::new(IconName::Check).size(ui(12.0)))
                .child("Installed")
        } else {
            div().text_xs().text_color(hsla(p.subtle)).child("Not found")
        };
        div()
            .id(SharedString::from(format!("agent-{}", agent_name(kind))))
            .relative()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_3()
            .rounded(ui(RADIUS_LG))
            .border_1()
            .p_3()
            .border_color(hsla(if selected { p.line_strong } else { p.line }))
            .when(selected, |c| c.bg(hsla(p.raised)))
            .when(!selected, |c| c.hover(|s| s.bg(hsla_a(p.hover, 0.5))))
            .when(!ok, |c| c.opacity(0.6))
            .when(ok, |c| {
                c.cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| this.pick_agent(kind, window, cx)))
            })
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .size(ui(32.0))
                    .items_center()
                    .justify_center()
                    .child(agent_logo(kind, 22.0, None, &p)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .gap(ui(2.0))
                    .child(div().font_weight(FontWeight::MEDIUM).child(agent_name(kind)))
                    .child(state),
            )
            .when_some(install_url.filter(|_| !ok), |c, url| {
                c.child(
                    div()
                        .id("install")
                        .ml_auto()
                        .flex()
                        .items_center()
                        .gap_1()
                        .rounded(ui(RADIUS_MD - 4.0))
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(hsla(p.fg))
                        .cursor_pointer()
                        .hover(|s| s.bg(hsla(p.hover)))
                        .on_click(move |_, _, cx| cx.open_url(url))
                        .child(Icon::new(IconName::Download).size(ui(12.0)))
                        .child("Install"),
                )
            })
    }
}

fn label(text: &'static str, p: &agentdeck_core::logic::theme::Palette) -> Div {
    div().text_xs().font_weight(FontWeight::MEDIUM).text_color(hsla(p.muted)).child(text)
}

impl Render for AgentPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let workspace = self.deck.read(cx).state.active_workspace().name.clone();
        let rows: Vec<Div> = AGENT_KINDS
            .chunks(2)
            .map(|pair| {
                div()
                    .flex()
                    .gap_2()
                    .children(pair.iter().map(|kind| self.agent_card(*kind, cx)))
            })
            .collect();
        let field = |input: &Entity<InputState>| Input::new(input).h(ui(36.0));
        let checked = self.worktree;

        div()
            .id("agent-picker-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla_a(p.gutter, 0.7))
            .occlude()
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    cx.emit(DismissEvent);
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(ui(576.0))
                    .max_w(relative(0.92))
                    .rounded(ui(RADIUS_XL))
                    .border_1()
                    .border_color(hsla(p.line))
                    .bg(hsla(p.panel))
                    .text_color(hsla(p.fg))
                    .text_size(ui(13.0))
                    .shadow_lg()
                    // Header
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pt_4()
                            .pr_3()
                            .pl_5()
                            .child(
                                div()
                                    .child(div().text_size(ui(15.0)).font_weight(FontWeight::MEDIUM).child("Add agent"))
                                    .child(div().text_xs().text_color(hsla(p.subtle)).child(format!("In {workspace}"))),
                            )
                            .child(icon_button("picker-close", IconName::X, "Close").large().on_click({
                                let weak = cx.entity().downgrade();
                                move |_, cx| {
                                    let _ = weak.update(cx, |_, cx| cx.emit(DismissEvent));
                                }
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_5()
                            .p_5()
                            // Agent
                            .child(div().flex().flex_col().gap_2().child(label("Agent", &p)).children(rows))
                            .children(self.mode_switch(cx))
                            // Working folder
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(ui(6.0))
                                    .child(label("Working folder", &p))
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .child(div().flex_1().font_family(FONT_MONO).text_xs().child(field(&self.cwd)))
                                            .child(
                                                div()
                                                    .id("browse")
                                                    .flex()
                                                    .items_center()
                                                    .gap(ui(6.0))
                                                    .h(ui(36.0))
                                                    .px_3()
                                                    .rounded(ui(RADIUS_MD))
                                                    .border_1()
                                                    .border_color(hsla(p.line))
                                                    .text_color(hsla(p.muted))
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                                                    .on_click(cx.listener(|this, _, window, cx| this.pick_folder(window, cx)))
                                                    .child(Icon::new(IconName::FolderOpen).size(ui(15.0)))
                                                    .child("Browse"),
                                            ),
                                    ),
                            )
                            // Arguments
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(ui(6.0))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(ui(6.0))
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(hsla(p.muted))
                                            .child(Icon::new(IconName::SquareTerminal).size(ui(13.0)))
                                            .child("Arguments")
                                            .child(div().font_weight(FontWeight::NORMAL).text_color(hsla(p.subtle)).child("(optional)")),
                                    )
                                    .child(div().font_family(FONT_MONO).text_xs().child(field(&self.args))),
                            )
                            // SSH
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(ui(6.0))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(ui(6.0))
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(hsla(p.muted))
                                            .child(Icon::new(IconName::Globe).size(ui(13.0)))
                                            .child("Run over SSH")
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::NORMAL)
                                                    .text_color(hsla(p.subtle))
                                                    .child("(optional; no status hooks, Git panel or worktree there)"),
                                            ),
                                    )
                                    .child(div().font_family(FONT_MONO).text_xs().child(field(&self.ssh))),
                            )
                            // Worktree
                            .child(
                                div()
                                    .id("worktree")
                                    .flex()
                                    .items_start()
                                    .gap_3()
                                    .rounded(ui(RADIUS_LG))
                                    .border_1()
                                    .border_color(hsla(p.line))
                                    .p_3()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(hsla_a(p.hover, 0.5)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.worktree = !this.worktree;
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .mt(ui(2.0))
                                            .flex()
                                            .flex_shrink_0()
                                            .items_center()
                                            .justify_center()
                                            .size(ui(16.0))
                                            .rounded(ui(4.0))
                                            .border_1()
                                            .border_color(hsla(if checked { p.accent } else { p.line_strong }))
                                            .when(checked, |b| {
                                                b.bg(hsla(p.accent)).child(
                                                    Icon::new(IconName::Check).size(ui(12.0)).text_color(hsla(p.on_accent)),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(ui(2.0))
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap(ui(6.0))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(Icon::new(IconName::GitBranch).size(ui(14.0)))
                                                    .child("Dedicated Git worktree"),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(hsla(p.subtle))
                                                    .child("The agent works on its own copy without getting in the way of others."),
                                            ),
                                    ),
                            )
                            .when_some(self.error.clone(), |body, error| {
                                body.child(
                                    div()
                                        .rounded(ui(RADIUS_MD))
                                        .bg(hsla_a(p.bad, 0.1))
                                        .px_3()
                                        .py_2()
                                        .text_xs()
                                        .text_color(hsla(p.bad))
                                        .child(error),
                                )
                            }),
                    )
                    // Footer
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .px_5()
                            .pb_5()
                            .child(
                                div()
                                    .id("picker-cancel")
                                    .flex()
                                    .items_center()
                                    .h_8()
                                    .px_3()
                                    .rounded(ui(RADIUS_MD))
                                    .text_color(hsla(p.muted))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
                                    .child("Cancel"),
                            )
                            .child(
                                div()
                                    .id("picker-launch")
                                    .flex()
                                    .items_center()
                                    .gap(ui(6.0))
                                    .h_8()
                                    .px(ui(14.0))
                                    .rounded(ui(RADIUS_MD))
                                    .bg(hsla(p.accent))
                                    .text_color(hsla(p.on_accent))
                                    .font_weight(FontWeight::MEDIUM)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(hsla(p.accent_hover)))
                                    .on_click(cx.listener(|this, _, _, cx| this.submit(cx)))
                                    .child(Icon::new(IconName::Play).size(ui(14.0)))
                                    .child("Launch"),
                            ),
                    ),
            )
    }
}
