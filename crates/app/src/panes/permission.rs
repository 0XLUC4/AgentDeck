//! Pending tool approvals (components/Permission): the card that shows exactly what will run, with
//! Allow / Always allow / Auto-allow here / Deny (with a reason) / Terminal, and the floating banner
//! that shows the oldest request of the current workspace while the activity panel is closed.
//! Requests live in `Deck::permissions` (filled from the hook server by state.rs).

use std::collections::HashMap;

use agentdeck_core::hooks_server::permissions::{PermissionRequest, answer_permission, answer_questions};
use agentdeck_core::logic::permissions::rule_for;
use agentdeck_core::logic::settings::AutoAllowRule;
use agentdeck_core::logic::theme::{FONT_MONO, RADIUS_LG, RADIUS_MD};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::header::display_title;
use super::logo::agent_logo;
use super::runtime::{self, focus_pane};
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};

fn tool_icon(tool: &str) -> IconName {
    match tool {
        "Bash" | "PowerShell" => IconName::SquareTerminal,
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => IconName::FilePen,
        "Read" => IconName::FileText,
        "WebFetch" | "WebSearch" => IconName::Globe,
        t if t.starts_with("mcp__") => IconName::Plug,
        _ => IconName::ShieldQuestionMark,
    }
}

fn is_shell(tool: &str) -> bool {
    tool == "Bash" || tool == "PowerShell"
}

/// Sends the user's answer (`allow` | `allow_always` | `deny` | `ask`); the request leaves the UI at
/// once (the backend confirms with `agent-permission-resolved`).
pub fn answer(request: &PermissionRequest, decision: &str, message: Option<String>, cx: &mut App) {
    let Some(deck) = runtime::deck(cx) else { return };
    let id = request.request_id.clone();
    deck.update(cx, |d, cx| {
        d.permissions.retain(|r| r.request_id != id);
        if let Err(e) = answer_permission(id.clone(), decision.to_owned(), message) {
            d.toast(format!("Answer not sent ({e}): reply in the terminal."), cx);
        }
        cx.notify();
    });
}

/// Sends the picked options of an `AskUserQuestion` (question -> labels); the request leaves the UI at once.
fn send_answers(request: &PermissionRequest, picked: &[Vec<usize>], cx: &mut App) {
    let Some(deck) = runtime::deck(cx) else { return };
    let answers: HashMap<String, String> = request
        .questions
        .iter()
        .zip(picked)
        .map(|(q, picks)| {
            let labels: Vec<&str> = picks.iter().filter_map(|&i| q.options.get(i)).map(|o| o.label.as_str()).collect();
            (q.question.clone(), labels.join(", "))
        })
        .collect();
    let id = request.request_id.clone();
    deck.update(cx, |d, cx| {
        d.permissions.retain(|r| r.request_id != id);
        if let Err(e) = answer_questions(&id, &answers) {
            d.toast(format!("Answer not sent ({e}): reply in the terminal."), cx);
        }
        cx.notify();
    });
}

/// Adds an AgentDeck rule for this request in the pane's folder (Settings > Agents), then allows it.
pub fn auto_allow_here(request: &PermissionRequest, cx: &mut App) {
    let Some(deck) = runtime::deck(cx) else { return };
    let Some(cwd) = deck.read(cx).state.panes.get(&request.pane_id).map(|p| p.cwd.clone()) else {
        return;
    };
    let Some(rule) = rule_for(&request.tool_name, &request.detail) else { return };
    deck.update(cx, |d, cx| {
        let known = d.settings.auto_allow.iter().any(|r| r.rule == rule && r.folder == cwd);
        if !known {
            d.update_settings(|s| s.auto_allow.push(AutoAllowRule { rule, folder: cwd }), cx);
        }
    });
    answer(request, "allow", None, cx);
}

/// Shows the pane: selects its workspace and session tab, then focuses it.
pub fn jump_to(pane_id: &str, cx: &mut App) {
    let Some(deck) = runtime::deck(cx) else { return };
    deck.update(cx, |d, cx| {
        let tab = d.state.tabs.iter().find(|t| t.pane_ids.iter().any(|p| p == pane_id)).cloned();
        if let Some(tab) = tab {
            d.state.select_workspace(&tab.workspace_id);
            d.state.active_tab_id = tab.id;
        }
        cx.notify();
    });
    focus_pane(pane_id, cx);
    // Reveals it in its dock (leaves a maximized pane) and moves keyboard focus into its terminal.
    crate::layout::jump_to_pane(pane_id, cx);
}

