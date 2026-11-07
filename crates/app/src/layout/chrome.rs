//! Window chrome around the dock: title bar (TitleBar.tsx), session tab bar with the add-agent
//! button (TabBar.tsx), sidebar of workspaces and sessions (Sidebar.tsx), activity list,
//! empty session (App.tsx) and toasts (Toasts.tsx).

use agentdeck_core::agents::AgentKind;
use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::{ActivityKey, activity_key, can_archive};
use agentdeck_core::logic::text::{AGENT_KINDS, agent_name, now_ms, split_args};
use agentdeck_core::logic::theme::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::component::TitleBar;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::DeckView;
use crate::panes::{agent_logo, last_agent, runtime, spawn_agent};
use crate::theme::{hsla, palette, ui};

/// IconButton.tsx: a square ghost button with an icon and a tooltip.
pub(super) fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    hint: impl Into<SharedString>,
    p: &Palette,
) -> Stateful<Div> {
    let hint: SharedString = hint.into();
    let hover = hsla(p.hover);
    let fg = hsla(p.fg);
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(ui(28.0))
        .rounded(ui(6.0))
        .text_size(ui(16.0))
        .text_color(hsla(p.muted))
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(fg))
        .child(icon)
        .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
}

/// Title bar logo, resized on the CPU to the exact device pixels: GPU downscaling of a big PNG looks blurry.
fn logo(size: f32, scale: f32) -> std::sync::Arc<Image> {
    use std::sync::{Arc, Mutex};
    static CACHE: Mutex<Option<(u32, Arc<Image>)>> = Mutex::new(None);
    let target = (size * scale).round().max(1.0) as u32;
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((cached, logo)) = cache.as_ref()
        && *cached == target
    {
        return logo.clone();
    }
    let source = include_bytes!("../../assets/logo.png");
    let mut png = Vec::new();
    let resized = ::image::load_from_memory(source).map(|img| {
        img.resize_exact(target, target, ::image::imageops::FilterType::Lanczos3)
            .write_to(&mut std::io::Cursor::new(&mut png), ::image::ImageFormat::Png)
    });
    // Decoding our own embedded PNG cannot fail in practice; fall back to the original bytes anyway.
    let bytes = if matches!(resized, Ok(Ok(()))) { png } else { source.to_vec() };
    let logo = Arc::new(Image::from_bytes(ImageFormat::Png, bytes));
    *cache = Some((target, logo.clone()));
    logo
}

/// "1/2/26, 3:04 PM" (en-US short date and time, like the original's Intl format).
fn format_date(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%-m/%-d/%y, %-I:%M %p").to_string())
        .unwrap_or_default()
}

/// Shortcuts worth discovering, shown where a new user lands.
const TIPS: [(&str, &str); 6] = [
    ("Ctrl+K", "Every command"),
    ("Ctrl+1", "Go to agent 1, 2, 3..."),
    ("Ctrl+Space", "Hold to talk to an agent"),
    ("Ctrl+Shift+A", "What each agent is doing"),
    ("Ctrl+Shift+J", "Jump to the agent waiting for you"),
    ("Ctrl+Shift+B", "Send one prompt to several agents"),
];

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

