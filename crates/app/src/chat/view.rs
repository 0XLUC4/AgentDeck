//! The chat pane body: message list (user bubbles, markdown answers, collapsible tool rows),
//! busy row, and the input with Send / Stop. Repaints when its transcript changes.

use std::collections::{HashMap, HashSet};

use agentdeck_core::chat::{Item, Tool};
use agentdeck_core::logic::theme::{FONT_MONO, Palette, RADIUS_LG, RADIUS_MD};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

/// Views by pane id, and the task that repaints them on transcript changes.
#[derive(Default)]
struct Views {
    map: HashMap<String, Entity<ChatView>>,
    _repaint: Option<Task<()>>,
}

impl Global for Views {}

/// The chat view of a pane, created on first use.
pub fn view(pane_id: &str, deck: &Entity<Deck>, window: &mut Window, cx: &mut App) -> Entity<ChatView> {
    if cx.default_global::<Views>()._repaint.is_none() {
        let task = cx.spawn(async move |cx| {
            while let Ok(id) = super::CHANGED.1.recv().await {
                cx.update(|cx| {
                    let view = cx.global::<Views>().map.get(&id).cloned();
                    if let Some(view) = view {
                        view.update(cx, |_, cx| cx.notify());
                    }
                });
            }
        });
        cx.global_mut::<Views>()._repaint = Some(task);
    }
    if let Some(view) = cx.global::<Views>().map.get(pane_id) {
        return view.clone();
    }
    let view = cx.new(|cx| ChatView::new(pane_id, deck.clone(), window, cx));
    // Views of closed panes go when a new one comes.
    let alive: HashSet<String> = deck.read(cx).state.panes.keys().cloned().collect();
    let views = &mut cx.global_mut::<Views>().map;
    views.retain(|id, _| alive.contains(id));
    views.insert(pane_id.to_owned(), view.clone());
    view
}

pub struct ChatView {
    pane_id: String,
    deck: Entity<Deck>,
    input: Entity<TextareaState>,
    scroll: ScrollHandle,
    /// Tool rows unfolded to show their output.
    open: HashSet<String>,
    /// Item count at the last render: new items scroll to the bottom.
    seen: usize,
    _input_events: Subscription,
}

impl ChatView {
    fn new(pane_id: &str, deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Message the agent (Enter to send, Shift+Enter for a new line)")
                .auto_grow(1, 8)
                .submit_on_enter(true)
        });
        let sub = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { shift: false, .. } = event {
                this.submit(window, cx);
            }
        });
        Self {
            pane_id: pane_id.to_owned(),
            deck,
            input,
            scroll: ScrollHandle::new(),
            open: HashSet::new(),
            seen: 0,
            _input_events: sub,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        let Some(pane) = self.deck.read(cx).state.panes.get(&self.pane_id).cloned() else { return };
        if text.trim().is_empty() {
            return;
        }
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        super::send(&pane, &text, cx);
        cx.notify();
    }

    fn tool_row(&self, tool: &Tool, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let open = self.open.contains(&tool.id);
        let id = tool.id.clone();
        let (mark, color) = match (&tool.output, tool.error) {
            (None, _) => ("…", p.busy),
            (Some(_), true) => ("✕", p.bad),
            (Some(_), false) => ("✓", p.ok),
        };
        div()
            .flex()
            .flex_col()
            .rounded(ui(RADIUS_MD))
            .border_1()
            .border_color(hsla(p.line))
            .bg(hsla(p.panel))
            .child(
                div()
                    .id(SharedString::from(format!("tool-{}", tool.id)))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(p.hover)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if !this.open.remove(&id) {
                            this.open.insert(id.clone());
                        }
                        cx.notify();
                    }))
                    .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(ui(12.0)))
                    .child(div().flex_shrink_0().font_weight(FontWeight::MEDIUM).child(tool.name.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(FONT_MONO)
                            .text_xs()
                            .text_color(hsla(p.muted))
                            .child(tool.input.clone()),
                    )
                    .child(div().flex_shrink_0().text_color(hsla(color)).child(mark)),
            )
            .when(open, |el| {
                let output = tool.output.clone().unwrap_or_else(|| "Running…".into());
                el.child(
                    div()
                        .px_2()
                        .py_1()
                        .border_t_1()
                        .border_color(hsla(p.line))
                        .font_family(FONT_MONO)
                        .text_xs()
                        .text_color(hsla(if tool.error { p.bad } else { p.muted }))
                        .whitespace_normal()
                        .child(if output.is_empty() { "(no output)".to_owned() } else { output }),
                )
            })
            .into_any_element()
    }

    fn item(&self, index: usize, item: &Item, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        match item {
            Item::User(text) => div()
                .flex()
                .justify_end()
                .child(
                    div()
                        .max_w(relative(0.85))
                        .px_3()
                        .py_2()
                        .rounded(ui(RADIUS_LG))
                        .bg(hsla_a(p.accent, 0.18))
                        .whitespace_normal()
                        .child(text.clone()),
                )
                .into_any_element(),
            Item::Assistant(text) => div()
                .child(TextView::markdown(SharedString::from(format!("chat-{}-{index}", self.pane_id)), text.clone()).selectable(true))
                .into_any_element(),
            Item::Tool(tool) => self.tool_row(tool, p, cx),
            Item::Notice(text) => div().text_xs().text_color(hsla(p.subtle)).child(text.clone()).into_any_element(),
            Item::Error(text) => div()
                .px_3()
                .py_2()
                .rounded(ui(RADIUS_MD))
                .bg(hsla_a(p.bad, 0.12))
                .text_color(hsla(p.bad))
                .whitespace_normal()
                .child(text.clone())
                .into_any_element(),
        }
    }
}