#[derive(Debug, Clone, PartialEq)]
pub enum PermissionCardEvent {
    /// "Terminal": the user answers in the agent's terminal; show that pane.
    Jump,
}

pub struct PermissionCard {
    pub request: PermissionRequest,
    details_open: bool,
    /// `AskUserQuestion`: picked option indexes, one list per question.
    picked: Vec<Vec<usize>>,
    /// Deny form open, with its reason field.
    reason: Option<Entity<InputState>>,
    _reason_sub: Option<Subscription>,
}

impl EventEmitter<PermissionCardEvent> for PermissionCard {}

impl PermissionCard {
    pub fn new(request: PermissionRequest) -> Self {
        Self {
            picked: vec![Vec::new(); request.questions.len()],
            request,
            details_open: false,
            reason: None,
            _reason_sub: None,
        }
    }

    fn start_deny(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Reason (optional), sent to the agent"));
        input.update(cx, |state, cx| state.focus(window, cx));
        let sub = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.deny(cx);
            }
        });
        self.reason = Some(input);
        self._reason_sub = Some(sub);
        cx.notify();
    }

    fn cancel_deny(&mut self, cx: &mut Context<Self>) {
        self.reason = None;
        self._reason_sub = None;
        cx.notify();
    }

    /// Single choice replaces, multi choice toggles. Sends at once when every question is single choice
    /// and answered; with a multi choice question the user presses "Send".
    fn pick(&mut self, question: usize, option: usize, cx: &mut Context<Self>) {
        let multi = self.request.questions[question].multi_select;
        let picks = &mut self.picked[question];
        if !multi {
            *picks = vec![option];
        } else if let Some(at) = picks.iter().position(|&i| i == option) {
            picks.remove(at);
        } else {
            picks.push(option);
        }
        let any_multi = self.request.questions.iter().any(|q| q.multi_select);
        if !any_multi && self.all_answered() {
            send_answers(&self.request, &self.picked, cx);
        }
        cx.notify();
    }

    fn all_answered(&self) -> bool {
        self.picked.iter().all(|p| !p.is_empty())
    }

    fn deny(&mut self, cx: &mut Context<Self>) {
        let reason = self
            .reason
            .as_ref()
            .map(|r| r.read(cx).value().trim().to_owned())
            .filter(|r| !r.is_empty());
        answer(&self.request, "deny", reason, cx);
    }
}

fn button(id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .h(ui(24.0))
        .rounded(ui(RADIUS_MD - 6.0))
        .px_2()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .child(label)
}

