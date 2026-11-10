//! Pane lifecycle for the orchestrator tools, on top of `panes::runtime` (store/runtime.ts port).

use std::time::Duration;

use agentdeck_core::logic::deck::PaneSpec;
use agentdeck_core::logic::queue::prompt_text;
use agentdeck_core::{AppState, pty};
use gpui_kit::*;

use crate::panes::runtime as panes;
use crate::state::Deck;

const START_POLL: Duration = Duration::from_millis(100);
/// Spawns wait for one of 3 slots. Below the 30 s the MCP server gives the UI to answer a tool
/// call: past that the orchestrator gets an error for a spawn that still succeeds.
const START_TIMEOUT: Duration = Duration::from_secs(25);

/// Creates the pane and resolves once its process runs, it failed, or it was closed meanwhile.
/// False when the wait ran out first: the pane is still queued for a spawn slot.
/// shortcut: polls the backend because `panes::runtime::create_pane` returns no task; switch to
/// awaiting it if it ever does.
pub async fn create_pane(deck: &Entity<Deck>, spec: PaneSpec, tab_id: String, cx: &mut AsyncApp) -> bool {
    let id = spec.id.clone();
    cx.update(|cx| panes::create_pane(spec, &tab_id, false, cx));
    let state = deck.read_with(cx, |d, _| d.backend.try_state::<AppState>());
    let mut waited = Duration::ZERO;
    while waited < START_TIMEOUT {
        let settled = deck.read_with(cx, |d, _| d.state.panes.get(&id).is_none_or(|p| p.exited));
        let running = state
            .as_ref()
            .and_then(|s| pty::list_panes(s).ok())
            .is_some_and(|ids| ids.contains(&id));
        if settled || running {
            return true;
        }
        cx.background_executor().timer(START_POLL).await;
        waited += START_POLL;
    }
    false
}

/// Types the prompt; with `submit`, Enter follows in a separate write.
pub fn send_prompt(pane_id: &str, text: &str, submit: bool, cx: &mut App) {
    if submit {
        panes::send_prompt(pane_id, text, cx);
    } else {
        panes::send_input(pane_id, &prompt_text(text), cx);
    }
}

pub use panes::{close_pane, focus_pane, interrupt_pane, pane_output_tail};
