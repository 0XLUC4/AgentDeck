//! Status bar items: subscription usage (Claude Pro/Max, ChatGPT/Codex: 5-hour %, weekly in the
//! tooltip) and agent CLI updates (count, menu with an Update button per CLI).
//! Backend: agentdeck_core::{usage::limits, agents::cli_update}. State lives in one global entity
//! created on first draw; reads and updates run on the background executor.

use std::time::{Duration, Instant};

use agentdeck_core::agents::AgentKind;
use agentdeck_core::agents::cli_update::{CliUpdate, check_updates, run_update};
use agentdeck_core::logic::text::agent_name;
use agentdeck_core::usage::limits::{LimitWindow, SubscriptionLimits, claude_limits, codex_limits};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{agent_mark, text_tooltip};
use crate::state::Deck;
use crate::theme::{hsla, palette, ui};
use agentdeck_core::logic::theme::{RADIUS_LG, RADIUS_MD};

const LIMITS_EVERY: Duration = Duration::from_secs(5 * 60);
const UPDATES_EVERY: Duration = Duration::from_secs(4 * 3600);

pub struct StatusItems {
    deck: WeakEntity<Deck>,
    claude: Option<SubscriptionLimits>,
    codex: Option<SubscriptionLimits>,
    refreshing: bool,
    updates: Vec<CliUpdate>,
    /// CLIs whose update command is running.
    updating: Vec<AgentKind>,
    menu_open: bool,
    /// When a click outside closed the menu: the same click on the toggle must not reopen it.
    menu_closed_at: Option<Instant>,
    _tickers: [Task<()>; 2],
}

struct Global(Entity<StatusItems>);
impl gpui::Global for Global {}

fn items(deck: &Entity<Deck>, cx: &mut App) -> Entity<StatusItems> {
    if let Some(global) = cx.try_global::<Global>() {
        return global.0.clone();
    }
    let deck = deck.downgrade();
    let entity = cx.new(|cx| StatusItems::new(deck, cx));
    cx.set_global(Global(entity.clone()));
    entity
}

impl StatusItems {
    fn new(deck: WeakEntity<Deck>, cx: &mut Context<Self>) -> Self {
        let limits = cx.spawn(async move |this, cx| {
            loop {
                if this.update(cx, |this, cx| this.refresh_limits(cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(LIMITS_EVERY).await;
            }
        });
        let updates = cx.spawn(async move |this, cx| {
            loop {
                if this.update(cx, |this, cx| this.check_updates(cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(UPDATES_EVERY).await;
            }
        });
        Self {
            deck,
            claude: None,
            codex: None,
            refreshing: false,
            updates: Vec::new(),
            updating: Vec::new(),
            menu_open: false,
            menu_closed_at: None,
            _tickers: [limits, updates],
        }
    }

    fn refresh_limits(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (claude, codex) = cx
                .background_executor()
                .spawn(async {
                    let claude = claude_limits().unwrap_or_else(|e| {
                        eprintln!("status: {e}");
                        None
                    });
                    (claude, codex_limits())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                (this.claude, this.codex, this.refreshing) = (claude, codex, false);
                cx.notify();
            });
        })
        .detach();
    }

    fn check_updates(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let updates = cx.background_executor().spawn(async { check_updates() }).await;
            let _ = this.update(cx, |this, cx| {
                this.updates = updates;
                this.menu_open &= !this.updates.is_empty();
                cx.notify();
            });
        })
        .detach();
    }