impl Render for PermissionCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let req = self.request.clone();
        let detail = div()
            .id("permission-detail")
            .max_h(ui(160.0))
            .overflow_y_scroll()
            .rounded(ui(4.0))
            .bg(hsla(p.gutter))
            .p(ui(6.0))
            .font_family(FONT_MONO)
            .text_size(ui(11.0))
            .text_color(hsla(p.fg))
            .child(req.detail.clone());
        let secondary = |b: Stateful<Div>| {
            b.border_1()
                .border_color(hsla(p.line_strong))
                .text_color(hsla(p.fg))
                .hover(|s| s.bg(hsla(p.hover)))
        };

        let terminal = button("terminal", "Terminal")
            .ml_auto()
            .text_color(hsla(p.subtle))
            .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
            .tooltip(|window, cx| Tooltip::new("Hand back to the agent's prompt in its terminal").build(window, cx))
            .on_click(cx.listener(|this, _, _, cx| {
                answer(&this.request, "ask", None, cx);
                cx.emit(PermissionCardEvent::Jump);
            }));

        if !req.questions.is_empty() {
            return self.render_questions(&req, terminal, cx).into_any_element();
        }

        let body: AnyElement = if let Some(input) = &self.reason {
            div()
                .flex()
                .flex_col()
                .gap(ui(6.0))
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.cancel_deny(cx);
                    }
                }))
                .child(Input::new(input).h(ui(24.0)).aria_label("Reason for denying (optional)"))
                .child(
                    div()
                        .flex()
                        .gap(ui(6.0))
                        .child(
                            button("deny-submit", "Deny")
                                .bg(hsla_a(p.bad, 0.2))
                                .text_color(hsla(p.bad))
                                .hover(|s| s.bg(hsla_a(p.bad, 0.3)))
                                .on_click(cx.listener(|this, _, _, cx| this.deny(cx))),
                        )
                        .child(secondary(button("deny-cancel", "Cancel")).on_click(cx.listener(|this, _, _, cx| this.cancel_deny(cx)))),
                )
                .into_any_element()
        } else {
            let suggestions = req.suggestions.join(", ");
            let auto_rule = rule_for(&req.tool_name, &req.detail);
            let r = req.clone();
            div()
                .flex()
                .flex_wrap()
                .gap(ui(6.0))
                .child(
                    button("allow", "Allow")
                        .bg(hsla(p.accent))
                        .text_color(hsla(p.on_accent))
                        .hover(|s| s.bg(hsla(p.accent_hover)))
                        .on_click(move |_, _, cx| answer(&r, "allow", None, cx)),
                )
                .when(!req.suggestions.is_empty(), |row| {
                    let r = req.clone();
                    let tip: SharedString = format!("Allow and add the rule {suggestions} suggested by the agent").into();
                    row.child(
                        secondary(button("allow-always", "Always allow"))
                            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                            .on_click(move |_, _, cx| answer(&r, "allow_always", None, cx)),
                    )
                })
                .when_some(auto_rule, |row, rule| {
                    let r = req.clone();
                    let tip: SharedString =
                        format!("AgentDeck will answer {rule} itself in this folder from now on (Settings > Agents)").into();
                    row.child(
                        secondary(button("auto-allow", "Auto-allow here"))
                            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                            .on_click(move |_, _, cx| auto_allow_here(&r, cx)),
                    )
                })
                .child(
                    button("deny", "Deny")
                        .text_color(hsla(p.bad))
                        .hover(|s| s.bg(hsla_a(p.bad, 0.15)))
                        .on_click(cx.listener(|this, _, window, cx| this.start_deny(window, cx))),
                )
                .child(terminal)
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap(ui(6.0))
            .rounded(ui(RADIUS_MD - 4.0))
            .border_1()
            .border_color(hsla_a(p.wait, 0.4))
            .bg(hsla(p.base))
            .p_2()
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(ui(6.0))
                    .text_xs()
                    .text_color(hsla(p.fg))
                    .child(Icon::new(tool_icon(&req.tool_name)).size(ui(14.0)).text_color(hsla(p.wait)))
                    .child(div().flex_1().min_w_0().child(req.summary.clone())),
            )
            // Shell commands are always shown in full: the summary is truncated.
            .map(|card| {
                if is_shell(&req.tool_name) {
                    return card.child(detail);
                }
                let open = self.details_open;
                card.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .text_xs()
                        .text_color(hsla(p.subtle))
                        .child(
                            div()
                                .id("details-toggle")
                                .cursor_pointer()
                                .hover(|s| s.text_color(hsla(p.fg)))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.details_open = !this.details_open;
                                    cx.notify();
                                }))
                                .child(if open { "▾ Details" } else { "▸ Details" }),
                        )
                        .when(open, |d| d.child(detail)),
                )
            })
            .child(body)
            .into_any_element()
    }
}

impl PermissionCard {
    /// `AskUserQuestion`: each question with its options as clickable choices (radio or checkbox).
    fn render_questions(&self, req: &PermissionRequest, terminal: Stateful<Div>, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let any_multi = req.questions.iter().any(|q| q.multi_select);
        let ready = self.all_answered();
        let mut card = div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap(ui(8.0))
            .rounded(ui(RADIUS_MD - 4.0))
            .border_1()
            .border_color(hsla_a(p.wait, 0.4))
            .bg(hsla(p.base))
            .p_2()
            .text_xs()
            .text_color(hsla(p.fg));
        for (qi, q) in req.questions.iter().enumerate() {
            let mut options = div().flex().flex_col().gap(ui(4.0));
            for (oi, option) in q.options.iter().enumerate() {
                let on = self.picked[qi].contains(&oi);
                let mark = match (q.multi_select, on) {
                    (true, true) => IconName::SquareCheck,
                    (true, false) => IconName::Square,
                    (false, true) => IconName::CircleDot,
                    (false, false) => IconName::Circle,
                };
                options = options.child(
                    div()
                        .id(SharedString::from(format!("q{qi}-o{oi}")))
                        .flex()
                        .items_start()
                        .gap(ui(6.0))
                        .rounded(ui(RADIUS_MD - 6.0))
                        .border_1()
                        .border_color(hsla(if on { p.accent } else { p.line_strong }))
                        .when(on, |d| d.bg(hsla_a(p.accent, 0.12)))
                        .px_2()
                        .py(ui(4.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(hsla(p.hover)))
                        .on_click(cx.listener(move |this, _, _, cx| this.pick(qi, oi, cx)))
                        .child(Icon::new(mark).size(ui(14.0)).text_color(hsla(if on { p.accent } else { p.subtle })))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(div().font_weight(FontWeight::MEDIUM).child(option.label.clone()))
                                .when(!option.description.is_empty(), |d| {
                                    d.child(div().text_color(hsla(p.subtle)).child(option.description.clone()))
                                }),
                        ),
                );
            }
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(ui(4.0))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(ui(6.0))
                            .child(Icon::new(IconName::CircleQuestionMark).size(ui(14.0)).text_color(hsla(p.wait)))
                            .child(div().flex_1().min_w_0().font_weight(FontWeight::MEDIUM).child(q.question.clone())),
                    )
                    .child(options),
            );
        }
        card.child(
            div()
                .flex()
                .gap(ui(6.0))
                .when(any_multi, |row| {
                    row.child(
                        button("send-answers", "Send")
                            .bg(hsla(if ready { p.accent } else { p.line_strong }))
                            .text_color(hsla(p.on_accent))
                            .when(ready, |b| {
                                b.hover(|s| s.bg(hsla(p.accent_hover)))
                                    .on_click(cx.listener(|this, _, _, cx| send_answers(&this.request, &this.picked, cx)))
                            }),
                    )
                })
                .child(terminal),
        )
    }
}