impl Render for ChatView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let transcript = super::transcript(&self.pane_id);
        let exited = self.deck.read(cx).state.panes.get(&self.pane_id).is_none_or(|p| p.exited);
        // New items, or streamed text growing the last one: follow the bottom.
        if transcript.items.len() != self.seen || transcript.busy {
            self.seen = transcript.items.len();
            self.scroll.scroll_to_bottom();
        }
        let items: Vec<AnyElement> = transcript.items.iter().enumerate().map(|(i, item)| self.item(i, item, &p, cx)).collect();
        let empty = items.is_empty();
        let pane_id = self.pane_id.clone();
        let button = |id: &'static str, bg: u32| {
            div()
                .id(id)
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_1()
                .px_3()
                .h(ui(32.0))
                .rounded(ui(RADIUS_MD))
                .bg(hsla(bg))
                .text_color(hsla(p.on_accent))
                .cursor_pointer()
        };
        let action = if transcript.busy {
            button("chat-stop", p.bad)
                .on_click(move |_, _, _| super::interrupt(&pane_id))
                .child(Icon::new(IconName::Square).size(ui(12.0)))
                .child("Stop")
        } else {
            button("chat-send", p.accent)
                .hover(|s| s.bg(hsla(p.accent_hover)))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.submit(window, cx)))
                .child(Icon::new(IconName::ArrowUp).size(ui(14.0)))
                .child("Send")
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(hsla(p.base))
            .text_color(hsla(p.fg))
            .text_size(ui(13.0))
            .child(
                div()
                    .id(SharedString::from(format!("chat-list-{}", self.pane_id)))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .p_4()
                            .when(empty, |el| {
                                el.child(div().text_color(hsla(p.subtle)).child(if exited {
                                    "The agent is stopped. Send a message to start it again."
                                } else {
                                    "Ask the agent anything."
                                }))
                            })
                            .children(items)
                            .when(transcript.busy, |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .text_xs()
                                        .text_color(hsla(p.busy))
                                        .child(div().size(ui(8.0)).rounded_full().bg(hsla(p.busy)))
                                        .child("Working…"),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .p_2()
                    .border_t_1()
                    .border_color(hsla(p.line))
                    .bg(hsla(p.panel))
                    .child(div().flex_1().min_w_0().child(Textarea::new(&self.input)))
                    .child(action),
            )
    }
}
