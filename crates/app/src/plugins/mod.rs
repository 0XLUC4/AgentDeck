//! Plugins page (rail button, and the Skills button of agent panes): a Plugins tab with AgentDeck's
//! catalog of color themes (live previews; installed ones go to Settings > Appearance, one is in use)
//! and bundled skills (cards with filters, search and categories), turned on or off for every agent at
//! once, plus skills installed from a GitHub link ("Add plugin"); then one tab per installed agent with
//! its own skills. Logic: agentdeck_core::plugins.
//!
//! The agents' skill folders are brought in line with the settings at start and after each toggle,
//! off the UI thread, one run at a time.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use agentdeck_core::agents::{AgentKind, detect_agents};
use agentdeck_core::logic::text::agent_name;
use agentdeck_core::logic::theme::{FONT_MONO, FONT_SANS, Palette, mix};
use agentdeck_core::logic::themes::ThemeVariant;
use agentdeck_core::plugins::{self, CATALOG, Plugin, THEMES};
use gpui_kit::assets::IconName;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::layout::DeckView;
use crate::panes::icon_button;
use crate::skills::{AgentDialogKind, AgentFilesDialog};
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};
use agentdeck_core::logic::theme::{RADIUS_LG, RADIUS_MD, RADIUS_SM, RADIUS_XL};

actions!(agentdeck, [ClosePlugins]);

/// Key context of the page (Esc closes).
const CONTEXT: &str = "Plugins";
/// Where the bundled plugins' sources live.
const SOURCE_URL: &str = "https://github.com/0XLUC4/AgentDeck/tree/main/crates/core/plugins";
/// Narrowest card of the grid.
const CARD_MIN_WIDTH: f32 = 220.0;
const GRID_GAP: f32 = 16.0;
/// A theme install's progress bar (nothing to copy: just long enough to read as an install).
const THEME_INSTALL: Duration = Duration::from_millis(800);
/// Shortest shimmer of a skill install, so a fast sync does not flash.
const MIN_INSTALL: Duration = Duration::from_millis(600);
/// How long the page toast after an install stays.
const TOAST_FOR: Duration = Duration::from_secs(5);

/// Progress bar of a card being installed.
#[derive(Clone, Copy)]
enum Bar {
    /// Eased from empty to full in [`THEME_INSTALL`].
    Timed,
    /// Shimmer until the real install is done.
    Busy,
}

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

/// Segmented filter of the catalog. "Installed" = turned on (its skills are in the agents' folders).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Installed,
    NotInstalled,
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
    search: Entity<InputState>,
    filter: Filter,
    /// Category chip picked (None = all).
    category: Option<&'static str>,
    /// The GitHub install popup is open.
    popup: bool,
    /// Plugin shown in the "Review & install" popup.
    review: Option<&'static Plugin>,
    installing: bool,
    /// Catalog installs running: plugin id -> (bar, key of its animations).
    progress: HashMap<&'static str, (Bar, u64)>,
    /// Plugins installed while the page is open: their "Installed" pill fades in.
    fresh: HashMap<&'static str, u64>,
    /// Variant picked on each multi-variant theme card (index in `plugin.themes`).
    variant: HashMap<&'static str, usize>,
    /// Page toast of the last install (with its key: a newer install replaces it).
    toast: Option<(&'static Plugin, u64)>,
    next_key: u64,
    /// Virtualized catalog: item 0 is the header (toolbar, chips, label), then one item per row of
    /// cards. Only the rows in view are drawn, so scrolling stays smooth with heavy theme previews.
    list: ListState,
    /// Cards per row and the plugins of each row, from the last render.
    columns: u16,
    rows: Vec<Vec<&'static Plugin>>,
    _subscriptions: [Subscription; 4],
}

