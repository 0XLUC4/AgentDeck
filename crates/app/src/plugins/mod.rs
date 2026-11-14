//! Plugins page (rail button, and the Skills button of agent panes): a Plugins tab with AgentDeck's
//! catalog of bundled skills, turned on or off for every agent at once, plus skills installed from a
//! GitHub link; then one tab per installed agent with its own skills. Logic: agentdeck_core::plugins.
//!
//! The agents' skill folders are brought in line with the settings at start and after each toggle,
//! off the UI thread, one run at a time.

use std::sync::atomic::{AtomicBool, Ordering};

use agentdeck_core::agents::{AgentKind, detect_agents};
use agentdeck_core::logic::text::agent_name;
use agentdeck_core::logic::theme::{FONT_MONO, FONT_SANS, Palette};
use agentdeck_core::plugins::{self, CATALOG, Plugin};
use gpui_kit::assets::IconName;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::layout::DeckView;
use crate::panes::icon_button;
use crate::skills::{AgentDialogKind, AgentFilesDialog};
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

actions!(agentdeck, [ClosePlugins]);

/// Key context of the page (Esc closes).
const CONTEXT: &str = "Plugins";

/// Last sync's notes and whether one is running.
#[derive(Default)]
struct SyncState {
    notes: Vec<String>,
    running: bool,
    /// Settings changed during a run: run once more after it.
    again: bool,
}

impl Global for SyncState {}

/// Registers the key binding and syncs the plugins once the main deck exists.
pub fn init(cx: &mut App) {
    cx.set_global(SyncState::default());
    cx.bind_keys([KeyBinding::new("escape", ClosePlugins, Some(CONTEXT))]);
    static STARTED: AtomicBool = AtomicBool::new(false);
    cx.observe_new(|view: &mut DeckView, _, cx| {
        if !STARTED.swap(true, Ordering::SeqCst) {
            sync(view.deck().clone(), cx);
        }
    })
    .detach();
}

/// Copies, updates or removes the bundled skills to match the settings (see `plugins::sync`).
fn sync(deck: Entity<Deck>, cx: &mut App) {
    let state = cx.global_mut::<SyncState>();
    if state.running {
        state.again = true;
        return;
    }
    state.running = true;
    let settings = deck.read(cx).settings.clone();
    cx.spawn(async move |cx| {
        let notes = cx.background_executor().spawn(async move { plugins::sync(&|p| settings.plugin_on(p)) }).await;
        cx.update(|cx| {
            let state = cx.global_mut::<SyncState>();
            state.notes = notes;
            state.running = false;
            if std::mem::take(&mut state.again) {
                sync(deck, cx);
            }
        });
    })
    .detach();
}

/// Opens the page on the Plugins tab; agent tabs read the active pane's folder.
pub fn open(deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    let state = &deck.read(cx).state;
    let cwd = state.active_pane_id.as_ref().and_then(|id| state.panes.get(id)).map(|pane| pane.spec.cwd.clone());
    show(deck, Tab::Plugins, cwd, window, cx);
}

/// Opens the page on the skills tab of an agent pane's agent.
pub fn open_skills(deck: Entity<Deck>, pane_id: &str, window: &mut Window, cx: &mut App) {
    let Some(spec) = deck.read(cx).state.panes.get(pane_id).map(|pane| pane.spec.clone()) else {
        return;
    };
    show(deck, Tab::Agent(spec.agent), Some(spec.cwd), window, cx);
}

fn show(deck: Entity<Deck>, tab: Tab, cwd: Option<String>, window: &mut Window, cx: &mut App) {
    let cwd = cwd.unwrap_or_else(|| std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default());
    let view = cx.new(|cx| PluginsView::new(deck.clone(), tab, cwd, window, cx));
    let focus = view.read(cx).focus.clone();
    crate::skills::show_page(&deck, "plugins", view.into(), focus, window, cx);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Plugins,
    Agent(AgentKind),
}

pub struct PluginsView {
    deck: Entity<Deck>,
    focus: FocusHandle,
    tab: Tab,
    /// Folder whose project skills the agent tabs show.
    cwd: String,
    /// Skills tab of each installed agent, in detection order.
    agents: Vec<(AgentKind, Entity<AgentFilesDialog>)>,
    link: Entity<InputState>,
    /// The GitHub install popup is open.
    popup: bool,
    installing: bool,
    _subscriptions: [Subscription; 3],
}

