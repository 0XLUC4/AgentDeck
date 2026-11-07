//! Git panel (status, diff, commit, merge worktree, PR, checkpoints restore, compare panes,
//! pull/push) and the diff chip in the pane header.
//! Original: src/components/Git/{GitPanel,DiffChip,diff}.tsx, src/lib/checkpoints.ts. Backend: agentdeck_core::git.
//!
//! Hosting: a pane header keeps one `Entity<DiffChip>` per pane (`DiffChip::new(deck, pane_id, cx)`) and
//! renders it; `DiffChip::open` shows the Git panel (pane menu "Git"). The panel draws itself as a
//! full-window overlay, so it can live anywhere in the tree. `GitPanel` can also be hosted directly:
//! it emits `DismissEvent` on close.

mod checkpoints;
mod chip;
mod diff;
mod panel;

use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::*;

pub use checkpoints::{checkpoint_pane, pane_git_dir};
pub use chip::DiffChip;
pub use panel::GitPanel;

use crate::layout::DeckView;

/// Starts the automatic checkpoint before each agent task, once the main `DeckView` exists.
pub fn init(cx: &mut App) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    cx.observe_new(|view: &mut DeckView, _, cx| {
        if !STARTED.swap(true, Ordering::SeqCst) {
            let deck = view.deck().clone();
            checkpoints::watch_task_starts(&deck, cx);
        }
    })
    .detach();
}