impl DeckView {
    pub(super) fn title_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let deck = self.deck.read(cx);
        let workspace = deck.state.active_workspace().name.clone();
        let session = deck.state.active_tab().map(|t| t.name.clone()).unwrap_or_default();
        let waiting = deck
            .state
            .panes
            .values()
            .filter(|pane| activity_key(pane) == ActivityKey::Status(PaneStatus::Waiting))
            .count();
        let failed = deck
            .state
            .panes
            .values()
            .any(|pane| activity_key(pane) == ActivityKey::Status(PaneStatus::Error));
        // Taskbar hint: "AgentDeck (2)" while agents wait for an answer.
        let title = if waiting > 0 { format!("AgentDeck ({waiting})") } else { "AgentDeck".into() };
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }

        let sidebar_hint = if self.sidebar_open { "Hide workspaces" } else { "Show workspaces" };
        let activity_hint = format!(
            "{} activity of all agents (Ctrl+Shift+A). Next waiting agent: Ctrl+Shift+J",
            if self.activity_open { "Hide" } else { "Show" }
        );
        let chevron = || div().text_size(ui(14.0)).text_color(hsla(p.subtle)).child(IconName::ChevronRight);
        let separator = || div().w(px(1.0)).h(ui(20.0)).bg(hsla(p.line));
        let activity = div()
            .relative()
            .child(
                icon_button("tb-activity", IconName::Activity, activity_hint, &p)
                    .when(self.activity_open, |el| el.bg(hsla(p.raised)).text_color(hsla(p.fg)))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.activity_open = !this.activity_open;
                        cx.notify();
                    })),
            )
            .when(waiting > 0, |el| {
                el.child(
                    div()
                        .absolute()
                        .top(ui(-2.0))
                        .right(ui(-2.0))
                        .min_w(ui(14.0))
                        .px_1()
                        .rounded_full()
                        .bg(hsla(p.wait))
                        .text_size(ui(10.0))
                        .line_height(ui(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hsla(p.on_accent))
                        .child(waiting.to_string()),
                )
            })
            .when(failed, |el| {
                el.child(
                    div()
                        .absolute()
                        .bottom(ui(-2.0))
                        .right(ui(-2.0))
                        .size(ui(8.0))
                        .rounded_full()
                        .bg(hsla(p.bad)),
                )
            });

        let content = div()
            .flex()
            .items_center()
            .size_full()
            .text_size(ui(13.0))
            .child(
                icon_button("tb-sidebar", IconName::PanelLeft, sidebar_hint, &p)
                    .occlude()
                    .when(self.sidebar_open, |el| el.text_color(hsla(p.fg)))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.sidebar_open = !this.sidebar_open;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .ml_2()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child({ let size = ui(18.0).to_pixels(window.rem_size()); img(logo(f32::from(size), window.scale_factor())).size(size).flex_shrink_0() })
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_color(hsla(p.fg)).child("AgentDeck"))
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .items_center()
                            .gap_1()
                            .pl_2()
                            .text_color(hsla(p.muted))
                            .child(chevron())
                            .child(div().whitespace_nowrap().overflow_hidden().child(workspace))
                            .child(chevron())
                            .child(div().whitespace_nowrap().overflow_hidden().text_color(hsla(p.fg)).child(session)),
                    ),
            )
            .child(div().flex_1().min_w(ui(24.0)).h_full())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(ui(2.0))
                    .pr_2()
                    .occlude()
                    .child(
                        icon_button(
                            "tb-broadcast",
                            IconName::Megaphone,
                            "Broadcast a prompt to several agents (Ctrl+Shift+B)",
                            &p,
                        )
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::palette::OpenBroadcast), cx)),
                    )
                    .child(
                        icon_button(
                            "tb-interrupt",
                            IconName::Pause,
                            "Interrupt all: sends Esc (Ctrl+C for a shell) to every agent in the session. Agents stay open.",
                            &p,
                        )
                        .on_click(|_, _, cx| runtime::interrupt_all(cx)),
                    )
                    .child(
                        icon_button("tb-restart", IconName::RotateCcw, "Restart every agent in the session", &p)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.confirm_bulk("Restart", window, cx, runtime::restart_all)
                            })),
                    )
                    .child(
                        icon_button("tb-stop", IconName::SquareStop, "Stop every process in the session", &p)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.confirm_bulk("Stop", window, cx, runtime::stop_all)
                            })),
                    ),
            )
            .child(separator())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(ui(2.0))
                    .px_2()
                    .occlude()
                    .child(crate::orchestrator::wall_toggle_button(cx))
                    .child(
                        icon_button("tb-usage", IconName::Gauge, "Usage and cost of the agents", &p).on_click(
                            |_, window, cx| window.dispatch_action(Box::new(crate::history::OpenUsage), cx),
                        ),
                    )
                    .child(
                        icon_button("tb-history", IconName::Clock, "Search history (Ctrl+Shift+F)", &p).on_click(
                            |_, window, cx| window.dispatch_action(Box::new(crate::history::SearchHistory), cx),
                        ),
                    )
                    .child(
                        icon_button("tb-journal", IconName::BookOpenText, "Journal of the day", &p).on_click(
                            |_, window, cx| window.dispatch_action(Box::new(crate::history::OpenJournal), cx),
                        ),
                    )
                    .child(activity)
                    .child(
                        icon_button("tb-settings", IconName::Settings, "Settings (Ctrl+,)", &p).on_click(
                            |_, window, cx| window.dispatch_action(Box::new(crate::settings::OpenSettings), cx),
                        ),
                    )
                    .child(self.update_button.clone()),
            );
        TitleBar::new()
            .bg(hsla(p.panel))
            .border_b_1()
            .border_color(hsla(p.line))
            .child(content)
            .into_any_element()
    }

    /// Runs a bulk action on the visible session, asking first unless disabled in Settings.
    fn confirm_bulk(&mut self, verb: &'static str, window: &mut Window, cx: &mut Context<Self>, action: fn(&mut App)) {
        let deck = self.deck.read(cx);
        let count = deck.state.active_tab().map_or(0, |t| t.pane_ids.len());
        if count == 0 {
            return;
        }
        if !deck.settings.confirm_bulk {
            action(cx);
            return;
        }
        let question = format!("{verb} the {count} agent(s) in this session?");
        let answer = window.prompt(PromptLevel::Warning, &question, None, &[verb, "Cancel"], cx);
        cx.spawn(async move |_, cx| {
            if answer.await.ok() == Some(0) {
                cx.update(action);
            }
        })
        .detach();
    }

    /// Open sessions of the current workspace, plus "new session" and the primary "add agent" action.
    pub(super) fn tab_bar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let deck = self.deck.read(cx);
        let workspace = deck.state.active_workspace().id.clone();
        let active_tab = deck.state.active_tab_id.clone();
        let tabs: Vec<(String, String, bool)> = deck
            .state
            .tabs
            .iter()
            .filter(|t| t.workspace_id == workspace)
            .map(|t| (t.id.clone(), t.name.clone(), can_archive(&deck.state.tabs, &t.id)))
            .collect();

        let mut list = div()
            .id("session-tabs")
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .h_full()
            .overflow_x_scroll();
        for (id, name, archivable) in tabs {
            let active = id == active_tab;
            if let Some((_, input, _)) = self.renaming.as_ref().filter(|r| r.0 == id) {
                let cancel = id.clone();
                list = list.child(
                    div()
                        .w(ui(160.0))
                        .flex_shrink_0()
                        .capture_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                            if e.keystroke.key == "escape" && this.renaming.as_ref().is_some_and(|r| r.0 == cancel) {
                                this.renaming = None;
                                cx.stop_propagation();
                                cx.notify();
                            }
                        }))
                        .child(Input::new(input)),
                );
                continue;
            }
            let group: SharedString = format!("session-tab-{id}").into();
            let (select, rename, close) = (id.clone(), id.clone(), id.clone());
            let tabs_focus = self.tabs_focus.clone();
            list = list.child(
                div()
                    .id(ElementId::Name(group.clone()))
                    .group(group.clone())
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .h_full()
                    .pl_3()
                    .pr(ui(2.0))
                    .gap_1()
                    .border_b_2()
                    .border_color(if active { hsla(p.accent) } else { hsla(p.panel) })
                    .text_color(hsla(if active { p.fg } else { p.muted }))
                    .hover(|s| s.text_color(hsla(p.fg)))
                    .tooltip(|window, cx| Tooltip::new("Double-click or F2 to rename").build(window, cx))
                    .child(div().max_w(ui(192.0)).overflow_hidden().whitespace_nowrap().child(name.clone()))
                    .when(archivable, |el| {
                        el.child(
                            icon_button(
                                ElementId::Name(format!("session-close-{id}").into()),
                                IconName::X,
                                "Close (the session stays in the sidebar and can be reopened)",
                                &p,
                            )
                            .size(ui(22.0))
                            .when(!active, |b| b.opacity(0.0).group_hover(group.clone(), |s| s.opacity(1.0)))
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.close_session(&close, window, cx);
                            })),
                        )
                    })
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        window.focus(&tabs_focus, cx);
                        if e.click_count() == 2 {
                            this.start_rename(&rename, window, cx);
                            return;
                        }
                        let id = select.clone();
                        this.deck.update(cx, |deck, cx| {
                            deck.state.active_tab_id = id;
                            cx.notify();
                        });
                    })),
            );
        }

        div()
            .key_context("SessionTabs")
            .track_focus(&self.tabs_focus)
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .h(ui(40.0))
            .px_2()
            .border_b_1()
            .border_color(hsla(p.line))
            .bg(hsla(p.panel))
            .child(list)
            .child(
                icon_button("new-session", IconName::Plus, "New session", &p).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.deck.update(cx, |deck, cx| {
                            deck.state.add_tab(None);
                            deck.save(cx);
                            cx.notify();
                        });
                    },
                )),
            )
            .child(div().ml_auto().child(self.add_agent_button("tabbar", cx)))
            .into_any_element()
    }

    /// F2 / double click on a session tab (or a workspace, id `ws:<id>`): an inline name field. Enter or blur saves, Esc cancels.
    pub(super) fn start_rename(&mut self, tab_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let state = &self.deck.read(cx).state;
        let name = match tab_id.strip_prefix("ws:") {
            Some(ws) => state.workspaces.iter().find(|w| w.id == ws).map(|w| w.name.clone()),
            None => state.tabs.iter().find(|t| t.id == tab_id).map(|t| t.name.clone()),
        };
        let Some(name) = name else {
            return;
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        input.update(cx, |state, cx| state.focus(window, cx));
        let subscription = cx.subscribe_in(&input, window, |this, input, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let value = input.read(cx).value().trim().to_owned();
                this.finish_rename(value, cx);
            }
        });
        self.renaming = Some((tab_id.to_owned(), input, subscription));
        cx.notify();
    }

    fn finish_rename(&mut self, value: String, cx: &mut Context<Self>) {
        let Some((tab_id, _, _)) = self.renaming.take() else { return };
        if !value.is_empty() {
            self.deck.update(cx, |deck, cx| {
                match tab_id.strip_prefix("ws:") {
                    Some(ws) => deck.state.rename_workspace(ws, &value),
                    None => deck.state.rename_tab(&tab_id, &value),
                }
                deck.save(cx);
                cx.notify();
            });
        }
        cx.notify();
    }

    /// Closes a session tab into the archive, asking first if agents are still working.
    pub(super) fn close_session(&mut self, tab_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let now = now_ms();
        let found = self.deck.read(cx).state.tabs.iter().find(|t| t.id == tab_id).map(|t| {
            (t.name.clone(), t.pane_ids.iter().any(|id| runtime::is_busy(id, now, cx)))
        });
        let Some((name, busy)) = found else { return };
        let id = tab_id.to_owned();
        if !busy {
            runtime::archive_session(&id, cx);
            return;
        }
        let question = format!("Agents in \"{name}\" are still working. Archive anyway?");
        let answer = window.prompt(PromptLevel::Warning, &question, None, &["Archive", "Cancel"], cx);
        cx.spawn(async move |_, cx| {
            if answer.await.ok() == Some(0) {
                cx.update(|cx| runtime::archive_session(&id, cx));
            }
        })
        .detach();
    }

    /// Workspace with a folder: one click starts the last used agent there (with its Settings
    /// defaults), the arrow picks another one.
    fn add_agent_button(&mut self, key: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let path = self.deck.read(cx).state.active_workspace().path.clone();
        let agent = last_agent(cx);
        let primary = |id: &'static str| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(ui(6.0))
                .h(ui(32.0))
                .bg(hsla(p.accent))
                .text_color(hsla(p.on_accent))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .hover(|s| s.bg(hsla(p.accent_hover)))
        };
        if path.is_empty() {
            return primary(if key == "tabbar" { "add-agent-tabbar" } else { "add-agent-empty" })
                .rounded(ui(6.0))
                .px_3()
                .child(div().text_size(ui(15.0)).child(IconName::Plus))
                .child("Add agent")
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_picker(window, cx)))
                .into_any_element();
        }
        let menu_open = self.add_menu_open == Some(key);
        let mut menu = div()
            .absolute()
            .top(ui(36.0))
            .right_0()
            .w(ui(192.0))
            .flex()
            .flex_col()
            .p_1()
            .rounded(ui(8.0))
            .border_1()
            .border_color(hsla(p.line))
            .bg(hsla(p.panel))
            .shadow_lg()
            .on_mouse_down_out(cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                // A press on this menu's own arrow is left to the arrow's toggle, or it would close then reopen.
                if this.add_arrow_hover == Some(key) {
                    return;
                }
                this.add_menu_open = None;
                cx.notify();
            }));
        for kind in AGENT_KINDS {
            menu = menu.child(
                div()
                    .id(ElementId::Name(format!("add-{key}-{}", agent_name(kind)).into()))
                    .flex()
                    .items_center()
                    .gap(ui(10.0))
                    .h(ui(32.0))
                    .px_2()
                    .rounded(ui(6.0))
                    .text_color(hsla(p.fg))
                    .hover(|s| s.bg(hsla(p.hover)))
                    .child(agent_logo(kind, 15.0, None, &p))
                    .child(agent_name(kind))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.add_agent(kind, cx))),
            );
        }
        menu = menu.child(div().my_1().h(px(1.0)).bg(hsla(p.line))).child(
            div()
                .id(ElementId::Name(format!("add-{key}-more").into()))
                .flex()
                .items_center()
                .gap(ui(10.0))
                .h(ui(32.0))
                .px_2()
                .rounded(ui(6.0))
                .text_color(hsla(p.muted))
                .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                .child(div().text_size(ui(15.0)).child(IconName::SlidersHorizontal))
                .child("More options...")
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.add_menu_open = None;
                    this.open_picker(window, cx);
                })),
        );
        div()
            .relative()
            .flex()
            .flex_shrink_0()
            .child(
                primary(if key == "tabbar" { "add-main-tabbar" } else { "add-main-empty" })
                    .rounded_l(ui(6.0))
                    .pl_3()
                    .pr(ui(10.0))
                    .child(agent_logo(agent, 15.0, Some(hsla(p.on_accent)), &p))
                    .child(format!("Add {}", agent_name(agent)))
                    .tooltip(move |window, cx| {
                        Tooltip::new(format!("Start {} in {path}", agent_name(agent))).build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.add_agent(agent, cx))),
            )
            .child(
                primary(if key == "tabbar" { "add-menu-tabbar" } else { "add-menu-empty" })
                    .rounded_r(ui(6.0))
                    .px(ui(6.0))
                    .border_l_1()
                    .border_color(hsla(p.accent_hover))
                    .text_size(ui(15.0))
                    .child(IconName::ChevronDown)
                    .on_hover(cx.listener(move |this, hovered: &bool, _, _| {
                        if *hovered {
                            this.add_arrow_hover = Some(key);
                        } else if this.add_arrow_hover == Some(key) {
                            this.add_arrow_hover = None;
                        }
                    }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.add_menu_open = if this.add_menu_open == Some(key) { None } else { Some(key) };
                        cx.notify();
                    })),
            )
            .when(menu_open, |el| el.child(deferred(menu).with_priority(1)))
            .into_any_element()
    }

    /// Starts `kind` in the workspace folder with its Settings defaults, without a dialog.
    fn add_agent(&mut self, kind: AgentKind, cx: &mut Context<Self>) {
        self.add_menu_open = None;
        let deck = self.deck.read(cx);
        let path = deck.state.active_workspace().path.clone();
        let defaults = deck.settings.agent(kind);
        spawn_agent(kind, &path, split_args(&defaults.args), defaults.worktree, cx);
        cx.notify();
    }

    fn link_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::orchestrator::open_link_workspace(self.deck.clone(), window, cx);
    }

    /// Unlinks a workspace after confirmation: its sessions are closed, no files are deleted.
    fn confirm_delete_archived(&mut self, session_id: &str, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Permanently delete \"{name}\"?"),
            None,
            &["Delete", "Cancel"],
            cx,
        );
        let id = session_id.to_owned();
        cx.spawn(async move |this, cx| {
            if answer.await.ok() == Some(0) {
                this.update(cx, |this, cx| {
                    this.deck.update(cx, |deck, cx| {
                        deck.state.delete_archived(&id);
                        deck.save(cx);
                        cx.notify();
                    });
                })
                .ok();
            }
        })
        .detach();
    }

    fn confirm_unlink(&mut self, workspace_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let state = &self.deck.read(cx).state;
        let Some(name) = state.workspaces.iter().find(|w| w.id == workspace_id).map(|w| w.name.clone()) else {
            return;
        };
        let count = state.tabs.iter().filter(|t| t.workspace_id == workspace_id).count()
            + state.archived.iter().filter(|s| s.workspace_id == workspace_id).count();
        let detail =
            format!("Its {count} session(s) will be closed and forgotten by AgentDeck. No files are deleted from disk.");
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Unlink \"{name}\"?"),
            Some(&detail),
            &["Unlink", "Cancel"],
            cx,
        );
        let id = workspace_id.to_owned();
        cx.spawn(async move |_, cx| {
            if answer.await.ok() == Some(0) {
                cx.update(|cx| runtime::unlink_workspace(&id, cx));
            }
        })
        .detach();
    }

    /// Shown over the dock when the visible session has no agent.
    pub(super) fn empty_session(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let path = self.deck.read(cx).state.active_workspace().path.clone();
        let logos = div().flex().gap_3().opacity(0.8).children(
            AGENT_KINDS
                .into_iter()
                .map(|kind| agent_logo(kind, 20.0, None, &p)),
        );
        // One text run so it wraps as a sentence; the folder stands out in the foreground color.
        let lead = "Start Claude Code, Codex, Gemini CLI, Grok Build, OpenCode or a shell";
        let folder = if path.is_empty() { String::new() } else { format!(" in {path}") };
        let sentence = format!("{lead}{folder}. Each agent opens in its own terminal; drag the headers to rearrange.");
        let folder_style = HighlightStyle { color: Some(hsla(p.fg)), ..Default::default() };
        let text = div().text_color(hsla(p.muted)).child(
            StyledText::new(SharedString::from(sentence))
                .with_highlights([(lead.len()..lead.len() + folder.len(), folder_style)]),
        );
        let tips = div().mt_2().flex().flex_col().gap(ui(6.0)).text_size(ui(12.0)).text_color(hsla(p.subtle)).children(
            TIPS.iter().map(|(keys, what)| {
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().w(ui(150.0)).flex().justify_end().gap_1().children(keys.split('+').map(|key| {
                        div()
                            .px(ui(6.0))
                            .rounded(ui(4.0))
                            .border_1()
                            .border_color(hsla(p.line_strong))
                            .bg(hsla(p.raised))
                            .font_family(agentdeck_core::logic::theme::FONT_MONO)
                            .text_size(ui(11.0))
                            .text_color(hsla(p.muted))
                            .child(key.to_owned())
                    })))
                    .child(*what)
            }),
        );
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .bg(hsla(p.gutter))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_5()
                    .max_w(ui(384.0))
                    .child(logos)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(ui(6.0))
                            .child(
                                div()
                                    .text_size(ui(15.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(hsla(p.fg))
                                    .child("Empty session"),
                            )
                            .child(text),
                    )
                    .child(self.add_agent_button("empty", cx))
                    .child(tips),
            )
            .into_any_element()
    }

    /// Left column: linked workspaces, each with its open and archived sessions.
    pub(super) fn sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let deck = self.deck.read(cx);
        let active_ws = deck.state.active_workspace().id.clone();
        let active_tab = deck.state.active_tab_id.clone();
        let mut list = div()
            .id("workspaces")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_2()
            .px_2()
            .pb_3()
            .overflow_y_scroll();
        for ws in &deck.state.workspaces {
            let active = ws.id == active_ws;
            let expanded = !self.collapsed_workspaces.contains(&ws.id);
            let tabs: Vec<_> = deck.state.tabs.iter().filter(|t| t.workspace_id == ws.id).collect();
            let keys: Vec<ActivityKey> = tabs
                .iter()
                .flat_map(|t| t.pane_ids.iter().filter_map(|id| deck.state.panes.get(id)).map(activity_key))
                .collect();
            let waiting = keys
                .iter()
                .filter(|k| **k == ActivityKey::Status(PaneStatus::Waiting))
                .count();
            let failed = keys.contains(&ActivityKey::Status(PaneStatus::Error));
            let (toggle_id, select_id, new_id) = (ws.id.clone(), ws.id.clone(), ws.id.clone());
            let ws_group: SharedString = format!("ws-row-{}", ws.id).into();
            let rename_key = format!("ws:{}", ws.id);
            let rename_field = self.renaming.as_ref().filter(|r| r.0 == rename_key).map(|r| r.1.clone());
            let renaming = rename_field.is_some();
            let path_hint: SharedString = if ws.path.is_empty() {
                "No linked folder: agents start in the last used folder.".into()
            } else {
                ws.path.clone().into()
            };
            let row = div()
                .id(ElementId::Name(ws_group.clone()))
                .group(ws_group.clone())
                .flex()
                .items_center()
                .gap_2()
                .h(ui(32.0))
                .pl_1()
                .pr_1()
                .rounded(ui(6.0))
                .text_color(hsla(if active { p.fg } else { p.muted }))
                .when(!active, |el| el.hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg))))
                .child(
                    div()
                        .id(ElementId::Name(format!("ws-toggle-{}", ws.id).into()))
                        .size(ui(20.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(ui(14.0))
                        .text_color(hsla(p.subtle))
                        .child(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if !this.collapsed_workspaces.remove(&toggle_id) {
                                this.collapsed_workspaces.insert(toggle_id.clone());
                            }
                            cx.notify();
                        })),
                )
                .when_some(rename_field, |el, input| {
                    let cancel = rename_key.clone();
                    el.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .capture_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                                if e.keystroke.key == "escape" && this.renaming.as_ref().is_some_and(|r| r.0 == cancel) {
                                    this.renaming = None;
                                    cx.stop_propagation();
                                    cx.notify();
                                }
                            }))
                            .child(Input::new(&input)),
                    )
                })
                .when(!renaming, |el| el.child(
                    div()
                        .id(ElementId::Name(format!("ws-{}", ws.id).into()))
                        .tooltip(move |window, cx| Tooltip::new(path_hint.clone()).build(window, cx))
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(ui(13.0))
                                .text_color(hsla(if active { p.muted } else { p.subtle }))
                                .child(if active { IconName::FolderOpen } else { IconName::Folder }),
                        )
                        .child(
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .font_weight(FontWeight::MEDIUM)
                                .child(ws.name.clone()),
                        )
                        .children(ws.git.as_ref().map(|git| {
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(ui(12.0))
                                .text_color(hsla(p.subtle))
                                .child(IconName::GitBranch)
                                .child(git.branch.clone().unwrap_or_else(|| "Git".into()))
                        }))
                        .child(div().flex_1())
                        .when(waiting > 0, |el| {
                            el.child(
                                div()
                                    .min_w(ui(14.0))
                                    .px_1()
                                    .rounded_full()
                                    .bg(hsla(p.wait))
                                    .text_size(ui(10.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(hsla(p.on_accent))
                                    .child(waiting.to_string()),
                            )
                        })
                        .when(failed, |el| el.child(div().size(ui(8.0)).rounded_full().bg(hsla(p.bad))))
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            let id = select_id.clone();
                            if e.click_count() == 2 {
                                this.start_rename(&format!("ws:{id}"), window, cx);
                                return;
                            }
                            this.deck.update(cx, |deck, cx| {
                                deck.state.select_workspace(&id);
                                cx.notify();
                            });
                        })),
                ))
                .when(!renaming, |el| {
                    let (template_id, assets_id, graph_id, unlink_id, rename_id) =
                        (ws.id.clone(), ws.id.clone(), ws.id.clone(), ws.id.clone(), ws.id.clone());
                    let linked = !ws.path.is_empty();
                    // The active workspace always shows its actions: new users find them without hovering.
                    let actions = div()
                        .flex()
                        .items_center()
                        .when(!active, |a| a.opacity(0.0).group_hover(ws_group.clone(), |s| s.opacity(1.0)));
                    el.child(actions.child(
                        icon_button(
                            ElementId::Name(format!("ws-new-{}", ws.id).into()),
                            IconName::Plus,
                            format!("New session in {}", ws.name),
                            &p,
                        )
                        .size(ui(22.0)).text_size(ui(13.0)).text_color(hsla(p.subtle))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let id = new_id.clone();
                            this.collapsed_workspaces.remove(&id);
                            this.deck.update(cx, |deck, cx| {
                                deck.state.select_workspace(&id);
                                deck.state.add_tab(None);
                                deck.save(cx);
                                cx.notify();
                            });
                        })),
                    )
                    .child(
                        icon_button(
                            ElementId::Name(format!("ws-template-{}", ws.id).into()),
                            IconName::LayoutTemplate,
                            format!("New session from a template in {}", ws.name),
                            &p,
                        )
                        .size(ui(22.0)).text_size(ui(13.0)).text_color(hsla(p.subtle))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            crate::palette::open_template_picker_for(this.deck.clone(), &template_id, window, cx);
                        })),
                    )
                    .when(linked, |el| {
                        el.child(
                            icon_button(
                                ElementId::Name(format!("ws-assets-{}", ws.id).into()),
                                IconName::Box,
                                "Assets: preview 3D models and ask an agent to change them",
                                &p,
                            )
                            .size(ui(22.0)).text_size(ui(13.0)).text_color(hsla(p.subtle))
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                crate::skills::open_assets(this.deck.clone(), &assets_id, window, cx);
                            })),
                        )
                        .child(
                            icon_button(
                                ElementId::Name(format!("ws-graph-{}", ws.id).into()),
                                IconName::Network,
                                "Graph: map of the project's notes and their links",
                                &p,
                            )
                            .size(ui(22.0)).text_size(ui(13.0)).text_color(hsla(p.subtle))
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                crate::skills::open_graph(this.deck.clone(), &graph_id, window, cx);
                            })),
                        )
                    })
                    .child(
                        icon_button(
                            ElementId::Name(format!("ws-rename-{}", ws.id).into()),
                            IconName::Pencil,
                            format!("Rename {}", ws.name),
                            &p,
                        )
                        .size(ui(22.0)).text_size(ui(13.0)).text_color(hsla(p.subtle))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.start_rename(&format!("ws:{rename_id}"), window, cx);
                        })),
                    )
                    .child(
                        icon_button(
                            ElementId::Name(format!("ws-unlink-{}", ws.id).into()),
                            IconName::Unlink,
                            "Unlink this workspace (no files deleted)",
                            &p,
                        )
                        .size(ui(22.0)).text_size(ui(13.0)).text_color(hsla(p.subtle))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.confirm_unlink(&unlink_id, window, cx);
                        })),
                    ))
                });
            let mut item = div().flex().flex_col().child(row);
            if expanded {
                for tab in tabs {
                    let current = tab.id == active_tab;
                    let archivable = can_archive(&deck.state.tabs, &tab.id);
                    let (open_id, close_id) = (tab.id.clone(), tab.id.clone());
                    let group: SharedString = format!("side-tab-{}", tab.id).into();
                    item = item.child(
                        div()
                            .id(ElementId::Name(group.clone()))
                            .group(group.clone())
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(ui(32.0))
                            .pl(ui(28.0))
                            .pr_1()
                            .rounded(ui(6.0))
                            .when(current, |el| el.bg(hsla(p.raised)).text_color(hsla(p.fg)))
                            .when(!current, |el| {
                                el.text_color(hsla(p.muted)).hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                            })
                            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(tab.name.clone()))
                            .when(!tab.pane_ids.is_empty(), |el| {
                                el.child(
                                    div()
                                        .text_size(ui(12.0))
                                        .text_color(hsla(p.subtle))
                                        .child(tab.pane_ids.len().to_string()),
                                )
                            })
                            .when(archivable, |el| {
                                el.child(
                                    icon_button(
                                        ElementId::Name(format!("side-archive-{}", tab.id).into()),
                                        IconName::Archive,
                                        format!("Close and archive {}", tab.name),
                                        &p,
                                    )
                                    .size(ui(22.0)).text_size(ui(13.0)).text_color(hsla(p.subtle))
                                    .opacity(0.0)
                                    .group_hover(group.clone(), |s| s.opacity(1.0))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.close_session(&close_id, window, cx);
                                    })),
                                )
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let id = open_id.clone();
                                this.deck.update(cx, |deck, cx| {
                                    deck.state.active_tab_id = id;
                                    cx.notify();
                                });
                            })),
                    );
                }
                for session in deck.state.archived.iter().filter(|s| s.workspace_id == ws.id) {
                    let (reopen_id, restore_id, delete_id) = (session.id.clone(), session.id.clone(), session.id.clone());
                    let delete_name = session.name.clone();
                    let group: SharedString = format!("archived-{}", session.id).into();
                    item = item.child(
                        div()
                            .id(ElementId::Name(group.clone()))
                            .group(group.clone())
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(ui(32.0))
                            .pl(ui(28.0))
                            .pr_1()
                            .rounded(ui(6.0))
                            .text_color(hsla(p.subtle))
                            .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.muted)))
                            .child(div().text_size(ui(13.0)).child(IconName::Archive))
                            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(session.name.clone()))
                            .child(
                                div()
                                    .flex()
                                    .opacity(0.0)
                                    .group_hover(group.clone(), |s| s.opacity(1.0))
                                    .child(
                                        icon_button(
                                            ElementId::Name(format!("archived-reopen-{}", session.id).into()),
                                            IconName::ArchiveRestore,
                                            format!("Reopen {}", session.name),
                                            &p,
                                        )
                                        .size(ui(22.0)).text_size(ui(13.0)).text_color(hsla(p.subtle))
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            runtime::reopen_session(&restore_id, cx);
                                        }),
                                    )
                                    .child(
                                        icon_button(
                                            ElementId::Name(format!("archived-delete-{}", session.id).into()),
                                            IconName::Trash,
                                            format!("Delete {}", session.name),
                                            &p,
                                        )
                                        .size(ui(22.0)).text_size(ui(13.0)).text_color(hsla(p.subtle))
                                        .hover(|s| s.text_color(hsla(p.bad)))
                                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                            cx.stop_propagation();
                                            this.confirm_delete_archived(&delete_id, &delete_name, window, cx);
                                        })),
                                    ),
                            )
                            .tooltip({
                                let hint = format!(
                                    "Archived {} · {} agent(s). Click to reopen.",
                                    format_date(session.saved_at),
                                    session.panes.len()
                                );
                                move |window, cx| Tooltip::new(hint.clone()).build(window, cx)
                            })
                            .on_click(move |_, _, cx| runtime::reopen_session(&reopen_id, cx)),
                    );
                }
            }
            list = list.child(item);
        }
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(ui(320.0))
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
                    .child(div().text_size(ui(12.0)).font_weight(FontWeight::MEDIUM).text_color(hsla(p.subtle)).child("Workspaces"))
                    .child(
                        icon_button("link-workspace", IconName::FolderPlus, "Link workspace", &p)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.link_workspace(window, cx))),
                    ),
            )
            .child(list)
            .child(
                div()
                    .id("link-workspace-bottom")
                    .m_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(ui(32.0))
                    .px_2()
                    .rounded(ui(6.0))
                    .text_color(hsla(p.subtle))
                    .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                    .child(div().text_size(ui(16.0)).child(IconName::Plus))
                    .child("Link workspace")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.link_workspace(window, cx))),
            )
            .into_any_element()
    }

    pub(super) fn toasts(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let toasts = &self.deck.read(cx).state.toasts;
        div()
            .absolute()
            .right_4()
            .bottom_4()
            .flex()
            .flex_col()
            .gap_2()
            .children(toasts.iter().map(|t| {
                div()
                    .flex()
                    .items_start()
                    .gap(ui(10.0))
                    .max_w(ui(448.0))
                    .px(ui(14.0))
                    .py(ui(10.0))
                    .rounded(ui(8.0))
                    .border_1()
                    .border_color(hsla(p.line_strong))
                    .bg(hsla(p.raised))
                    .text_size(ui(13.0))
                    .text_color(hsla(p.fg))
                    .child(div().text_size(ui(16.0)).text_color(hsla(p.wait)).child(IconName::CircleAlert))
                    .child(t.message.clone())
            }))
            .into_any_element()
    }
}