impl PluginsView {
    fn new(deck: Entity<Deck>, tab: Tab, cwd: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let link = cx.new(|cx| InputState::new(window, cx).placeholder("https://github.com/owner/repo/tree/main/skills/name"));
        let link_events = cx.subscribe_in(&link, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { .. } => this.install(window, cx),
            _ => cx.notify(),
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search plugins"));
        let search_events = cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify());
        let mut this = Self {
            _subscriptions: [
                cx.observe_global::<SyncState>(|_, cx| cx.notify()),
                cx.observe(&deck, |_, _, cx| cx.notify()),
                link_events,
                search_events,
            ],
            deck,
            focus: cx.focus_handle(),
            tab,
            cwd,
            agents: Vec::new(),
            link,
            search,
            filter: Filter::All,
            category: None,
            popup: false,
            review: None,
            installing: false,
            list: ListState::new(1, ListAlignment::Top, px(600.0)),
            columns: 4,
            rows: Vec::new(),
            progress: HashMap::new(),
            fresh: HashMap::new(),
            variant: HashMap::new(),
            toast: None,
            next_key: 0,
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
        if std::mem::take(&mut self.popup) || self.review.take().is_some() {
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

    /// Turns the plugin on (skills installed for every agent, theme listed in Settings > Appearance)
    /// or off (removed). Installs go through [`Self::install_plugin`] for the progress bar and toast.
    fn set_plugin(&mut self, plugin: &'static Plugin, on: bool, cx: &mut Context<Self>) {
        self.deck.update(cx, |d, cx| {
            d.update_settings(
                |s| {
                    s.plugins.insert(plugin.id.to_owned(), on);
                },
                cx,
            )
            .detach();
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
                        format!("{} installed", names.join(", "))
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

    /// Card of the grid: banner (icon + tagline over the category tint, or a live mini window for a
    /// theme), name, author, description, footer with category, state and actions.
    fn card(&self, plugin: &'static Plugin, on: bool, active_theme: &str, p: &Palette, cx: &mut Context<Self>) -> Div {
        let progress = self.progress.get(plugin.id).copied();
        let shown = self.shown_variant(plugin, active_theme);
        // The variant of this theme in use, if any.
        let active = plugin.themes.iter().find(|v| v.id == active_theme);
        let installed_pill = div()
            .absolute()
            .top_2()
            .when(plugin.is_theme(), |d| d.right_2()) // a theme's window shows its dots on the left
            .when(!plugin.is_theme(), |d| d.left_2())
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_0p5()
            .rounded_full()
            .bg(hsla(p.panel))
            .border_1()
            .border_color(hsla(p.line))
            .text_size(ui(11.0))
            .text_color(hsla(p.ok))
            .child(Icon::new(IconName::Check).size(ui(11.0)))
            .child("Installed");
        // Installed while the page is open: the pill fades in.
        let installed_pill = match self.fresh.get(plugin.id) {
            Some(key) => installed_pill
                .with_animation(
                    SharedString::from(format!("plugin-pill-{}-{key}", plugin.id)),
                    Animation::new(Duration::from_millis(400)).with_easing(ease_out_quint()),
                    |pill, t| pill.opacity(t),
                )
                .into_any_element(),
            None => installed_pill.into_any_element(),
        };
        let banner = match shown {
            Some(variant) => theme_preview(variant),
            None => {
                let (icon, tint) = category_style(plugin.category, p);
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .px_5()
                    .bg(hsla_a(tint, 0.12))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(ui(52.0))
                            .rounded(ui(RADIUS_XL))
                            .bg(hsla_a(tint, 0.2))
                            .text_color(hsla(tint))
                            .child(Icon::new(icon).size(ui(28.0))),
                    )
                    .child(
                        div()
                            .text_center()
                            .text_size(ui(12.0))
                            .text_color(hsla(p.fg))
                            .line_clamp(2)
                            .child(tagline(plugin.description)),
                    )
            }
        }
        .relative()
        .h(ui(150.0))
        .overflow_hidden()
        .border_b_1()
        .border_color(hsla(p.line))
        .when(on && progress.is_none(), |d| d.child(installed_pill))
        .children(progress.map(|(bar, key)| progress_bar(plugin.id, bar, key, p)));

        let weak = cx.entity().downgrade();
        let menu = Button::new(SharedString::from(format!("plugin-menu-{}", plugin.id)))
            .ghost()
            .small()
            .icon(IconName::Ellipsis)
            .dropdown_menu(move |menu, _, _| {
                let details = weak.clone();
                let menu = menu.item(PopupMenuItem::new("Details").on_click(move |_, _, cx| {
                    details
                        .update(cx, |this, cx| {
                            this.review = Some(plugin);
                            cx.notify();
                        })
                        .ok();
                }));
                if !on {
                    return menu;
                }
                let remove = weak.clone();
                menu.item(PopupMenuItem::new("Uninstall").on_click(move |_, window, cx| {
                    remove.update(cx, |this, cx| this.uninstall(plugin, window, cx)).ok();
                }))
            });

        let state = if progress.is_some() {
            div().child(install_button(format!("plugin-installing-{}", plugin.id), "Installing…", true, p))
        } else if plugin.is_theme() {
            match (on, active) {
                (false, _) => div().child(
                    install_button(format!("plugin-install-{}", plugin.id), "Install", false, p)
                        .on_click(cx.listener(move |this, _, _, cx| this.install_plugin(plugin, cx))),
                ),
                // The toggle uninstalls the theme: back to AgentDeck's own, gone from Settings > Appearance.
                (true, Some(_)) => div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        crate::automations::toggle(SharedString::from(format!("plugin-{}", plugin.id)), true, p)
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.uninstall(plugin, window, cx))),
                    ),
                (true, None) => div().child(
                    install_button(format!("plugin-use-{}", plugin.id), "Use", false, p)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(variant) = shown {
                                this.use_theme(variant.id, window, cx);
                            }
                        })),
                ),
            }
        } else if on {
            div().flex().items_center().gap_2().child(
                crate::automations::toggle(SharedString::from(format!("plugin-{}", plugin.id)), on, p)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.uninstall(plugin, window, cx))),
            )
        } else {
            div().child(
                install_button(format!("plugin-review-{}", plugin.id), "Review & install", false, p)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.review = Some(plugin);
                        cx.notify();
                    })),
            )
        };

        let variants = shown
            .filter(|_| plugin.themes.len() > 1)
            .map(|shown| self.variant_pills(plugin, shown, active.is_some(), p, cx));
        let body = div()
            .flex()
            .flex_col()
            .flex_1()
            .gap_1p5()
            .p_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(div().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).child(plugin.name))
                    .child(Icon::new(IconName::BadgeCheck).size(ui(14.0)).text_color(hsla(p.accent))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(ui(18.0))
                    .child(div().flex_1().text_xs().text_color(hsla(p.subtle)).child("AgentDeck"))
                    .children(variants),
            )
            .child(
                div()
                    .h(ui(34.0))
                    .text_xs()
                    .text_color(hsla(p.muted))
                    .line_clamp(2)
                    .child(plugin.description),
            )
            .child(
                div()
                    .mt_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(category_chip(plugin.category, p))
                    .child(div().flex_1())
                    .child(state)
                    .child(menu),
            );

        let border = if active.is_some() {
            p.accent
        } else if on {
            p.line_strong
        } else {
            p.line
        };
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .overflow_hidden()
            .rounded(ui(RADIUS_LG))
            .border_1()
            .border_color(hsla(border))
            .bg(hsla(p.base))
            .child(banner)
            .child(body)
    }

    /// Variant chips of a theme card (Night, Storm, Light): pick the one previewed; on the theme in
    /// use, switch to it at once.
    fn variant_pills(
        &self,
        plugin: &'static Plugin,
        shown: &'static ThemeVariant,
        in_use: bool,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        div().flex().flex_shrink_0().gap_1().children(plugin.themes.iter().enumerate().map(|(i, variant)| {
            let selected = variant.id == shown.id;
            div()
                .id(SharedString::from(format!("theme-variant-{}", variant.id)))
                .px_1p5()
                .rounded_full()
                .border_1()
                .cursor_pointer()
                .text_size(ui(10.0))
                .when(selected, |d| d.border_color(hsla(p.accent)).bg(hsla_a(p.accent, 0.12)).text_color(hsla(p.fg)))
                .when(!selected, |d| {
                    d.border_color(hsla(p.line)).text_color(hsla(p.muted)).hover(|s| s.text_color(hsla(p.fg)))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.variant.insert(plugin.id, i);
                    if in_use {
                        this.use_theme(variant.id, window, cx);
                    }
                    cx.notify();
                }))
                .child(variant.name)
        }))
    }

    /// The variant a theme card previews: the one picked on the card, else the one in use, else the
    /// first. None for a skill plugin.
    fn shown_variant(&self, plugin: &'static Plugin, active_theme: &str) -> Option<&'static ThemeVariant> {
        self.variant
            .get(plugin.id)
            .and_then(|i| plugin.themes.get(*i))
            .or_else(|| plugin.themes.iter().find(|v| v.id == active_theme))
            .or_else(|| plugin.themes.first())
    }

    /// Catalog install with a progress bar on its card: a theme (nothing to copy) gets a short eased
    /// bar, a skill plugin a shimmer until the agents' skill folders are synced. Then the toast.
    fn install_plugin(&mut self, plugin: &'static Plugin, cx: &mut Context<Self>) {
        if self.progress.contains_key(plugin.id) {
            return;
        }
        self.next_key += 1;
        let key = self.next_key;
        let theme = plugin.is_theme();
        self.progress.insert(plugin.id, (if theme { Bar::Timed } else { Bar::Busy }, key));
        if !theme {
            self.set_plugin(plugin, true, cx);
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(if theme { THEME_INSTALL } else { MIN_INSTALL }).await;
            // Skills: the shimmer stays while the sync copying them runs.
            while !theme && this.update(cx, |_, cx| cx.global::<SyncState>().running).unwrap_or(false) {
                cx.background_executor().timer(Duration::from_millis(100)).await;
            }
            this.update(cx, |this, cx| {
                this.progress.remove(plugin.id);
                if theme {
                    this.set_plugin(plugin, true, cx);
                }
                this.fresh.insert(plugin.id, key);
                this.toast = Some((plugin, key));
                cx.notify();
            })
            .ok();
            cx.background_executor().timer(TOAST_FOR).await;
            this.update(cx, |this, cx| {
                if this.toast.is_some_and(|(_, k)| k == key) {
                    this.toast = None;
                }
                if this.fresh.get(plugin.id) == Some(&key) {
                    this.fresh.remove(plugin.id);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Turns the plugin off; a theme in use goes back to AgentDeck's own first.
    fn uninstall(&mut self, plugin: &'static Plugin, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.deck.read(cx).settings.appearance.color_theme.clone();
        if plugin.themes.iter().any(|v| v.id == active) {
            self.use_theme("", window, cx);
        }
        self.set_plugin(plugin, false, cx);
    }

    /// Makes a theme variant the app's color theme ("" = AgentDeck's own), saved with the appearance
    /// settings, and restyles at once.
    fn use_theme(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = id.to_owned();
        let appearance = self.deck.update(cx, |d, cx| {
            d.update_settings(|s| s.appearance.color_theme = id, cx).detach();
            d.settings.appearance.clone()
        });
        crate::theme::apply(&appearance, Some(window), cx);
        window.refresh();
        cx.notify();
    }

    /// Bottom-right note after an install; "Try it" uses the theme (the variant its card shows).
    fn toast_view(&self, plugin: &'static Plugin, key: u64, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        div()
            .absolute()
            .right_6()
            .bottom_6()
            .flex()
            .items_center()
            .gap_3()
            .px(ui(14.0))
            .py(ui(10.0))
            .rounded(ui(RADIUS_LG))
            .border_1()
            .border_color(hsla(p.line_strong))
            .bg(hsla(p.raised))
            .shadow_lg()
            .child(Icon::new(IconName::CircleCheck).size(ui(16.0)).text_color(hsla(p.ok)))
            .child(format!("{} installed", plugin.name))
            .when(plugin.is_theme(), |d| {
                d.child(Button::new("plugins-toast-try").small().primary().label("Try it").on_click(cx.listener(
                    move |this, _, window, cx| {
                        let active = this.deck.read(cx).settings.appearance.color_theme.clone();
                        if let Some(variant) = this.shown_variant(plugin, &active) {
                            this.use_theme(variant.id, window, cx);
                        }
                        this.toast = None;
                        cx.notify();
                    },
                )))
            })
            .with_animation(
                SharedString::from(format!("plugins-toast-{key}")),
                Animation::new(Duration::from_millis(250)).with_easing(ease_out_quint()),
                |toast, t| toast.opacity(t),
            )
            .into_any_element()
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
                "Paste the link of a GitHub repository or folder with SKILL.md files. Installed for every agent \
                 found on this computer (Claude Code, Codex, Gemini, Grok, OpenCode). See or remove them in each \
                 agent's tab.",
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
        crate::skills::popup("plugins-install-popup", "Add plugin from GitHub", body, cx).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.popup = false;
                cx.notify();
            }),
        )
    }

    /// Popup of a catalog plugin: what it does, the skills it copies, Install (or Uninstall).
    fn review_popup(&self, plugin: &'static Plugin, on: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette(cx);
        let names = plugin.skills.iter().map(|s| s.name).chain(plugin.themes.iter().map(|v| v.title));
        let skills = names.map(|name| {
            div()
                .px_1p5()
                .py_0p5()
                .rounded(ui(RADIUS_SM))
                .bg(hsla(p.hover))
                .font_family(FONT_MONO)
                .text_size(ui(11.0))
                .text_color(hsla(p.muted))
                .child(name)
        });
        let action = if on {
            Button::new("plugins-review-action")
                .ghost()
                .label("Uninstall")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.review = None;
                    this.uninstall(plugin, window, cx);
                }))
        } else {
            Button::new("plugins-review-action")
                .primary()
                .icon(IconName::Download)
                .label("Install")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.review = None;
                    this.install_plugin(plugin, cx);
                }))
        };
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().flex().items_center().gap_2().child(category_chip(plugin.category, &p)).child(
                div().text_xs().text_color(hsla(p.subtle)).child("AgentDeck · verified"),
            ))
            .child(div().text_xs().text_color(hsla(p.muted)).whitespace_normal().child(plugin.description))
            .child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(hsla(p.muted)).child(
                if plugin.is_theme() { "Variants" } else { "Skills" },
            ))
            .child(div().flex().flex_wrap().gap_1().children(skills))
            .child(div().text_xs().text_color(hsla(p.subtle)).whitespace_normal().child(if plugin.is_theme() {
                "Added to Settings > Appearance. Use it from its card or there; uninstalling the theme in use \
                 goes back to AgentDeck's own."
            } else {
                "Copied into the user skill folder of every agent on this computer, removed when uninstalled. \
                 Skills you edit are never overwritten."
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("plugins-review-cancel").ghost().label("Cancel").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.review = None;
                            cx.notify();
                        },
                    )))
                    .child(action),
            );
        crate::skills::popup("plugins-review-popup", plugin.name, body, cx).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.review = None;
                cx.notify();
            }),
        )
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
                    .rounded(ui(RADIUS_MD))
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
        div().flex().child(div().flex().flex_wrap().gap_1().rounded(ui(RADIUS_LG)).bg(hsla(p.base)).p_1().children(buttons))
    }

    /// Filter row: All | Installed N | Not installed, search, refresh.
    fn toolbar(&self, installed: usize, p: &Palette, cx: &mut Context<Self>) -> Div {
        let running = cx.global::<SyncState>().running;
        let deck = self.deck.clone();
        let options = [
            (Filter::All, "All".to_owned()),
            (Filter::Installed, format!("Installed {installed}")),
            (Filter::NotInstalled, "Not installed".to_owned()),
        ];
        let segments = options.into_iter().enumerate().map(|(i, (value, label))| {
            let selected = self.filter == value;
            div()
                .id(("plugins-filter", i))
                .px_3()
                .h(ui(26.0))
                .flex()
                .items_center()
                .rounded(ui(RADIUS_MD))
                .cursor_pointer()
                .text_xs()
                .when(selected, |d| d.bg(hsla(p.raised)).text_color(hsla(p.fg)).font_weight(FontWeight::MEDIUM))
                .when(!selected, |d| d.text_color(hsla(p.muted)).hover(|s| s.text_color(hsla(p.fg))))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.filter = value;
                    cx.notify();
                }))
                .child(label)
        });
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex()
                    .p(ui(2.0))
                    .gap(ui(2.0))
                    .rounded_md()
                    .border_1()
                    .border_color(hsla(p.line))
                    .children(segments),
            )
            .child(
                div().flex_1().min_w(ui(180.0)).max_w(ui(360.0)).child(
                    Input::new(&self.search)
                        .h(ui(32.0))
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).text_color(hsla(p.subtle))),
                ),
            )
            .child(
                icon_button(
                    "plugins-refresh",
                    IconName::RefreshCw,
                    if running { "Updating the agents' skills…" } else { "Re-sync the agents' skills" },
                )
                .disabled(running)
                .on_click(move |_, cx| sync(deck.clone(), cx)),
            )
    }

    /// Category chips (with icons) of the categories in the catalog; a second click clears.
    fn category_chips(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let chips = categories().into_iter().enumerate().map(|(i, category)| {
            let (icon, _) = category_style(category, p);
            let selected = self.category == Some(category);
            div()
                .id(("plugins-category", i))
                .flex()
                .items_center()
                .gap_1p5()
                .h(ui(28.0))
                .px_3()
                .rounded_full()
                .border_1()
                .cursor_pointer()
                .text_xs()
                .when(selected, |d| d.border_color(hsla(p.accent)).bg(hsla_a(p.accent, 0.12)).text_color(hsla(p.fg)))
                .when(!selected, |d| {
                    d.border_color(hsla(p.line)).text_color(hsla(p.muted)).hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.category = if this.category == Some(category) { None } else { Some(category) };
                    cx.notify();
                }))
                .child(Icon::new(icon).size(ui(13.0)))
                .child(category)
        });
        div().flex().flex_wrap().gap_2().children(chips)
    }

    /// Plugins passing the filter, category and search, in catalog order.
    fn visible_plugins(&self, cx: &App) -> Vec<&'static Plugin> {
        let settings = &self.deck.read(cx).settings;
        let query = self.search.read(cx).value().trim().to_lowercase();
        CATALOG
            .iter()
            .filter(|plugin| {
                let on = settings.plugin_on(plugin);
                let by_filter = match self.filter {
                    Filter::All => true,
                    Filter::Installed => on,
                    Filter::NotInstalled => !on,
                };
                by_filter && self.category.is_none_or(|c| c == plugin.category) && matches(plugin, &query)
            })
            .collect()
    }

    /// Recomputes the rows for `columns` and tells the list when their number changed.
    fn update_rows(&mut self, columns: u16, cx: &App) {
        let visible = self.visible_plugins(cx);
        self.columns = columns;
        self.rows = visible.chunks(usize::from(columns.max(1))).map(<[_]>::to_vec).collect();
        // Header + rows, or header + the "no match" line.
        let count = 1 + self.rows.len().max(1);
        let old = self.list.item_count();
        if old != count {
            self.list.splice(0..old, count);
        }
    }

    /// One item of the virtualized catalog.
    fn list_item(&mut self, index: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        if index == 0 {
            return self.catalog_header(&p, cx).into_any_element();
        }
        let Some(row) = self.rows.get(index - 1).cloned() else {
            return div().py_10().text_center().text_xs().text_color(hsla(p.subtle)).child("No plugin matches.").into_any_element();
        };
        let settings = self.deck.read(cx).settings.clone();
        let active_theme = settings.appearance.color_theme.clone();
        let cards: Vec<Div> = row.into_iter().map(|plugin| self.card(plugin, settings.plugin_on(plugin), &active_theme, &p, cx)).collect();
        let last = index == self.rows.len();
        div()
            .px_6()
            .pb(ui(if last { 24.0 } else { GRID_GAP }))
            .child(div().grid().grid_cols(self.columns).gap(ui(GRID_GAP)).children(cards))
            .into_any_element()
    }

    /// Toolbar, category chips, sync notes and the section label above the cards.
    fn catalog_header(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let settings = self.deck.read(cx).settings.clone();
        let notes = cx.global::<SyncState>().notes.clone();
        let installed = CATALOG.iter().filter(|plugin| settings.plugin_on(plugin)).count();
        let shown = self.rows.iter().map(Vec::len).sum::<usize>();
        let notes = (!notes.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_2()
                .rounded(ui(RADIUS_MD))
                .bg(hsla_a(p.wait, 0.1))
                .text_xs()
                .text_color(hsla(p.wait))
                .children(notes.into_iter().map(|note| div().whitespace_normal().child(note)))
        });
        div()
            .flex()
            .flex_col()
            .gap_4()
            .px_6()
            .pt_6()
            .pb_4()
            .child(self.toolbar(installed, p, cx))
            .child(self.category_chips(p, cx))
            .children(notes)
            .child(
                div()
                    .mt_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(ui(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(hsla(p.muted))
                    .child("OFFICIAL PLUGINS")
                    .child(div().text_color(hsla(p.subtle)).child("· verified"))
                    .child(div().flex_1())
                    .child(div().text_color(hsla(p.subtle)).child(format!("{shown} of {}", CATALOG.len()))),
            )
    }
}

