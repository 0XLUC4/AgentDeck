//! Merges the orchestrator's worktree agents when its turn ends, so finished work never stays
//! stranded in `<repo>-agentdeck-*` folders. Each merge goes through the merge check (a build,
//! undone when it fails); failures go back to the orchestrator as a prompt, at most
//! [`MAX_RETRIES`] times in a row, then to the user as a toast.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use agentdeck_core::git;
use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::Pane;
use agentdeck_core::logic::orchestrator::scope_of;
use agentdeck_core::logic::queue::ADVANCE_DELAY_MS;
use gpui_kit::*;

use super::runtime;
use crate::state::Deck;

/// Failure prompts sent to one orchestrator in a row before leaving it to the user.
const MAX_RETRIES: u8 = 2;

#[derive(Default)]
struct Watch {
    /// Last seen status of every pane, to catch "just finished".
    statuses: HashMap<String, PaneStatus>,
    /// Orchestrator id -> failure prompts sent in a row.
    retries: HashMap<String, u8>,
    /// Orchestrators with a sweep running.
    running: HashSet<String>,
}

impl Global for Watch {}

/// Watches the deck for orchestrators finishing their turn. Called once with the main deck.
pub fn start(deck: &Entity<Deck>, cx: &mut App) {
    cx.set_global(Watch::default());
    cx.observe(deck, |deck, cx| on_change(&deck, cx)).detach();
}

fn on_change(deck: &Entity<Deck>, cx: &mut App) {
    let d = deck.read(cx);
    let watch = cx.global::<Watch>();
    let ended: Vec<String> = d
        .state
        .panes
        .values()
        .filter(|p| p.orchestrator && p.status == PaneStatus::Done)
        .filter(|p| watch.statuses.get(&p.id) != Some(&PaneStatus::Done))
        // A queued prompt is about to start its next turn: not the end of the run.
        .filter(|p| d.queues.get(&p.id).is_none_or(|q| q.items.is_empty() || q.paused))
        .map(|p| p.id.clone())
        .collect();
    let auto = d.settings.auto_merge;
    let statuses = d.state.panes.iter().map(|(id, p)| (id.clone(), p.status)).collect();
    let watch = cx.global_mut::<Watch>();
    watch.statuses = statuses;
    if !auto {
        return;
    }
    let start: Vec<String> = ended.into_iter().filter(|id| watch.running.insert(id.clone())).collect();
    for id in start {
        sweep(deck.clone(), id, cx);
    }
}

/// Commits and merges every finished worktree agent of the orchestrator `orch_id`.
fn sweep(deck: Entity<Deck>, orch_id: String, cx: &mut App) {
    cx.spawn(async move |cx| {
        // Same pause as the queue: a prompt typed right at the end turns it busy again first.
        cx.background_executor().timer(Duration::from_millis(ADVANCE_DELAY_MS)).await;
        let (panes, check, still_done) = deck.read_with(cx, |d, _| {
            let done = d.state.panes.get(&orch_id).is_some_and(|p| p.status == PaneStatus::Done);
            let panes: Vec<Pane> = scope_of(&d.state, &orch_id)
                .map(|s| s.panes.iter().map(|(p, _)| (*p).clone()).filter(|p| p.worktree && !p.orchestrator).collect())
                .unwrap_or_default();
            (panes, d.settings.merge_check.clone(), done)
        });
        let results = if still_done && !panes.is_empty() {
            cx.background_executor().spawn(async move { merge_all(&panes, &check) }).await
        } else {
            Vec::new()
        };
        cx.update(|cx| report(&deck, &orch_id, results, cx));
    })
    .detach();
}

/// Blocking: (agent title, merge outcome) of each worktree with work to merge. Busy agents are
/// left out: their work is not finished.
fn merge_all(panes: &[Pane], check: &str) -> Vec<(String, Result<String, String>)> {
    let mut results = Vec::new();
    for pane in panes {
        if !pane.exited && matches!(pane.status, PaneStatus::Working | PaneStatus::Waiting) {
            results.push((pane.title.clone(), Err("still working: not merged".to_owned())));
            continue;
        }
        let dir = git::git_pane_dir(pane.cwd.clone(), pane.id.clone(), true);
        let outcome = git::git_status(dir.clone()).and_then(|status| {
            if !status.agent_worktree {
                return Ok(git::NOTHING_TO_MERGE.to_owned()); // its worktree is already gone
            }
            if !status.files.is_empty() {
                git::git_commit(dir.clone(), format!("{}: finished work (AgentDeck)", pane.title))?;
            }
            git::git_merge_worktree(dir, check.to_owned())
        });
        if !outcome.as_ref().is_ok_and(|out| out.ends_with(git::NOTHING_TO_MERGE)) {
            results.push((pane.title.clone(), outcome));
        }
    }
    results
}

fn report(deck: &Entity<Deck>, orch_id: &str, results: Vec<(String, Result<String, String>)>, cx: &mut App) {
    let watch = cx.global_mut::<Watch>();
    watch.running.remove(orch_id);
    let merged: Vec<&str> = results.iter().filter(|(_, r)| r.is_ok()).map(|(t, _)| t.as_str()).collect();
    let failed: Vec<(&str, &str)> = results
        .iter()
        .filter_map(|(t, r)| r.as_ref().err().map(|e| (t.as_str(), e.as_str())))
        .collect();
    let prompt_orchestrator = if failed.is_empty() {
        watch.retries.remove(orch_id);
        false
    } else {
        let retries = watch.retries.entry(orch_id.to_owned()).or_default();
        *retries += 1;
        *retries <= MAX_RETRIES
    };
    if !merged.is_empty() {
        deck.update(cx, |d, cx| d.toast(format!("Auto-merged: {}", merged.join(", ")), cx));
    }
    if failed.is_empty() {
        return;
    }
    let idle = deck.read(cx).state.panes.get(orch_id).is_some_and(|p| !p.exited && p.status != PaneStatus::Working);
    if prompt_orchestrator && idle {
        runtime::send_prompt(orch_id, &failure_prompt(&failed), true, cx);
    } else {
        let names: Vec<&str> = failed.iter().map(|(t, _)| *t).collect();
        let message = format!("Auto-merge failed for {}: open its Git panel to see why.", names.join(", "));
        deck.update(cx, |d, cx| d.toast(message, cx));
    }
}

fn failure_prompt(failed: &[(&str, &str)]) -> String {
    let list: Vec<String> = failed.iter().map(|(title, error)| format!("- {title}: {error}")).collect();
    format!(
        "AgentDeck could not merge these worktrees into the main repository:\n{}\n\
         Fix it: send each problem to the agent that owns the worktree (send_prompt), wait for it, then end \
         your turn. AgentDeck merges again when you finish. If the main repository has uncommitted changes, \
         tell the user instead.",
        list.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::failure_prompt;

    #[test]
    fn failure_prompt_lists_every_agent() {
        let text = failure_prompt(&[("Front", "conflicts in a.ts"), ("API", "`cargo check` failed")]);
        assert!(text.contains("- Front: conflicts in a.ts") && text.contains("- API: `cargo check` failed"));
    }
}
