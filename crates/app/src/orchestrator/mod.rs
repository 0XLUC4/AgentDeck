//! Orchestrator (a Claude pane driving the workspace agents over MCP), its activity log and pause,
//! the wall (every agent at a glance, Ctrl+Shift+W), link workspace dialog (open projects, clone),
//! folder trust. Original: src/components/{Orchestrator,Wall,LinkWorkspace}, src/lib/orchestrator.ts.
//! Logic: agentdeck_core::logic::orchestrator; backend: agentdeck_core::{mcp,workspace,trust}.
//!
//! Folder trust has no UI of its own: `settings.trust_agent_folders` goes to `pty::spawn_pane`
//! (`trust`), which calls `trust::trust_folder` before the agent starts.
//!
//! Public entry points for the other modules:
//! - title bar: [`wall_toggle_button`]; pane area: [`wall_overlay`] (render it on top while open);
//! - pane header: [`orchestrator_badge`] (crown + activity log), palette: [`create_orchestrator`],
//!   [`set_paused`], [`toggle_wall`]; sidebar: [`open_link_workspace`].

mod button;
mod link;
pub(crate) mod runtime;
mod tools;
mod wall;

use gpui_kit::*;

use crate::layout::DeckView;

// Entry points for layout, panes and palette (not all wired yet).
#[allow(unused_imports)]
pub use button::{OrchestratorLog, open_activity_log, orchestrator_badge, orchestrator_crown, pause_toggle};
#[allow(unused_imports)]
pub use link::{LinkWorkspace, open_link_workspace};
#[allow(unused_imports)]
pub use tools::{create_orchestrator, set_paused};
#[allow(unused_imports)]
pub use wall::{Wall, jump_to_pane, toggle_wall, wall_open, wall_overlay, wall_toggle_button};

actions!(agentdeck, [ToggleWall]);

/// Set once the main window's deck is wired to the MCP tool handler.
struct Listening;

impl Global for Listening {}

/// Registers this module's key bindings and globals. Called once from `main`.
pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("ctrl-shift-w", ToggleWall, None)]);
    cx.on_action(|_: &ToggleWall, cx| toggle_wall(cx));
    // The first DeckView is the main window: answer the acting MCP tools from its deck.
    cx.observe_new(|view: &mut DeckView, _, cx| {
        if cx.has_global::<Listening>() {
            return;
        }
        cx.set_global(Listening);
        let deck = view.deck().clone();
        cx.subscribe(&deck, |_, deck, event, cx| tools::on_deck_event(&deck, event, cx))
            .detach();
    })
    .detach();
}