/// Slim bar along the bottom of a card's banner while it installs.
fn progress_bar(id: &str, bar: Bar, key: u64, p: &Palette) -> Div {
    let anim_id = SharedString::from(format!("plugin-bar-{id}-{key}"));
    let fill = div().absolute().top_0().bottom_0().left_0().rounded_full().bg(hsla(p.accent));
    let fill = match bar {
        Bar::Timed => fill
            .with_animation(anim_id, Animation::new(THEME_INSTALL).with_easing(ease_in_out), |fill, t| fill.w(relative(t)))
            .into_any_element(),
        // A third of the width sweeping across, again and again.
        Bar::Busy => fill
            .w(relative(0.35))
            .with_animation(
                anim_id,
                Animation::new(Duration::from_millis(1100)).repeat().with_easing(ease_in_out),
                |fill, t| fill.left(relative(1.35 * t - 0.35)),
            )
            .into_any_element(),
    };
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(ui(3.0))
        .overflow_hidden()
        .bg(hsla_a(p.accent, 0.18))
        .child(fill)
}

/// Theme card banner: a light mini window in the theme's own colors (title bar, a few "code" bars
/// in its syntax colors, a strip of its hues). Plain boxes only, no text shaping or shadows: the
/// grid shows many of them and must stay smooth while scrolling or moving the window.
fn theme_preview(variant: &ThemeVariant) -> Div {
    let t = variant.palette();
    let s = t.syntax;
    let dot = |color: u32| div().size(ui(6.0)).rounded_full().bg(hsla(color));
    let bar = |width: f32, color: u32| div().h(ui(4.0)).w(ui(width)).rounded_full().bg(hsla(color));
    let line = |indent: f32, bars: &[(f32, u32)]| {
        let mut row = div().flex().items_center().gap(ui(4.0)).pl(ui(indent));
        for (width, color) in bars {
            row = row.child(bar(*width, *color));
        }
        row
    };
    let code = div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(ui(7.0))
        .p(ui(10.0))
        .child(line(0.0, &[(46.0, s.comment)]))
        .child(line(0.0, &[(14.0, s.keyword), (26.0, s.function), (18.0, s.type_)]))
        .child(line(10.0, &[(16.0, s.keyword), (34.0, s.string)]))
        .child(line(10.0, &[(22.0, s.function), (12.0, s.number), (20.0, t.fg)]))
        .child(line(0.0, &[(8.0, t.muted)]));
    let hues = [t.accent, t.bad, t.wait, t.ok, t.busy, s.keyword];
    let window = div()
        .flex()
        .flex_col()
        .flex_1()
        .overflow_hidden()
        .rounded_t(ui(RADIUS_LG))
        .border_1()
        .border_b_0()
        .border_color(hsla(t.line_strong))
        .bg(hsla(t.base))
        .child(
            div()
                .flex()
                .items_center()
                .gap(ui(4.0))
                .h(ui(16.0))
                .px(ui(7.0))
                .bg(hsla(t.gutter))
                .child(dot(t.bad))
                .child(dot(t.wait))
                .child(dot(t.ok)),
        )
        .child(div().flex().flex_1().child(div().w(ui(40.0)).bg(hsla(t.panel))).child(code))
        .child(div().flex().h(ui(5.0)).children(hues.map(|hue| div().flex_1().bg(hsla(hue)))));
    div().flex().flex_col().px(ui(16.0)).pt(ui(16.0)).bg(hsla(mix(t.gutter, t.accent, 0.85))).child(window)
}

