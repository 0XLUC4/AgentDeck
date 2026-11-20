//! Window chrome around the dock: title bar (TitleBar.tsx), session tab bar with the add-agent
//! button (TabBar.tsx), sidebar of workspaces and sessions (Sidebar.tsx), activity list,
//! empty session (App.tsx) and toasts (Toasts.tsx).

use agentdeck_core::agents::AgentKind;
use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::{ActivityKey, activity_key, session_status};
use agentdeck_core::logic::text::{
    AGENT_KINDS, agent_name, format_cost, format_since, format_tokens, now_ms, split_args,
};
use agentdeck_core::logic::theme::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::component::TitleBar;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::DeckView;
use crate::panes::{agent_logo, last_agent, runtime, spawn_agent, status_color, status_label};
use crate::theme::{hsla, hsla_a, palette, ui};
use agentdeck_core::logic::theme::{RADIUS_LG, RADIUS_MD, RADIUS_SM};

/// Amber count of agents waiting: a true circle for one digit, a pill for more, number centered.
fn waiting_badge(count: usize, p: &Palette) -> Div {
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .h(ui(16.0))
        .min_w(ui(16.0))
        .px(ui(4.0))
        .rounded_full()
        .bg(hsla(p.wait))
        .text_size(ui(10.0))
        .line_height(ui(16.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(hsla(p.on_accent))
        .child(count.to_string())
}

/// IconButton.tsx: a square ghost button with an icon and a tooltip.
pub(super) fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    hint: impl Into<SharedString>,
    p: &Palette,
) -> Stateful<Div> {
    key_button(id, icon, hint, &[], p)
}

/// [`icon_button`] whose tooltip ends with its shortcut as key chips (`&["Ctrl", ","]`).
pub(super) fn key_button(
    id: impl Into<ElementId>,
    icon: IconName,
    hint: impl Into<SharedString>,
    keys: &'static [&'static str],
    p: &Palette,
) -> Stateful<Div> {
    let hover = hsla(p.hover);
    let fg = hsla(p.fg);
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(ui(28.0))
        .rounded(ui(RADIUS_MD))
        .text_size(ui(16.0))
        .text_color(hsla(p.muted))
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(fg))
        .child(icon)
        .tooltip(key_tip(hint, keys))
}

/// Tooltip builder: the hint, then its shortcut as key chips (plain text when `keys` is empty).
pub(super) fn key_tip(
    hint: impl Into<SharedString>,
    keys: &'static [&'static str],
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let hint: SharedString = hint.into();
    move |window, cx| {
        if keys.is_empty() {
            return Tooltip::new(hint.clone()).build(window, cx);
        }
        let hint = hint.clone();
        Tooltip::element(move |_, cx| {
            let p = palette(cx);
            div().flex().items_center().gap_2().child(hint.clone()).child(crate::palette::ui::keys(keys, &p))
        })
        .build(window, cx)
    }
}

