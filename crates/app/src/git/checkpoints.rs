//! Task checkpoints (original: src/lib/checkpoints.ts): an exact snapshot of a pane's working tree
//! (untracked files included) taken before each agent task, so the task can be undone from the Git
//! panel. Never touches the working tree or index. Also the reviewer setter of src/store/review.ts.

use std::collections::HashMap;

use agentdeck_core::git::{git_checkpoint, git_pane_dir};
use agentdeck_core::logic::queue::creates_loop;
use gpui_kit::*;

use crate::state::{Deck, DeckEvent};

/// Directory where the pane's agent actually runs (its git worktree when it has one).
pub fn pane_git_dir(deck: &Deck, pane_id: &str) -> Option<String> {
    let pane = deck.state.panes.get(pane_id)?;
    Some(git_pane_dir(pane.cwd.clone(), pane.id.clone(), pane.worktree))
}

/// Records a checkpoint of the pane's working tree, off the UI thread.
/// Resolves to its sha, or `None` when the pane is gone or not in a git repo.
pub fn checkpoint_pane(deck: &Entity<Deck>, pane_id: &str, cx: &App) -> Task<Result<Option<String>, String>> {
    let pane = deck
        .read(cx)
        .state
        .panes
        .get(pane_id)
        .map(|p| (p.cwd.clone(), p.worktree));
    let pane_id = pane_id.to_owned();
    cx.background_spawn(async move {
        let Some((cwd, worktree)) = pane else {
            return Ok(None);
        };
        git_checkpoint(git_pane_dir(cwd, pane_id.clone(), worktree), pane_id)
    })
}

/// Takes the automatic checkpoint each time an agent starts a task (`onTaskStart` in the original).
pub(super) fn watch_task_starts(deck: &Entity<Deck>, cx: &mut App) {
    // pane id -> startedAt of the task we last checkpointed.
    let mut started: HashMap<String, i64> = HashMap::new();
    cx.subscribe(deck, move |deck, event: &DeckEvent, cx| {
        #[allow(irrefutable_let_patterns)]
        let DeckEvent::Backend(event) = event else {
            return;
        };
        if event.name != "agent-status" {
            return;
        }
        let Some(pane_id) = event.payload["paneId"].as_str() else {
            return;
        };
        let current = deck
            .read(cx)
            .tasks
            .get(pane_id)
            .and_then(|log| log.current.as_ref())
            .map(|task| task.started_at);
        let Some(at) = current else {
            started.remove(pane_id);
            return;
        };
        if started.insert(pane_id.to_owned(), at) == Some(at) {
            return;
        }
        let job = checkpoint_pane(&deck, pane_id, cx);
        cx.background_spawn(async move {
            if let Err(e) = job.await {
                eprintln!("checkpoint failed: {e}");
            }
        })
        .detach();
    })
    .detach();
}

/// Code then review: `reviewer` reviews `author`'s changes when it finishes a task (`None` clears).
pub fn set_reviewer(deck: &mut Deck, author: &str, reviewer: Option<&str>, cx: &mut Context<Deck>) {
    let Some(reviewer) = reviewer else {
        deck.reviews.remove(author);
        cx.notify();
        return;
    };
    let mut rest = deck.reviews.clone();
    rest.remove(author);
    if creates_loop(&rest, author, reviewer) {
        deck.toast("That reviewer already sends its work back to this agent: pick another one.", cx);
        return;
    }
    deck.reviews.insert(author.to_owned(), reviewer.to_owned());
    cx.notify();
}