impl PluginsView {
    fn new(deck: Entity<Deck>, tab: Tab, cwd: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let link = cx.new(|cx| InputState::new(window, cx).placeholder("https://github.com/owner/repo/tree/main/skills/name"));
        let link_events = cx.subscribe_in(&link, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { .. } => this.install(window, cx),
            _ => cx.notify(),
        });
        let mut this = Self {
            _subscriptions: [
                cx.observe_global::<SyncState>(|_, cx| cx.notify()),
                cx.observe(&deck, |_, _, cx| cx.notify()),
                link_events,
            ],
            deck,
            focus: cx.focus_handle(),
            tab,
            cwd,
            agents: Vec::new(),
            link,
            popup: false,
            installing: false,
        };
        // The agent asked for gets its tab now; the other installed agents once detected.
        if let Tab::Agent(agent) = tab {
            this.add_agent(agent, window, cx);
        }
        cx.spawn_in(window, async move |this, cx| {
            let found = cx.background_spawn(async { detect_agents() }).await;
            this.update_in(cx, |this, window, cx| {
                for info in found.into_iter().filter(|info| info.installed && info.kind != AgentKind::Shell) {
                    this.add_agent(info.kind, window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        this
    }

    fn add_agent(&mut self, agent: AgentKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.agents.iter().any(|(kind, _)| *kind == agent) {
            return;
        }
        let cwd = self.cwd.clone();
        let view = cx.new(|cx| AgentFilesDialog::new(agent, cwd, AgentDialogKind::Skills, true, window, cx));
        self.agents.push((agent, view));
    }

    fn agent_view(&self) -> Option<Entity<AgentFilesDialog>> {
        let Tab::Agent(agent) = self.tab else { return None };
        self.agents.iter().find(|(kind, _)| *kind == agent).map(|(_, view)| view.clone())
    }

    /// Esc and the close button: a popup closes first; an unsaved skill edit asks before going.
    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.popup) {
            cx.notify();
            return;
        }
        if let Some(view) = self.agent_view() {
            if view.update(cx, |view, cx| view.close_popup(cx)) {
                return;
            }
            if view.read(cx).has_unsaved(cx) {
                cx.spawn_in(window, async move |_, cx| {
                    if crate::skills::confirm("Discard your changes?".into(), "Discard", cx).await {
                        let _ = cx.update(|window, cx| crate::skills::close(window, cx));
                    }
                })
                .detach();
                return;
            }
        }
        crate::skills::close(window, cx);
    }

    fn toggle_plugin(&mut self, plugin: &'static Plugin, cx: &mut Context<Self>) {
        self.deck.update(cx, |d, cx| {
            d.update_settings(
                |s| {
                    let on = s.plugin_on(plugin);
                    s.plugins.insert(plugin.id.to_owned(), !on);
                },
                cx,
            )
        });
        sync(self.deck.clone(), cx);
    }

    fn open_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.popup = true;
        self.link.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.link.read(cx).value().trim().to_owned();
        if url.is_empty() || self.installing {
            return;
        }
        self.installing = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { plugins::install_everywhere(&url) }).await;
            this.update_in(cx, |this, window, cx| {
                this.installing = false;
                let message = match result {
                    Ok(names) => {
                        this.link.update(cx, |s, cx| s.set_value("", window, cx));
                        this.popup = false;
                        format!("Installed for every agent: {}", names.join(", "))
                    }
                    Err(e) => format!("Not installed: {e}"),
                };
                this.deck.update(cx, |d, cx| d.toast(message, cx));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn card(&self, plugin: &'static Plugin, on: bool, p: &Palette, cx: &mut Context<Self>) -> Div {
        let skills = plugin.skills.iter().map(|s| {
            div()
                .px_1p5()
                .py_0p5()
                .rounded(ui(4.0))
                .bg(hsla(p.hover))
                .font_family(FONT_MONO)
                .text_size(ui(11.0))
                .text_color(hsla(p.muted))
                .child(s.name)
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .w(ui(340.0))
            .p_4()
            .rounded(ui(10.0))
            .border_1()
            .border_color(hsla(if on { p.accent } else { p.line }))
            .bg(hsla(p.base))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(plugin.name))
                    .child(
                        crate::automations::toggle(SharedString::from(format!("plugin-{}", plugin.id)), on, p)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_plugin(plugin, cx))),
                    ),
            )
            .child(div().text_xs().text_color(hsla(p.muted)).whitespace_normal().child(plugin.description))
            .child(div().flex().flex_wrap().gap_1().children(skills))
    }

    fn github(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .max_w(ui(700.0))
            .child(section_title("From GitHub", p))
            .child(div().text_xs().text_color(hsla(p.subtle)).whitespace_normal().child(
                "Any repository or folder with SKILL.md files, installed for every agent found on this computer \
                 (Claude Code, Codex, Gemini). See or remove them in each agent's tab.",
            ))
            .child(
                div().flex().child(
                    Button::new("plugins-install-open")
                        .primary()
                        .icon(IconName::Download)
                        .label("Install from GitHub")
                        .on_click(cx.listener(|this, _, window, cx| this.open_popup(window, cx))),
                ),
            )
    }

    /// Popup of the GitHub install: link field, Cancel / Install.
    fn install_popup(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette(cx);
        let can_install = !self.installing && !self.link.read(cx).value().trim().is_empty();
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xs().text_color(hsla(p.subtle)).whitespace_normal().child(
                "Paste the link of a GitHub repository or folder with SKILL.md files. \
                 Installed for every agent on this computer.",
            ))
            .child(Input::new(&self.link))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("plugins-install-cancel").ghost().label("Cancel").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.popup = false;
                            cx.notify();
                        },
                    )))
                    .child(
                        Button::new("plugins-install")
                            .primary()
                            .icon(IconName::Download)
                            .label(if self.installing { "Installing…" } else { "Install" })
                            .disabled(!can_install)
                            .on_click(cx.listener(|this, _, window, cx| this.install(window, cx))),
                    ),
            );
        crate::skills::popup("plugins-install-popup", "Install from GitHub", body, cx)
    }

    /// Tab bar: Plugins, then one tab per installed agent.
    fn tabs(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let options = std::iter::once((Tab::Plugins, "Plugins"))
            .chain(self.agents.iter().map(|(agent, _)| (Tab::Agent(*agent), agent_name(*agent))));
        let buttons: Vec<Stateful<Div>> = options
            .enumerate()
            .map(|(i, (value, label))| {
                let pressed = self.tab == value;
                div()
                    .id(("plugins-tab", i))
                    .h(ui(28.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded(ui(6.0))
                    .text_size(ui(12.0))
                    .cursor_pointer()
                    .when(pressed, |d| d.bg(hsla(p.raised)).font_weight(FontWeight::MEDIUM).text_color(hsla(p.fg)))
                    .when(!pressed, |d| d.text_color(hsla(p.muted)).hover(|s| s.text_color(hsla(p.fg))))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.tab = value;
                        cx.notify();
                    }))
                    .child(label)
            })
            .collect();
        div().flex().child(div().flex().flex_wrap().gap_1().rounded(ui(8.0)).bg(hsla(p.base)).p_1().children(buttons))
    }

    fn plugins_body(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let settings = self.deck.read(cx).settings.clone();
        let notes = cx.global::<SyncState>().notes.clone();
        let mut body = div().flex().flex_col().gap_6().p_6();
        for category in categories() {
            let cards: Vec<Div> = CATALOG
                .iter()
                .filter(|plugin| plugin.category == category)
                .map(|plugin| self.card(plugin, settings.plugin_on(plugin), p, cx))
                .collect();
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(section_title(category, p))
                    .child(div().flex().flex_wrap().gap_3().children(cards)),
            );
        }
        let notes = (!notes.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .max_w(ui(700.0))
                .px_3()
                .py_2()
                .rounded(ui(6.0))
                .bg(hsla_a(p.wait, 0.1))
                .text_xs()
                .text_color(hsla(p.wait))
                .children(notes.into_iter().map(|note| div().whitespace_normal().child(note)))
        });
        body.children(notes).child(self.github(p, cx))
    }
}

