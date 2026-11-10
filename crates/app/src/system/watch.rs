//! Agent watchers (original: store/runtime.ts listenAgentStatus / watchAgents / scheduleRateLimitRetry
//! and the permission notification of components/Permission/permissions.ts): status notifications,
//! taskbar flash, stuck and still-waiting reminders, "continue" after a rate limit, tray waiting count.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use agentdeck_core::hooks_server::permissions::PermissionRequest;
use agentdeck_core::hooks_server::{AgentStatusEvent, PaneStatus};
use agentdeck_core::logic::deck::{ActivityKey, activity_key};
use agentdeck_core::logic::text::{agent_name, now_ms};
use gpui_kit::*;

use super::notify::notify;
use crate::state::{Deck, DeckEvent};

struct Watch {
    deck: Entity<Deck>,
    /// Pane statuses as of the last deck change: the "previous" status when a hook event arrives.
    statuses: HashMap<String, PaneStatus>,
    last_hook_at: HashMap<String, i64>,
    /// Alerts sent per pane: the status_since they were for, and how many.
    stall_alerts: HashMap<String, (i64, i64)>,
    rate_limit_retries: HashSet<String>,
    waiting: usize,
}

struct WatchGlobal(#[allow(dead_code)] Entity<Watch>);

impl Global for WatchGlobal {}

pub(super) fn init(deck: Entity<Deck>, cx: &mut App) {
    let watch = cx.new(|cx| {
        cx.subscribe(&deck, Watch::on_event).detach();
        cx.observe(&deck, Watch::on_change).detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |w, cx| w.watch_agents(now_ms(), cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Watch {
            deck: deck.clone(),
            statuses: HashMap::new(),
            last_hook_at: HashMap::new(),
            stall_alerts: HashMap::new(),
            rate_limit_retries: HashSet::new(),
            waiting: 0,
        }
    });
    watch.update(cx, |w, cx| w.on_change(w.deck.clone(), cx));
    cx.set_global(WatchGlobal(watch));
}

impl Watch {
    fn on_change(&mut self, deck: Entity<Deck>, cx: &mut Context<Self>) {
        let state = &deck.read(cx).state;
        self.statuses = state.panes.iter().map(|(id, p)| (id.clone(), p.status)).collect();
        let waiting = state
            .panes
            .values()
            .filter(|p| activity_key(p) == ActivityKey::Status(PaneStatus::Waiting))
            .count();
        if waiting != self.waiting {
            self.waiting = waiting;
            super::tray::set_waiting(cx, waiting);
        }
    }

    fn on_event(&mut self, deck: Entity<Deck>, event: &DeckEvent, cx: &mut Context<Self>) {
        let DeckEvent::Backend(event) = event;
        match event.name.as_str() {
            "agent-status" => {
                if let Ok(e) = serde_json::from_value::<AgentStatusEvent>(event.payload.clone()) {
                    self.on_status(&deck, &e, cx);
                }
            }
            "agent-permission" => {
                if let Ok(req) = serde_json::from_value::<PermissionRequest>(event.payload.clone()) {
                    self.on_permission(&deck, &req, cx);
                }
            }
            _ => {}
        }
    }

    fn on_status(&mut self, deck: &Entity<Deck>, e: &AgentStatusEvent, cx: &mut Context<Self>) {
        self.last_hook_at.insert(e.pane_id.clone(), now_ms());
        let prev = self.statuses.insert(e.pane_id.clone(), e.status);
        let focused = super::is_main_focused(cx);
        let d = deck.read(cx);
        let Some(pane) = d.state.panes.get(&e.pane_id) else {
            return;
        };
        let (agent, mut title) = (agent_name(pane.agent), pane.title.clone());
        // Several agents share a title ("Claude"): the workspace says which one it is.
        let workspace = d.state.tabs.iter().find(|t| t.pane_ids.contains(&e.pane_id)).and_then(|t| {
            d.state.workspaces.iter().find(|w| w.id == t.workspace_id)
        });
        if let Some(ws) = workspace {
            title = format!("{} · {title}", ws.name);
        }
        let unseen = !focused || d.state.active_pane_id.as_deref() != Some(&e.pane_id);
        let settings = d.settings.clone();

        if e.status == PaneStatus::Error
            && e.event == "StopFailure"
            && e.last_action.as_deref().is_some_and(mentions_rate_limit)
        {
            self.schedule_rate_limit_retry(&e.pane_id, cx);
        }
        // Flash the taskbar button while the window is in the background and an agent needs the user.
        if e.status == PaneStatus::Waiting && prev != Some(PaneStatus::Waiting) && !focused {
            if let Some(w) = super::window::main_window(cx) {
                let _ = w.update(cx, |_, window, _| window.request_attention());
            }
        }
        // PermissionRequest has its own, more precise notification (on_permission).
        if prev == Some(e.status) || e.event == "PermissionRequest" {
            return;
        }
        let prefs = &settings.notify;
        let (wanted, text) = match e.status {
            PaneStatus::Done => (prefs.done, "finished"),
            PaneStatus::Waiting => (prefs.waiting, "is waiting for your answer"),
            PaneStatus::Error => (prefs.error, "hit an error"),
            _ => return,
        };
        if wanted && (unseen || !prefs.only_unseen) {
            // A waiting agent's own message ("Approval requested: npm test") says more than the pane title.
            let body = match (e.status, &e.last_action) {
                (PaneStatus::Waiting, Some(message)) => format!("{title}: {message}"),
                _ => title,
            };
            notify(&settings, &format!("{agent} {text}"), &body);
        }
    }

    fn on_permission(&mut self, deck: &Entity<Deck>, req: &PermissionRequest, cx: &mut Context<Self>) {
        let focused = super::is_main_focused(cx);
        let d = deck.read(cx);
        // Auto-allowed requests never reach the pending list: no notification for those.
        if !d.permissions.iter().any(|r| r.request_id == req.request_id) {
            return;
        }
        let Some(pane) = d.state.panes.get(&req.pane_id) else {
            return;
        };
        let prefs = &d.settings.notify;
        let unseen = !focused || d.state.active_pane_id.as_deref() != Some(&req.pane_id);
        if prefs.waiting && (unseen || !prefs.only_unseen) {
            let title = match req.questions.first() {
                Some(q) => format!("{} asks: {}", agent_name(pane.agent), q.question),
                None => format!("{} is asking for permission: {}", agent_name(pane.agent), req.summary),
            };
            notify(&d.settings, &title, &pane.title);
        }
    }

    /// Every second: a hooked agent working with no hook event for `stuck_minutes`, and reminders
    /// every `remind_minutes` while an agent keeps waiting.
    fn watch_agents(&mut self, now: i64, cx: &mut Context<Self>) {
        let d = self.deck.read(cx);
        let (stuck, remind) = (i64::from(d.settings.stuck_minutes), i64::from(d.settings.remind_minutes));
        let mut due_list = Vec::new();
        for pane in d.state.panes.values() {
            if pane.exited || !pane.hooked {
                continue;
            }
            let sent = self.stall_alerts.get(&pane.id).filter(|s| s.0 == pane.status_since);
            let mut count = sent.map_or(0, |s| s.1);
            let agent = agent_name(pane.agent);
            let mut due = None;
            if pane.status == PaneStatus::Working && stuck > 0 && count == 0 {
                // A long command or a big write sends no hook for minutes: only silence on both counts is stuck.
                let last_hook = self.last_hook_at.get(&pane.id).copied().unwrap_or(0);
                let last_output = crate::panes::runtime::pane_last_output_at(&pane.id, cx);
                if now - last_hook.max(last_output).max(pane.status_since) > stuck * 60_000 {
                    due = Some(format!("{agent} looks stuck ({stuck} min without progress)"));
                }
            } else if pane.status == PaneStatus::Waiting && remind > 0 && d.settings.notify.waiting {
                let reminders = (now - pane.status_since) / (remind * 60_000);
                if reminders > count {
                    due = Some(format!("{agent} is still waiting for you"));
                    count = reminders - 1;
                }
            }
            if let Some(due) = due {
                due_list.push((pane.id.clone(), pane.status_since, count + 1, due, pane.title.clone()));
            }
        }
        if due_list.is_empty() {
            return;
        }
        let settings = d.settings.clone();
        for (id, since, count, due, title) in due_list {
            self.stall_alerts.insert(id, (since, count));
            notify(&settings, &due, &title);
        }
    }

    /// Sends "continue" once the wait is over, if the agent is still stopped on that error.
    fn schedule_rate_limit_retry(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        let d = self.deck.read(cx);
        let minutes = d.settings.retry_rate_limit_minutes;
        let Some(since) = d.state.panes.get(pane_id).map(|p| p.status_since) else {
            return;
        };
        if minutes == 0 || !self.rate_limit_retries.insert(pane_id.to_owned()) {
            return;
        }
        let id = pane_id.to_owned();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(u64::from(minutes) * 60))
                .await;
            let _ = this.update(cx, |w, cx| {
                w.rate_limit_retries.remove(&id);
                let deck = w.deck.clone();
                let still_stopped = deck.read(cx).state.panes.get(&id).is_some_and(|p| {
                    !p.exited && p.status == PaneStatus::Error && p.status_since == since
                });
                if !still_stopped {
                    return;
                }
                let (title, backend) = {
                    let d = deck.read(cx);
                    (d.state.panes.get(&id).map(|p| p.title.clone()).unwrap_or_default(), d.backend.clone())
                };
                deck.update(cx, |d, cx| d.toast(format!("{title}: continuing after the rate limit"), cx));
                let send = crate::voice::send_prompt(&backend, id, "continue", true, cx);
                cx.background_spawn(async move {
                    if let Err(e) = send.await {
                        eprintln!("rate limit retry failed: {e}");
                    }
                })
                .detach();
            });
        })
        .detach();
    }
}

/// `/rate.?limit/i`.
fn mentions_rate_limit(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.match_indices("rate").any(|(i, _)| {
        let rest = &lower[i + 4..];
        rest.starts_with("limit") || rest.chars().next().is_some_and(|c| rest[c.len_utf8()..].starts_with("limit"))
    })
}

#[cfg(test)]
mod tests {
    use super::mentions_rate_limit;

    #[test]
    fn rate_limit_text() {
        assert!(mentions_rate_limit("API Error: Rate limit reached"));
        assert!(mentions_rate_limit("rate_limit_error"));
        assert!(mentions_rate_limit("ratelimited"));
        assert!(!mentions_rate_limit("generate a limit"));
        assert!(!mentions_rate_limit("rate  limit"));
    }
}
