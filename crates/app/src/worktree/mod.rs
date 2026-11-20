//! "Create worktree" dialog: a git worktree on a branch (typed, existing, from a GitHub pull
//! request / issue or a Linear issue), a session named after the branch under the chosen project,
//! and the chosen agent started in it, on this computer or a saved SSH host.
//!
//! Entry point for other modules (sidebar, palette, Tasks panel): [`open_create_worktree`].
//! Logic: `agentdeck_core::logic::new_worktree`, git: `agentdeck_core::git::branch_worktree`.

mod dialog;

use gpui_kit::*;

#[allow(unused_imports)] // public API for the Tasks panel
pub use agentdeck_core::logic::new_worktree::{GitHubKind, TaskSource, WorktreePrefill};
use dialog::CreateWorktree;

/// Dialog width (px at 100% zoom).
const WIDTH: f32 = 400.0;

/// Opens the dialog over the current window, prefilled with a project and/or a task
/// (shown as a chip in "Create from"). Does nothing before the deck exists.
pub fn open_create_worktree(prefill: Option<WorktreePrefill>, window: &mut Window, cx: &mut App) {
    let Some(deck) = crate::panes::runtime::deck(cx) else { return };
    let view = cx.new(|cx| CreateWorktree::new(deck, prefill.unwrap_or_default(), window, cx));
    let focus = view.read(cx).query_focus(cx);
    crate::palette::open_modal(view, WIDTH, None, focus, window, cx);
}