fn section_title(text: &'static str, p: &Palette) -> Div {
    div().text_xs().font_weight(FontWeight::MEDIUM).text_color(hsla(p.muted)).child(text)
}

/// Catalog sections in first-seen order.
fn categories() -> Vec<&'static str> {
    let mut seen: Vec<&'static str> = Vec::new();
    for plugin in CATALOG {
        if !seen.contains(&plugin.category) {
            seen.push(plugin.category);
        }
    }
    seen
}

impl Render for PluginsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let running = cx.global::<SyncState>().running;
        let this = cx.entity().downgrade();
        let header = div()
            .flex()
            .flex_shrink_0()
            .flex_col()
            .gap_3()
            .border_b_1()
            .border_color(hsla(p.line))
            .px_5()
            .py_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(ui(15.0)).font_weight(FontWeight::MEDIUM).child("Plugins & skills"))
                            .child(div().text_xs().text_color(hsla(p.subtle)).child(if running {
                                "Updating the agents' skills…"
                            } else {
                                "Plugins: skills for every agent at once. Agent tabs: each agent's own skills."
                            })),
                    )
                    .child(icon_button("plugins-close", IconName::X, "Close (Esc)").large().on_click(move |window, cx| {
                        let _ = this.update(cx, |this, cx| this.request_close(window, cx));
                    })),
            )
            .child(self.tabs(&p, cx));
        let content = match self.agent_view() {
            Some(view) => div().id("plugins-body").flex_1().min_h_0().overflow_y_scroll().child(view),
            None => div().id("plugins-body").flex_1().min_h_0().overflow_y_scroll().child(self.plugins_body(&p, cx)),
        };
        let popup = self.popup.then(|| self.install_popup(cx));
        div()
            .id("plugins")
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &ClosePlugins, window, cx| this.request_close(window, cx)))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .occlude()
            .bg(hsla(p.panel))
            .text_color(hsla(p.fg))
            .text_size(ui(13.0))
            .font_family(FONT_SANS)
            .child(header)
            .child(content)
            .children(popup)
    }
}