/// Icon and tint of a catalog category.
fn category_style(category: &str, p: &Palette) -> (IconName, u32) {
    match category {
        THEMES => (IconName::Palette, p.agent_claude),
        "Browser" => (IconName::Globe, p.agent_gemini),
        "Engineering" => (IconName::Code, p.accent),
        "Quality" => (IconName::ShieldCheck, p.ok),
        "Performance" => (IconName::Gauge, p.busy),
        "Research" => (IconName::Telescope, p.agent_codex),
        "Style" => (IconName::Feather, p.agent_claude),
        "3D" => (IconName::Cuboid, p.agent_grok),
        _ => (IconName::Puzzle, p.accent),
    }
}

/// The card's main action: white (the text color) and rounded; a spinner replaces the icon while
/// `loading`, and the button stops reacting.
fn install_button(id: String, label: &'static str, loading: bool, p: &Palette) -> Stateful<Div> {
    let icon: AnyElement = if loading {
        gpui_kit::component::spinner::Spinner::new().icon(gpui_kit::assets::IconName::LoaderCircle).with_size(px(14.0)).color(hsla(p.base)).into_any_element()
    } else if label == "Use" {
        Icon::new(IconName::Check).size(ui(14.0)).into_any_element()
    } else {
        Icon::new(IconName::Download).size(ui(14.0)).into_any_element()
    };
    div()
        .id(SharedString::from(id))
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(ui(6.0))
        .h(ui(32.0))
        .px(ui(14.0))
        .rounded(ui(RADIUS_MD))
        .bg(hsla(p.fg))
        .text_color(hsla(p.base))
        .text_size(ui(12.5))
        .font_weight(FontWeight::SEMIBOLD)
        .child(icon)
        .child(label)
        .when(loading, |el| el.opacity(0.85).cursor_default())
        .when(!loading, |el| el.cursor_pointer().hover(|s| s.opacity(0.88)).active(|s| s.opacity(0.75)))
}

