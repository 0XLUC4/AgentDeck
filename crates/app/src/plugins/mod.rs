//! Plugins page (rail button): AgentDeck's catalog of bundled skills, turned on or off for every agent
//! at once, and skills installed from a GitHub link. Logic: agentdeck_core::plugins.
//!
//! The agents' skill folders are brought in line with the settings at start and after each toggle,
//! off the UI thread, one run at a time.

use std::sync::atomic::{AtomicBool, Ordering};

use agentdeck_core::logic::theme::{FONT_MONO, FONT_SANS, Palette};
use agentdeck_core::plugins::{self, CATALOG, Plugin};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::layout::DeckView;
use crate::panes::icon_button;
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

/// Opens the Plugins page in place of the panes.
pub fn open(deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| PluginsView::new(deck.clone(), window, cx));
    let focus = view.read(cx).focus.clone();
    crate::skills::show_page(&deck, "plugins", view.into(), focus, window, cx);
}

pub struct PluginsView {
    deck: Entity<Deck>,
    focus: FocusHandle,
    link: Entity<InputState>,
    installing: bool,
    _redraw: [Subscription; 2],
}

impl PluginsView {
    fn new(deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let link = cx.new(|cx| InputState::new(window, cx).placeholder("https://github.com/owner/repo/tree/main/skills/name"));
        Self {
            _redraw: [cx.observe_global::<SyncState>(|_, cx| cx.notify()), cx.observe(&deck, |_, _, cx| cx.notify())],
            deck,
            focus: cx.focus_handle(),
            link,
            installing: false,
        }
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
        let label = if self.installing { "Installing…" } else { "Install" };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .max_w(ui(700.0))
            .child(section_title("From GitHub", p))
            .child(div().text_xs().text_color(hsla(p.subtle)).whitespace_normal().child(
                "Any repository or folder with SKILL.md files: installed for every agent found on this computer \
                 (Claude Code, Codex, Gemini). Manage or remove them from an agent's Skills dialog.",
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.link).h(ui(32.0))))
                    .child(
                        div()
                            .id("plugins-install")
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .h(ui(32.0))
                            .px_3()
                            .rounded(ui(6.0))
                            .bg(hsla(p.accent))
                            .text_color(hsla(p.on_accent))
                            .cursor_pointer()
                            .when(self.installing, |el| el.opacity(0.6))
                            .hover(|s| s.bg(hsla(p.accent_hover)))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.install(window, cx)))
                            .child(label),
                    ),
            )
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
        let settings = self.deck.read(cx).settings.clone();
        let (notes, running) = {
            let state = cx.global::<SyncState>();
            (state.notes.clone(), state.running)
        };
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
                    .child(div().text_size(ui(15.0)).font_weight(FontWeight::MEDIUM).child("Plugins"))
                    .child(div().text_xs().text_color(hsla(p.subtle)).child(if running {
                        "Updating the agents' skills…"
                    } else {
                        "Skills for every agent at once: on here, they appear in Claude Code, Codex, Gemini and OpenCode"
                    })),
            )
            .child(icon_button("plugins-close", IconName::X, "Close (Esc)").large().on_click(crate::skills::close));
        let mut body = div().flex().flex_col().gap_6().p_6();
        for category in categories() {
            let cards: Vec<Div> = CATALOG
                .iter()
                .filter(|plugin| plugin.category == category)
                .map(|plugin| self.card(plugin, settings.plugin_on(plugin), &p, cx))
                .collect();
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(section_title(category, &p))
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
        div()
            .id("plugins")
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(|_: &ClosePlugins, window, cx| crate::skills::close(window, cx))
            .size_full()
            .flex()
            .flex_col()
            .occlude()
            .bg(hsla(p.panel))
            .text_color(hsla(p.fg))
            .text_size(ui(13.0))
            .font_family(FONT_SANS)
            .child(header)
            .child(
                div()
                    .id("plugins-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body.children(notes).child(self.github(&p, cx))),
            )
    }
}