/// Floating card (top right) with the oldest pending permission of the current workspace and "+N" more.
/// Other workspaces' requests stay in the activity panel.
pub struct PermissionBanner {
    deck: Entity<Deck>,
    cards: HashMap<String, (Entity<PermissionCard>, Subscription)>,
    _observe: Subscription,
}

impl PermissionBanner {
    pub fn new(deck: Entity<Deck>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&deck, |this: &mut Self, deck, cx| {
            // Pruned here, not in render: the banner is hidden while the activity panel shows the cards.
            let pending = &deck.read(cx).permissions;
            this.cards.retain(|id, _| pending.iter().any(|r| &r.request_id == id));
            cx.notify();
        });
        Self {
            deck,
            cards: HashMap::new(),
            _observe: observe,
        }
    }

    /// The one card of a request, shared by the banner and the activity panel.
    pub(crate) fn card(&mut self, request: &PermissionRequest, cx: &mut Context<Self>) -> Entity<PermissionCard> {
        if let Some((card, _)) = self.cards.get(&request.request_id) {
            return card.clone();
        }
        let card = cx.new(|_| PermissionCard::new(request.clone()));
        let pane_id = request.pane_id.clone();
        let sub = cx.subscribe(&card, move |_, _, _: &PermissionCardEvent, cx| jump_to(&pane_id, cx));
        self.cards.insert(request.request_id.clone(), (card.clone(), sub));
        card
    }
}

impl Render for PermissionBanner {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let deck = self.deck.read(cx);
        let workspace = deck.state.active_workspace().id.clone();
        let in_workspace = |r: &&PermissionRequest| {
            deck.state
                .tabs
                .iter()
                .find(|t| t.pane_ids.contains(&r.pane_id))
                .is_some_and(|t| t.workspace_id == workspace)
        };
        let requests: Vec<PermissionRequest> = deck.permissions.iter().filter(in_workspace).cloned().collect();
        let Some(request) = requests.first().cloned() else {
            return div();
        };
        let Some(pane) = self.deck.read(cx).state.panes.get(&request.pane_id).cloned() else {
            return div();
        };
        let more = requests.len() - 1;
        let card = self.card(&request, cx);
        div()
            .absolute()
            .top(ui(48.0))
            .right(ui(16.0))
            .w(ui(340.0))
            .flex()
            .flex_col()
            .gap(ui(6.0))
            .rounded(ui(RADIUS_LG - 4.0))
            .border_1()
            .border_color(hsla(p.line_strong))
            .bg(hsla(p.raised))
            .p(ui(10.0))
            .shadow_lg()
            .occlude()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(hsla(p.muted))
                    .child(agent_logo(pane.agent, 14.0, None, &p))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .child(div().font_weight(FontWeight::MEDIUM).text_color(hsla(p.fg)).child(display_title(&pane)))
                                    .child(if request.questions.is_empty() { "is asking for permission" } else { "has a question" }),
                            ),
                    )
                    .when(more > 0, |row| row.child(div().text_color(hsla(p.subtle)).child(format!("+{more}")))),
            )
            .child(card)
    }
}