    fn run_update(&mut self, update: CliUpdate, cx: &mut Context<Self>) {
        if self.updating.contains(&update.kind) {
            return;
        }
        self.updating.push(update.kind);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn({
                let update = update.clone();
                async move { run_update(&update) }
            });
            let result = result.await;
            let name = agent_name(update.kind);
            let message = match result {
                Ok(()) => format!("{name} updated to {}", update.latest),
                Err(e) => format!("{name} update failed ({}): {e}", update.command.join(" ")),
            };
            let _ = this.update(cx, |this, cx| {
                this.updating.retain(|kind| *kind != update.kind);
                if let Some(deck) = this.deck.upgrade() {
                    deck.update(cx, |deck, cx| deck.toast(message, cx));
                }
                this.check_updates(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_menu(&mut self, cx: &mut Context<Self>) {
        let just_closed = self.menu_closed_at.is_some_and(|at| at.elapsed() < Duration::from_millis(300));
        self.menu_open = !self.menu_open && !just_closed;
        cx.notify();
    }
}

/// "in 2h 13m" within a day, else the local day and time.
fn reset_label(resets_at: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let left = resets_at - now;
    if left <= 0 {
        return "now".into();
    }
    if left < 24 * 3600 {
        return format!("in {}h {:02}m", left / 3600, left % 3600 / 60);
    }
    chrono::DateTime::from_timestamp(resets_at, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%a %H:%M").to_string())
        .unwrap_or_default()
}

fn window_line(label: &str, window: Option<LimitWindow>) -> Option<String> {
    let w = window?;
    let reset = w.resets_at.map(|r| format!(", resets {}", reset_label(r))).unwrap_or_default();
    Some(format!("{label}: {:.0}% used{reset}", w.percent))
}

/// Subscription usage: one logo + 5-hour % per signed-in provider and a refresh button.
/// `None` when neither provider reports usage.
pub fn subscription_item(deck: &Entity<Deck>, cx: &mut App) -> Option<AnyElement> {
    let entity = items(deck, cx);
    let p = palette(cx);
    let state = entity.read(cx);
    let providers = [(AgentKind::Claude, "Claude", state.claude), (AgentKind::Codex, "ChatGPT", state.codex)];
    let shown: Vec<_> = providers.into_iter().filter_map(|(kind, plan, limits)| Some((kind, plan, limits?))).collect();
    if shown.is_empty() {
        return None;
    }
    let refreshing = state.refreshing;
    let mut row = div().flex().flex_shrink_0().items_center().gap_2().whitespace_nowrap();
    for (kind, plan, limits) in shown {
        let headline = limits.five_hour.or(limits.weekly).map_or(0.0, |w| w.percent);
        let color = if headline >= 90.0 {
            p.bad
        } else if headline >= 75.0 {
            p.wait
        } else {
            p.muted
        };
        let tip = [Some(format!("{plan} subscription usage")), window_line("5-hour", limits.five_hour), window_line("Weekly", limits.weekly)]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n");
        row = row.child(
            div()
                .id(SharedString::from(format!("status-limit-{plan}")))
                .flex()
                .items_center()
                .gap(ui(4.0))
                .text_color(hsla(color))
                .child(agent_mark(kind, 12.0, cx))
                .child(format!("{headline:.0}%"))
                .tooltip(text_tooltip(tip)),
        );
    }
    let refresh = entity.clone();
    row = row.child(
        div()
            .id("status-limits-refresh")
            .flex()
            .items_center()
            .text_size(ui(12.0))
            .text_color(hsla(p.subtle))
            .when(!refreshing, |d| d.cursor_pointer().hover(|s| s.text_color(hsla(p.fg))))
            .child(if refreshing { IconName::LoaderCircle } else { IconName::RefreshCw })
            .tooltip(text_tooltip("Refresh subscription usage"))
            .on_click(move |_, _, cx| refresh.update(cx, |this, cx| this.refresh_limits(cx))),
    );
    Some(row.into_any_element())
}

/// "<n> CLI updates" with a menu listing each outdated CLI and its Update button. `None` when all
/// installed CLIs are current.
pub fn cli_updates_item(deck: &Entity<Deck>, cx: &mut App) -> Option<AnyElement> {
    let entity = items(deck, cx);
    let p = palette(cx);
    let state = entity.read(cx);
    if state.updates.is_empty() {
        return None;
    }
    let count = state.updates.len();
    let toggle = entity.clone();
    let mut root = div().relative().flex().flex_shrink_0().child(
        div()
            .id("status-cli-updates")
            .flex()
            .items_center()
            .gap(ui(6.0))
            .whitespace_nowrap()
            .cursor_pointer()
            .text_color(hsla(if state.menu_open { p.fg } else { p.muted }))
            .hover(|s| s.text_color(hsla(p.fg)))
            .child(div().text_size(ui(13.0)).child(IconName::Download))
            .child(format!("{count} CLI update{}", if count == 1 { "" } else { "s" }))
            .tooltip(text_tooltip("Newer versions of installed agent CLIs"))
            .on_click(move |_, _, cx| toggle.update(cx, |this, cx| this.toggle_menu(cx))),
    );
    if !state.menu_open {
        return Some(root.into_any_element());
    }

    let rows = state.updates.iter().map(|update| {
        let busy = state.updating.contains(&update.kind);
        let (run, update_now) = (entity.clone(), update.clone());
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py(ui(6.0))
            .child(agent_mark(update.kind, 14.0, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_color(hsla(p.fg)).child(agent_name(update.kind)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(hsla(p.subtle))
                            .child(format!("{} → {}", update.current, update.latest)),
                    ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("cli-update-{}", update.kind.id())))
                    .flex_shrink_0()
                    .px_2()
                    .h(ui(24.0))
                    .flex()
                    .items_center()
                    .rounded(ui(RADIUS_MD))
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .map(|d| {
                        if busy {
                            d.text_color(hsla(p.subtle)).child("Updating…")
                        } else {
                            d.bg(hsla(p.busy))
                                .text_color(hsla(p.on_accent))
                                .hover(|s| s.opacity(0.9))
                                .child("Update")
                                .tooltip(text_tooltip(update.command.join(" ")))
                                .on_click(move |_, _, cx| {
                                    run.update(cx, |this, cx| this.run_update(update_now.clone(), cx))
                                })
                        }
                    }),
            )
    });
    let close = entity.clone();
    let panel = div()
        .id("status-cli-menu")
        .occlude()
        .mb_1()
        .w(ui(288.0))
        .p_1()
        .rounded(ui(RADIUS_LG))
        .border_1()
        .border_color(hsla(p.line_strong))
        .bg(hsla(p.raised))
        .shadow_lg()
        .text_size(ui(13.0))
        .on_mouse_down_out(move |_, _, cx| {
            close.update(cx, |this, cx| {
                this.menu_open = false;
                this.menu_closed_at = Some(Instant::now());
                cx.notify();
            })
        })
        .children(rows);
    root = root.child(
        div()
            .absolute()
            .bottom_full()
            .right_0()
            .child(deferred(anchored().anchor(Anchor::BottomRight).snap_to_window_with_margin(px(8.0)).child(panel))),
    );
    Some(root.into_any_element())
}