fn category_chip(category: &'static str, p: &Palette) -> Div {
    let (icon, _) = category_style(category, p);
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_1()
        .px_1p5()
        .py_0p5()
        .rounded(ui(RADIUS_SM))
        .bg(hsla(p.hover))
        .text_size(ui(11.0))
        .text_color(hsla(p.muted))
        .child(Icon::new(icon).size(ui(11.0)))
        .child(category)
}

/// First clause of a description, shown big in the card banner.
fn tagline(description: &str) -> &str {
    let end = [": ", ". ", ", "].iter().filter_map(|sep| description.find(sep)).min().unwrap_or(description.len());
    description[..end].trim_end_matches('.')
}

/// The search (lowercased) is in the plugin's name, description, category or skill names.
fn matches(plugin: &Plugin, query: &str) -> bool {
    query.is_empty()
        || [plugin.name, plugin.description, plugin.category]
            .into_iter()
            .chain(plugin.skills.iter().map(|s| s.name))
            .chain(plugin.themes.iter().map(|v| v.title))
            .any(|text| text.to_lowercase().contains(query))
}

/// Catalog categories in first-seen order.
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let this = cx.entity().downgrade();
        let header = div()
            .flex()
            .flex_shrink_0()
            .flex_col()
            .gap_4()
            .border_b_1()
            .border_color(hsla(p.line))
            .px_6()
            .pt_5()
            .pb_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().text_size(ui(24.0)).font_weight(FontWeight::SEMIBOLD).child("Plugins"))
                                    .child(
                                        div()
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded(ui(RADIUS_SM))
                                            .border_1()
                                            .border_color(hsla(p.line_strong))
                                            .text_size(ui(10.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(hsla(p.muted))
                                            .child("ALPHA"),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(ui(13.0))
                                    .text_color(hsla(p.subtle))
                                    .child("Themes, skills, workflows and more."),
                            ),
                    )
                    .child(
                        Button::new("plugins-add")
                            .primary()
                            .icon(IconName::Plus)
                            .label("Add plugin")
                            .on_click(cx.listener(|this, _, window, cx| this.open_popup(window, cx))),
                    )
                    .child(
                        Button::new("plugins-more")
                            .ghost()
                            .icon(IconName::Ellipsis)
                            .dropdown_menu(|menu, _, _| menu.link("Plugins source on GitHub", SOURCE_URL)),
                    )
                    .child(crate::panes::close_button("plugins-close", "Close (Esc)").on_click(move |window, cx| {
                        let _ = this.update(cx, |this, cx| this.request_close(window, cx));
                    })),
            )
            .child(self.tabs(&p, cx));
        let content = match self.agent_view() {
            Some(view) => div().id("plugins-body").flex_1().min_h_0().overflow_y_scroll().child(view),
            None => {
                // ~4 cards per row on a wide window, never narrower than CARD_MIN_WIDTH.
                let rem = window.rem_size();
                let width = f32::from(window.viewport_size().width) - f32::from(ui(160.0).to_pixels(rem));
                let card = f32::from(ui(CARD_MIN_WIDTH + GRID_GAP).to_pixels(rem));
                let columns = (width / card).floor().clamp(1.0, 4.0) as u16;
                self.update_rows(columns, cx);
                let items = list(self.list.clone(), cx.processor(|this, index, window, cx| this.list_item(index, window, cx)));
                div().id("plugins-body").flex().flex_col().flex_1().min_h_0().child(items.flex_1().size_full())
            }
        };
        let popup = self.popup.then(|| self.install_popup(cx));
        let review = self.review.map(|plugin| {
            let on = self.deck.read(cx).settings.plugin_on(plugin);
            self.review_popup(plugin, on, cx)
        });
        let toast = self.toast.map(|(plugin, key)| self.toast_view(plugin, key, cx));
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
            .children(toast)
            .children(popup)
            .children(review)
    }
}