/// Small ghost icon button opening a menu, for the sidebar rows.
fn menu_trigger(id: impl Into<ElementId>, icon: IconName, hint: impl Into<SharedString>) -> Button {
    Button::new(id).ghost().xsmall().icon(icon).tooltip(hint)
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

/// Agents still running: every pane whose process has not exited.
fn count_live(keys: impl Iterator<Item = ActivityKey>) -> usize {
    keys.filter(|key| *key != ActivityKey::Exited).count()
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
        // Taskbar hint: "AgentDeck (2)" while agents wait for an answer.
        let title = if waiting > 0 { format!("AgentDeck ({waiting})") } else { "AgentDeck".into() };
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }

        let chevron = || div().text_size(ui(14.0)).text_color(hsla(p.subtle)).child(IconName::ChevronRight);
        let separator = || div().w(px(1.0)).h(ui(20.0)).bg(hsla(p.line));
        let content = div()
            .flex()
            .items_center()
            .size_full()
            .text_size(ui(13.0))
            .child(
                div()
                    .ml_3()
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
        // Shows or hides the left panel (projects, or activity in its place): first thing of the
        // tab strip, right at the sidebar's edge, and still there when the sidebar is hidden.
        let (toggle_icon, toggle_hint) = if self.left_panel_shown(cx) {
            (IconName::PanelLeftClose, "Hide sidebar")
        } else {
            (IconName::PanelLeftOpen, "Show sidebar: projects and sessions")
        };
        let sidebar_toggle = icon_button("tb-sidebar", toggle_icon, toggle_hint, &p)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_sidebar(cx)));
        let deck = self.deck.read(cx);
        let workspace = deck.state.active_workspace().id.clone();
        let active_tab = deck.state.active_tab_id.clone();
        let tabs: Vec<(String, String)> = deck
            .state
            .tabs
            .iter()
            .filter(|t| t.workspace_id == workspace)
            .map(|t| (t.id.clone(), t.name.clone()))
            .collect();

        let mut list = div()
            .id("session-tabs")
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .h_full()
            .overflow_x_scroll();
        for (id, name) in tabs {
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
                    .child(
                        icon_button(
                            ElementId::Name(format!("session-close-{id}").into()),
                            IconName::X,
                            "Delete this session (close its agents first)",
                            &p,
                        )
                        .size(ui(22.0))
                        .when(!active, |b| b.opacity(0.0).group_hover(group.clone(), |s| s.opacity(1.0)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.close_session(&close, window, cx);
                        })),
                    )
                    .cursor_pointer()
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
            .child(sidebar_toggle)
            .child(list)
            .child(
                div()
                    .id("new-session")
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap(ui(6.0))
                    .h(ui(28.0))
                    .px_2()
                    .rounded(ui(RADIUS_MD))
                    .text_color(hsla(p.muted))
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                    .child(div().text_size(ui(15.0)).child(IconName::Plus))
                    .child(div().whitespace_nowrap().child("New tab"))
                    .tooltip(|window, cx| {
                        Tooltip::new("New session (Enter), or search open tabs, files, URLs, agents (Ctrl+T)").build(window, cx)
                    })
                    // The omnibox (palette module); its first row, "New session", is selected.
                    .on_click(|_: &ClickEvent, window, cx| window.dispatch_action(Box::new(crate::palette::OpenNewTab), cx)),
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

    /// Deletes a session tab after confirmation. Refused (with a popup) while agents are open in it.
    pub(super) fn close_session(&mut self, tab_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let found = self.deck.read(cx).state.tabs.iter().find(|t| t.id == tab_id).map(|t| (t.name.clone(), t.pane_ids.len()));
        let Some((name, agents)) = found else { return };
        if agents > 0 {
            let detail = format!("{agents} agent(s) are still open in it. Close them first, then delete the session.");
            let answer = window.prompt(PromptLevel::Info, &format!("Can't delete \"{name}\""), Some(&detail), &["OK"], cx);
            cx.spawn(async move |_, _| {
                let _ = answer.await;
            }).detach();
            return;
        }
        let answer = window.prompt(PromptLevel::Warning, &format!("Delete \"{name}\"?"), None, &["Delete", "Cancel"], cx);
        let id = tab_id.to_owned();
        cx.spawn(async move |_, cx| {
            if answer.await.ok() == Some(0) {
                cx.update(|cx| runtime::delete_session(&id, cx));
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
                .rounded(ui(RADIUS_MD))
                .px_3()
                .child(div().text_size(ui(15.0)).child(IconName::Plus))
                .child("Add agent")
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_picker(window, cx)))
                .into_any_element();
        }
        let menu_open = self.add_menu_open == Some(key);
        if menu_open {
            crate::browser::popup_shown();
        }
        let mut menu = div()
            .absolute()
            .top(ui(36.0))
            .right_0()
            .w(ui(192.0))
            .flex()
            .flex_col()
            .p_1()
            .rounded(ui(RADIUS_LG))
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
                    .rounded(ui(RADIUS_MD))
                    .text_color(hsla(p.fg))
                    .hover(|s| s.bg(hsla(p.hover)))
                    .child(agent_logo(kind, 15.0, None, &p))
                    .child(agent_name(kind))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.add_agent(kind, cx))),
            );
        }
        menu = menu.child(div().my_1().h(px(1.0)).bg(hsla(p.line))).child(
            div()
                .id(ElementId::Name(format!("add-{key}-worktree").into()))
                .flex()
                .items_center()
                .gap(ui(10.0))
                .h(ui(32.0))
                .px_2()
                .rounded(ui(RADIUS_MD))
                .text_color(hsla(p.muted))
                .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                .child(div().text_size(ui(15.0)).child(IconName::GitBranch))
                .child("Create worktree...")
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.add_menu_open = None;
                    crate::worktree::open_create_worktree(None, window, cx);
                })),
        );
        menu = menu.child(
            div()
                .id(ElementId::Name(format!("add-{key}-more").into()))
                .flex()
                .items_center()
                .gap(ui(10.0))
                .h(ui(32.0))
                .px_2()
                .rounded(ui(RADIUS_MD))
                .text_color(hsla(p.muted))
                .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                .child(div().text_size(ui(15.0)).child(IconName::SlidersHorizontal))
                .child("More options...")
                .cursor_pointer()
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
                    .rounded_l(ui(RADIUS_MD))
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
                    .rounded_r(ui(RADIUS_MD))
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
        spawn_agent(kind, &path, split_args(&defaults.args), defaults.worktree, None, cx);
        cx.notify();
    }

    fn link_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::orchestrator::open_link_workspace(self.deck.clone(), window, cx);
    }

    /// A new empty session in the workspace, which becomes the active one (and gets expanded).
    fn new_session(&mut self, workspace_id: &str, cx: &mut Context<Self>) {
        self.collapsed_workspaces.remove(workspace_id);
        let id = workspace_id.to_owned();
        self.deck.update(cx, |deck, cx| {
            deck.state.select_workspace(&id);
            deck.state.add_tab(None);
            deck.save(cx);
            cx.notify();
        });
    }

    /// Unlinks a workspace after confirmation: its sessions are closed, no files are deleted.
    fn confirm_unlink(&mut self, workspace_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let state = &self.deck.read(cx).state;
        let Some(name) = state.workspaces.iter().find(|w| w.id == workspace_id).map(|w| w.name.clone()) else {
            return;
        };
        let count = state.tabs.iter().filter(|t| t.workspace_id == workspace_id).count();
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
        let text = div().text_size(ui(12.0)).text_color(hsla(p.subtle)).child(
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
                            .rounded(ui(RADIUS_SM))
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
                            .text_size(ui(18.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(hsla(p.fg))
                            .child("Start an agent"),
                    )
                    .child(self.add_agent_button("empty", cx))
                    .child(text)
                    .child(div().w_full().h(px(1.0)).bg(hsla(p.line)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(
                                div()
                                    .text_size(ui(12.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(hsla(p.muted))
                                    .child("Shortcuts"),
                            )
                            .child(tips),
                    ),
            )
            .into_any_element()
    }

    /// Left column: linked workspaces, each with its sessions.
    pub(super) fn sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let deck = self.deck.read(cx);
        let active_ws = deck.state.active_workspace().id.clone();
        let active_tab = deck.state.active_tab_id.clone();
        let now = now_ms();
        let weak = cx.entity().downgrade();
        let all_keys: Vec<ActivityKey> = deck.state.panes.values().map(activity_key).collect();
        let waiting_all = all_keys.iter().filter(|k| **k == ActivityKey::Status(PaneStatus::Waiting)).count();
        let failed = all_keys.contains(&ActivityKey::Status(PaneStatus::Error));
        let live = count_live(all_keys.iter().copied());
        let wall_on = crate::orchestrator::wall_open(cx);
        // The open page (bots, automations, plugins...), marked in the navigation.
        let kind = crate::skills::page_kind(cx);
        // Navigation rows above the projects: icon, label, then an optional badge.
        let nav_row = |id: &'static str, icon: IconName, label: &'static str, on: bool| {
            div()
                .id(id)
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .h(ui(28.0))
                .px_2()
                .rounded(ui(RADIUS_MD))
                .text_color(hsla(if on { p.fg } else { p.muted }))
                .when(on, |el| el.bg(hsla(p.raised)))
                .cursor_pointer()
                .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                .child(div().flex_shrink_0().text_size(ui(15.0)).child(icon))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(label))
        };
        let count = |n: usize| div().flex_shrink_0().text_size(ui(12.0)).text_color(hsla(p.subtle)).child(n.to_string());
        let search = div()
            .id("side-search")
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .h(ui(30.0))
            .mb_1()
            .px_2()
            .rounded(ui(RADIUS_MD))
            .border_1()
            .border_color(hsla(p.line))
            .bg(hsla(p.base))
            .text_color(hsla(p.subtle))
            .cursor_pointer()
            .hover(|s| s.border_color(hsla(p.line_strong)).text_color(hsla(p.muted)))
            .child(div().flex_shrink_0().text_size(ui(14.0)).child(IconName::Search))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child("Search history"))
            .child(crate::palette::ui::keys(&["Ctrl", "Shift", "F"], &p))
            .tooltip(key_tip("Search every agent's past prompts and answers", &["Ctrl", "Shift", "F"]))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::history::SearchHistory), cx));
        let activity = nav_row("side-activity", IconName::Activity, "Activity", self.activity_open)
            .tooltip(key_tip("Activity of all agents. Next waiting agent: Ctrl+Shift+J", &["Ctrl", "Shift", "A"]))
            .when(failed, |el| el.child(div().flex_shrink_0().size(ui(8.0)).rounded_full().bg(hsla(p.bad))))
            .when(waiting_all > 0, |el| el.child(waiting_badge(waiting_all, &p)))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_activity(cx)));
        let wall_hint = if wall_on { "Exit wall" } else { "Wall: every agent at a glance" };
        let wall = nav_row("side-wall", IconName::LayoutGrid, "Wall", wall_on)
            .tooltip(key_tip(wall_hint, &["Ctrl", "Shift", "W"]))
            .when(live > 0, |el| el.child(count(live)))
            .on_click(|_, _, cx| crate::orchestrator::toggle_wall(cx));
        let bots = nav_row("side-bots", IconName::Bot, "Bots", kind == Some("bots"))
            .tooltip(key_tip("Bots: missions, memory and tasks", &[]))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| crate::bots::open(this.deck.clone(), window, cx)));
        let automations = nav_row("side-automations", IconName::RotateCcwClock, "Automations", kind == Some("automations"))
            .tooltip(key_tip("Automations: prompts that run on a schedule", &[]))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| crate::automations::open(this.deck.clone(), window, cx)));
        let orchestrator = nav_row("side-orchestrator", IconName::Crown, "Orchestrator", false)
            .tooltip(key_tip("Start an orchestrator: an agent that starts workers, sends them tasks and gathers their results", &[]))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| crate::orchestrator::create_orchestrator(&this.deck, cx)));
        // Projects header: one "+" for everything that adds a session, a worktree or a project.
        let add_menu = {
            let weak = weak.clone();
            menu_trigger("projects-add", IconName::Plus, "New session, worktree or project").dropdown_menu_with_anchor(
                Anchor::TopRight,
                move |menu, _, _| {
                    let (session, worktree, link) = (weak.clone(), weak.clone(), weak.clone());
                    menu.item(PopupMenuItem::new("New session").icon(IconName::Plus).on_click(move |_, _, cx| {
                        session
                            .update(cx, |this, cx| {
                                let id = this.deck.read(cx).state.active_workspace().id.clone();
                                this.new_session(&id, cx);
                            })
                            .ok();
                    }))
                    .item(PopupMenuItem::new("Create worktree…").icon(IconName::GitBranchPlus).on_click(
                        move |_, window, cx| {
                            let Some(project_id) =
                                worktree.upgrade().map(|this| this.read(cx).deck.read(cx).state.active_workspace().id.clone())
                            else {
                                return;
                            };
                            let prefill = crate::worktree::WorktreePrefill { project_id: Some(project_id), source: None };
                            crate::worktree::open_create_worktree(Some(prefill), window, cx);
                        },
                    ))
                    .separator()
                    .item(PopupMenuItem::new("Link project…").icon(IconName::FolderPlus).on_click(move |_, window, cx| {
                        link.update(cx, |this, cx| this.link_workspace(window, cx)).ok();
                    }))
                },
            )
        };
        // The agent finished a task already: its check stays until it works again.
        let finished = |pane_id: &String| deck.tasks.get(pane_id).is_some_and(|log| !log.history.is_empty());
        let mut list = div()
            .id("workspaces")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_4()
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
                .relative()
                .flex()
                .items_center()
                .gap_2()
                .h(ui(30.0))
                .pl_1()
                .pr_2()
                .rounded(ui(RADIUS_MD))
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
                        .cursor_pointer()
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
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .font_weight(FontWeight::MEDIUM)
                                .child(ws.name.clone()),
                        )
                        .children(ws.git.as_ref().map(|git| {
                            div()
                                .flex()
                                .flex_shrink_0()
                                .max_w(ui(110.0))
                                .items_center()
                                .gap_1()
                                .text_size(ui(12.0))
                                .text_color(hsla(p.subtle))
                                .child(div().flex_shrink_0().child(IconName::GitBranch))
                                .child(
                                    div()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(git.branch.clone().unwrap_or_else(|| "Git".into())),
                                )
                        }))
                        .child(div().flex_1())
                        .when(waiting > 0, |el| {
                            el.child(waiting_badge(waiting, &p))
                        })
                        .cursor_pointer()
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
                    // Two actions, shown on hover over the row's right end: the name keeps the room.
                    let linked = !ws.path.is_empty();
                    // Git of the project's agent: the active one if it is in this project, else its first.
                    let in_project = |id: &String| tabs.iter().any(|t| t.pane_ids.contains(id));
                    let git_pane = deck
                        .state
                        .active_pane_id
                        .clone()
                        .filter(in_project)
                        .or_else(|| tabs.iter().find_map(|t| t.pane_ids.first().cloned()));
                    let (menu_id, menu_deck, menu_weak) = (ws.id.clone(), self.deck.clone(), weak.clone());
                    let more = menu_trigger(
                        ElementId::Name(format!("ws-more-{}", ws.id).into()),
                        IconName::Ellipsis,
                        format!("More actions for {}", ws.name),
                    )
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                        let page = |open: fn(Entity<crate::state::Deck>, &str, &mut Window, &mut App)| {
                            let (deck, id) = (menu_deck.clone(), menu_id.clone());
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| open(deck.clone(), &id, window, cx)
                        };
                        let (template_deck, template_id) = (menu_deck.clone(), menu_id.clone());
                        let worktree_id = menu_id.clone();
                        let (git_weak, git_pane) = (menu_weak.clone(), git_pane.clone());
                        let (rename_weak, rename_id) = (menu_weak.clone(), menu_id.clone());
                        let (unlink_weak, unlink_id) = (menu_weak.clone(), menu_id.clone());
                        menu.item(PopupMenuItem::new("New session from a template…").icon(IconName::LayoutTemplate).on_click(
                            move |_, window, cx| {
                                crate::palette::open_template_picker_for(template_deck.clone(), &template_id, window, cx)
                            },
                        ))
                        .item(PopupMenuItem::new("Create worktree…").icon(IconName::GitBranchPlus).on_click(
                            move |_, window, cx| {
                                let prefill =
                                    crate::worktree::WorktreePrefill { project_id: Some(worktree_id.clone()), source: None };
                                crate::worktree::open_create_worktree(Some(prefill), window, cx);
                            },
                        ))
                        .separator()
                        .item(PopupMenuItem::new("Files").icon(IconName::Folder).disabled(!linked).on_click(page(crate::skills::open_files)))
                        .item(PopupMenuItem::new("Notes").icon(IconName::Network).disabled(!linked).on_click(page(crate::skills::open_graph)))
                        .item(PopupMenuItem::new("Assets").icon(IconName::Box).disabled(!linked).on_click(page(crate::skills::open_assets)))
                        .item(PopupMenuItem::new("Git").icon(IconName::GitBranch).disabled(git_pane.is_none()).on_click(
                            move |_, window, cx| {
                                if let Some(pane) = git_pane.clone() {
                                    git_weak.update(cx, |this, cx| this.open_git(&pane, window, cx)).ok();
                                }
                            },
                        ))
                        .separator()
                        .item(PopupMenuItem::new("Rename").icon(IconName::Pencil).on_click(move |_, window, cx| {
                            // After the menu closes: it gives focus back, which would blur the name field.
                            let (weak, id) = (rename_weak.clone(), rename_id.clone());
                            window.defer(cx, move |window, cx| {
                                weak.update(cx, |this, cx| this.start_rename(&format!("ws:{id}"), window, cx)).ok();
                            });
                        }))
                        .item(PopupMenuItem::new("Unlink…").icon(IconName::Unlink).on_click(move |_, window, cx| {
                            unlink_weak.update(cx, |this, cx| this.confirm_unlink(&unlink_id, window, cx)).ok();
                        }))
                    });
                    let actions = div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_1()
                        .flex()
                        .items_center()
                        .gap(ui(2.0))
                        .pl_1()
                        .rounded(ui(RADIUS_MD))
                        .bg(hsla(if active { p.panel } else { p.hover }))
                        .opacity(0.0)
                        .group_hover(ws_group.clone(), |s| s.opacity(1.0))
                        .child(
                            menu_trigger(
                                ElementId::Name(format!("ws-new-{}", ws.id).into()),
                                IconName::Plus,
                                format!("New session in {}", ws.name),
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.new_session(&new_id, cx))),
                        )
                        .child(more);
                    el.child(actions)
                });
            let mut item = div().flex().flex_col().child(row);
            if expanded {
                for tab in tabs {
                    let current = tab.id == active_tab;
                    let (open_id, close_id) = (tab.id.clone(), tab.id.clone());
                    let group: SharedString = format!("side-tab-{}", tab.id).into();
                    let session = session_status(&tab.pane_ids, &deck.state.panes);
                    let mark = session.and_then(|s| status_mark(s, 13.0, &p, tab.pane_ids.iter().any(|id| finished(id))));
                    let status = session.filter(|s| *s != PaneStatus::Idle);
                    // One block per session: its line, then its agents. The current one is a card.
                    // Same box for every session, so selecting one only colors it: the text stays put.
                    let boxed = div().flex().flex_col().ml(ui(16.0)).my_1().p(ui(3.0)).rounded(ui(RADIUS_LG)).border_1();
                    let boxed = boxed.border_color(hsla_a(p.line_strong, 0.0));
                    let mut block = boxed.when(current, |el| {
                        el.border_color(hsla(p.line_strong)).bg(hsla(p.raised))
                    });
                    block = block.child(
                        div()
                            .id(ElementId::Name(group.clone()))
                            .group(group.clone())
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(ui(30.0))
                            .pl_2()
                            .pr_1()
                            .rounded(ui(RADIUS_MD))
                            .text_color(hsla(p.muted))
                            .when(!current, |el| el.hover(|s| s.bg(hsla(p.hover))))
                            .when_some(mark, |el, mark| el.child(mark))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(hsla(p.fg))
                                    .child(tab.name.clone()),
                            )
                            .when_some(
                                status.filter(|s| status_mark(*s, 13.0, &p, true).is_none()),
                                |el, status| {
                                    // "Working", "Waiting"...: the first word of the status label.
                                    let label = status_label(status).split(' ').next().unwrap_or_default();
                                    el.child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(ui(12.0))
                                            .text_color(hsla(status_color(status, &p)))
                                            .child(label),
                                    )
                                },
                            )
                            .when(!tab.pane_ids.is_empty(), |el| {
                                el.child(
                                    div()
                                        .text_size(ui(12.0))
                                        .text_color(hsla(p.subtle))
                                        .child(tab.pane_ids.len().to_string()),
                                )
                            })
                            .child(
                                icon_button(
                                    ElementId::Name(format!("side-delete-{}", tab.id).into()),
                                    IconName::Trash,
                                    format!("Delete {}", tab.name),
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
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let id = open_id.clone();
                                this.deck.update(cx, |deck, cx| {
                                    deck.state.active_tab_id = id;
                                    cx.notify();
                                });
                            })),
                    );
                    // The session's agents, one compact row each.
                    for pane in tab.pane_ids.iter().filter_map(|id| deck.state.panes.get(id)) {
                        let key = activity_key(pane);
                        let focused = deck.state.active_pane_id.as_deref() == Some(pane.id.as_str());
                        let subject = pane
                            .topic
                            .clone()
                            .filter(|topic| !topic.is_empty())
                            .or_else(|| Some(pane.title.clone()).filter(|title| !title.is_empty()))
                            .unwrap_or_else(|| agent_name(pane.agent).to_owned());
                        // Working and Done read as a mark left of the logo; other states keep their word.
                        let mark = match key {
                            ActivityKey::Status(status) => status_mark(status, 12.0, &p, finished(&pane.id)),
                            ActivityKey::Exited => None,
                        };
                        let status_tone = match key {
                            ActivityKey::Status(PaneStatus::Idle) => None,
                            _ if mark.is_some() => None,
                            ActivityKey::Status(status) => Some(status_color(status, &p)),
                            ActivityKey::Exited => Some(p.subtle),
                        };
                        let model = crate::chat::model_arg(&pane.args);
                        let jump_id = pane.id.clone();
                        block = block.child(
                            div()
                                .id(ElementId::Name(format!("side-agent-{}", pane.id).into()))
                                .flex()
                                .items_center()
                                .gap(ui(6.0))
                                .h(ui(24.0))
                                .pl(ui(22.0))
                                .pr_2()
                                .rounded(ui(RADIUS_MD))
                                .text_size(ui(12.0))
                                .text_color(hsla(if focused { p.fg } else { p.muted }))
                                .when(!focused, |el| el.hover(|s| s.bg(hsla(p.hover))))
                                .child(div().flex().flex_shrink_0().justify_center().w(ui(12.0)).children(mark))
                                .child(agent_logo(pane.agent, 12.0, None, &p))
                                .when(pane.ssh.is_some(), |el| {
                                    el.child(
                                        div()
                                            .id(ElementId::Name(format!("side-remote-{}", pane.id).into()))
                                            .flex_shrink_0()
                                            .text_size(ui(11.0))
                                            .text_color(hsla(p.subtle))
                                            .child(IconName::Server)
                                            .tooltip(move |window, cx| Tooltip::new("Runs on a remote host").build(window, cx)),
                                    )
                                })
                                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(subject))
                                .when_some(model, |el, model| {
                                    el.child(
                                        div()
                                            .flex_shrink_0()
                                            .max_w(ui(90.0))
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .font_family(agentdeck_core::logic::theme::FONT_MONO)
                                            .text_size(ui(11.0))
                                            .text_color(hsla(p.subtle))
                                            .child(model),
                                    )
                                })
                                .when_some(status_tone, |el, tone| {
                                    el.child(div().flex_shrink_0().text_color(hsla(tone)).child(activity_label(key)))
                                })
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_color(hsla(p.subtle))
                                        .child(format_since(now - pane.status_since)),
                                )
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    cx.stop_propagation();
                                    // Same as `layout::jump_to_pane`, which cannot re-enter the shell from its own listener.
                                    this.jump(&jump_id, cx);
                                })),
                        );
                    }
                    item = item.child(block);
                }
            }
            list = list.child(item);
        }
        // Bottom bar: the app-wide tools, icons with tooltips.
        let plugins_on = kind == Some("plugins");
        let bottom = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(ui(2.0))
            .h(ui(40.0))
            .px_2()
            .border_t_1()
            .border_color(hsla(p.line))
            .child(
                key_button("side-settings", IconName::Settings, "Settings", &["Ctrl", ","], &p)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::settings::OpenSettings), cx)),
            )
            .child(
                icon_button("side-plugins", IconName::Blocks, "Plugins: skills for every agent", &p)
                    .when(plugins_on, |el| el.bg(hsla(p.raised)).text_color(hsla(p.fg)))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| crate::plugins::open(this.deck.clone(), window, cx))),
            )
            .child(
                icon_button("side-usage", IconName::Gauge, "Usage and cost of the agents", &p)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::history::OpenUsage), cx)),
            )
            .child(
                key_button("side-history", IconName::Clock, "Search history", &["Ctrl", "Shift", "F"], &p)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::history::SearchHistory), cx)),
            )
            .child(
                icon_button("side-journal", IconName::BookOpenText, "Journal of the day", &p)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::history::OpenJournal), cx)),
            )
            .child(div().flex_1())
            .child(
                key_button("side-commands", IconName::Command, "Every command", &["Ctrl", "K"], &p)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::palette::OpenPalette), cx)),
            );
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
                    .flex_col()
                    .flex_shrink_0()
                    .gap(ui(2.0))
                    .px_2()
                    .pt_2()
                    .child(search)
                    .child(activity)
                    .child(wall)
                    .child(bots)
                    .child(automations)
                    .child(orchestrator),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .h(ui(36.0))
                    .mt_2()
                    .pl_3()
                    .pr_2()
                    .child(
                        div()
                            .text_size(ui(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(hsla(p.subtle))
                            .child("PROJECTS"),
                    )
                    .child(add_menu),
            )
            .child(list)
            .child(bottom)
            .into_any_element()
    }

    /// Bottom strip: today's usage per agent on the left; subscription usage, agents waiting, working /
    /// live, ready for review, projects with live agents, CLI updates, AgentDeck's RAM, phone access
    /// and the version on the right.
    pub(super) fn status_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let count = |wanted: ActivityKey| {
            self.deck.read(cx).state.panes.values().filter(|pane| activity_key(pane) == wanted).count()
        };
        let live = count_live(self.deck.read(cx).state.panes.values().map(activity_key));
        let working = count(ActivityKey::Status(PaneStatus::Working));
        let waiting = count(ActivityKey::Status(PaneStatus::Waiting));
        let review = count(ActivityKey::Status(PaneStatus::Done));
        let projects = {
            let state = &self.deck.read(cx).state;
            let live_pane = |id: &String| state.panes.get(id).is_some_and(|pane| activity_key(pane) != ActivityKey::Exited);
            let mut ids: Vec<&str> =
                state.tabs.iter().filter(|tab| tab.pane_ids.iter().any(live_pane)).map(|tab| tab.workspace_id.as_str()).collect();
            ids.sort_unstable();
            ids.dedup();
            ids.len()
        };
        let phone = self.deck.read(cx).settings.remote.enabled;
        let separator = || div().w(px(1.0)).h(ui(12.0)).flex_shrink_0().bg(hsla(p.line));
        let item = || div().flex().flex_shrink_0().items_center().gap(ui(6.0)).whitespace_nowrap();

        // Today's usage, one group per agent that spent something: logo, share of the day, tokens and cost.
        let mut left = div().flex().min_w_0().items_center().gap_4().overflow_hidden();
        if let Some(usage) = &self.usage_today {
            let day_tokens = usage.total.tokens.max(1) as f32;
            for agent in usage.by_agent.iter().filter(|agent| agent.totals.tokens > 0) {
                // The usage key is a short id ("claude", "codex"): the start of the agent's display name.
                let kind = AGENT_KINDS.into_iter().find(|kind| agent_name(*kind).to_lowercase().starts_with(&agent.key));
                let share = (agent.totals.tokens as f32 / day_tokens).min(1.0);
                left = left.child(
                    item()
                        .children(kind.map(|kind| agent_logo(kind, 12.0, None, &p)))
                        .child(
                            div()
                                .w(ui(48.0))
                                .h(ui(4.0))
                                .rounded_full()
                                .overflow_hidden()
                                .bg(hsla(p.raised))
                                .child(div().h_full().w(relative(share)).rounded_full().bg(hsla(p.muted))),
                        )
                        .child(format!("{} · {}", format_tokens(agent.totals.tokens as f64), format_cost(agent.totals.cost))),
                );
            }
        }

        let mut right: Vec<AnyElement> = Vec::new();
        right.extend(crate::history::subscription_item(&self.deck, cx));
        if waiting > 0 {
            right.push(
                item()
                    .id("status-waiting")
                    .text_color(hsla(p.wait))
                    .cursor_pointer()
                    .child(format!("{waiting} waiting"))
                    .tooltip(|window, cx| Tooltip::new("Jump to the agent waiting for you (Ctrl+Shift+J)").build(window, cx))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.next_waiting(cx)))
                    .into_any_element(),
            );
        }
        if live > 0 {
            right.push(
                item()
                    .id("status-working")
                    .child(format!("{working}/{live} working"))
                    .tooltip(|window, cx| Tooltip::new("Agents working / live terminals").build(window, cx))
                    .into_any_element(),
            );
        }
        if review > 0 {
            right.push(item().text_color(hsla(p.ok)).child(format!("{review} to review")).into_any_element());
        }
        if projects > 1 {
            right.push(item().child(format!("{projects} projects")).into_any_element());
        }
        right.extend(crate::history::cli_updates_item(&self.deck, cx));
        if let Some(mb) = self.ram_mb {
            right.push(
                item()
                    .id("status-ram")
                    .child(div().text_size(ui(13.0)).child(IconName::MemoryStick))
                    .child(format!("{mb} MB"))
                    .tooltip(|window, cx| Tooltip::new("Memory used by AgentDeck (agents not included)").build(window, cx))
                    .into_any_element(),
            );
        }
        if phone {
            right.push(item().child("Phone on").into_any_element());
        }
        right.extend(crate::hosts::status_item(cx));
        right.push(item().text_color(hsla(p.subtle)).child(concat!("v", env!("CARGO_PKG_VERSION"))).into_any_element());
        let last = right.len() - 1;
        let right = div().flex().flex_shrink_0().items_center().gap_2().children(
            right.into_iter().enumerate().flat_map(|(index, element)| {
                [Some(element), (index < last).then(|| separator().into_any_element())].into_iter().flatten()
            }),
        );
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .gap_3()
            .h(ui(24.0))
            .px_3()
            .border_t_1()
            .border_color(hsla(p.line))
            .bg(hsla(p.panel))
            .text_size(ui(12.0))
            .text_color(hsla(p.muted))
            .child(left)
            .child(right)
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
                    .rounded(ui(RADIUS_LG))
                    .border_1()
                    .border_color(hsla(p.line_strong))
                    .bg(hsla(p.raised))
                    .text_size(ui(13.0))
                    .text_color(hsla(p.fg))
                    .child(div().flex_shrink_0().text_size(ui(16.0)).text_color(hsla(p.wait)).child(IconName::CircleAlert))
                    // The text takes the width left by the icon and wraps inside the toast.
                    .child(div().min_w_0().flex_1().overflow_hidden().whitespace_normal().child(t.message.clone()))
            }))
            .into_any_element()
    }
}

