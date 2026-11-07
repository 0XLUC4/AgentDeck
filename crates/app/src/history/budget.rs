//! Cost limits (Settings > Notifications): warns once per day / per agent run when the estimated cost
//! crosses the limit and, if asked, interrupts the agents and pauses the orchestrator.
//! Original: `src/lib/budget.ts`.

use std::collections::HashSet;
use std::time::Duration;

use agentdeck_core::agents::AgentKind;
use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::Pane;
use agentdeck_core::logic::settings::BudgetSettings;
use agentdeck_core::logic::text::over_limit;
use agentdeck_core::usage::{pane_usage, usage_summary};
use gpui_kit::*;

use super::usage::agent_id;
use crate::state::Deck;

const CHECK: Duration = Duration::from_secs(60);

/// Starts the periodic check (once, from `history::init`).
pub(super) fn start_watch(deck: Entity<Deck>, cx: &mut App) {
    cx.spawn(async move |cx| {
        // Limits already reported: "day:2026-10-27", "pane:<id>:<generation>".
        let mut warned = HashSet::new();
        loop {
            cx.background_executor().timer(CHECK).await;
            check(&deck, &mut warned, cx).await;
        }
    })
    .detach();
}

async fn check(deck: &Entity<Deck>, warned: &mut HashSet<String>, cx: &mut AsyncApp) {
    let (budget, panes) = deck.read_with(cx, |deck, _| {
        let panes: Vec<Pane> = deck
            .state
            .panes
            .values()
            .filter(|p| matches!(p.agent, AgentKind::Claude | AgentKind::Codex) && !p.exited)
            .cloned()
            .collect();
        (deck.settings.budget.clone(), panes)
    });
    let day = format!("day:{}", chrono::Local::now().format("%Y-%m-%d"));
    if budget.daily_usd > 0.0 && !warned.contains(&day) {
        match cx.background_spawn(async { usage_summary("today".into()) }).await {
            Ok(summary) if over_limit(summary.total.cost, budget.daily_usd) => {
                warned.insert(day);
                let message =
                    format!("Today's agents cost about ${:.2} (limit ${}).", summary.total.cost, budget.daily_usd);
                hit(message, &budget, &panes, deck, cx);
            }
            Ok(_) => {}
            Err(e) => eprintln!("budget check failed: {e}"),
        }
    }
    if budget.pane_usd <= 0.0 {
        return;
    }
    for pane in &panes {
        let key = format!("pane:{}:{}", pane.id, pane.generation);
        if warned.contains(&key) {
            continue;
        }
        let Some(started_at) = cx.update(|cx| crate::panes::runtime::pane_started_at(&pane.id, cx)) else {
            continue;
        };
        let (id, cwd, agent) = (pane.id.clone(), pane.cwd.clone(), agent_id(pane.agent));
        match cx.background_spawn(async move { pane_usage(id, cwd, started_at, agent) }).await {
            Ok(usage) if over_limit(usage.totals.cost, budget.pane_usd) => {
                warned.insert(key);
                let message = format!(
                    "{} cost about ${:.2} (limit ${} per agent).",
                    pane.title, usage.totals.cost, budget.pane_usd
                );
                hit(message, &budget, std::slice::from_ref(pane), deck, cx);
            }
            Ok(_) => {}
            Err(e) => eprintln!("budget check failed: {e}"),
        }
    }
}

/// Toast + notification; with `interrupt`, stops the working agents and pauses the orchestrator.
fn hit(message: String, budget: &BudgetSettings, panes: &[Pane], deck: &Entity<Deck>, cx: &mut AsyncApp) {
    let settings = deck.read_with(cx, |d, _| d.settings.clone());
    deck.update(cx, |deck, cx| deck.toast(message.clone(), cx));
    crate::system::notify(&settings, "AgentDeck budget", &message);
    if !budget.interrupt {
        return;
    }
    cx.update(|cx| {
        for pane in panes.iter().filter(|p| p.status == PaneStatus::Working) {
            crate::panes::runtime::interrupt_pane(&pane.id, cx);
        }
        crate::orchestrator::set_paused(deck, true, cx);
    });
}