/// Sidebar mark of a session or agent: a spinner while it works, a green check once done; `None`
/// for the states that keep their word (waiting, error...).
/// Status glyph: spinner (working), amber ? (waiting), red dot (error), green check (done, or idle
/// after a finished task). An idle agent that never finished a task gets nothing.
fn status_mark(status: PaneStatus, size: f32, p: &Palette, finished: bool) -> Option<AnyElement> {
    let check = || div().text_size(ui(size)).text_color(hsla(p.ok)).child(IconName::CircleCheck).into_any_element();
    use gpui_kit::component::Sizable as _;
    match status {
        PaneStatus::Working => {
            Some(gpui_kit::component::spinner::Spinner::new().icon(gpui_kit::assets::IconName::LoaderCircle).with_size(px(size)).color(hsla(p.wait)).into_any_element())
        }
        PaneStatus::Done => Some(check()),
        // Waiting on the user (permission, question): the glyph that asks to look here.
        PaneStatus::Waiting => {
            Some(div().text_size(ui(size)).text_color(hsla(p.wait)).child(IconName::CircleQuestionMark).into_any_element())
        }
        PaneStatus::Error => Some(dot(size, p.bad)),
        PaneStatus::Idle => finished.then(check),
    }
}

/// Small round status dot, centered in a `size` box so it lines up with the other glyphs.
fn dot(size: f32, color: u32) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(ui(size))
        .child(div().size(ui(size * 0.5)).rounded_full().bg(hsla(color)))
        .into_any_element()
}
