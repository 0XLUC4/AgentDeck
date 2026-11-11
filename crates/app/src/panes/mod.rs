//! Pane chrome and pane lifecycle: pane header (logo, title, timer, actions, menu), status badge,
//! working outline and voice glow, exit bar, agent picker, permission cards, and the runtime that
//! spawns / restarts / closes the agents (store/runtime.ts).
//! Original: src/components/TerminalPane/{PaneHeader,PaneMenu,PanePanel,WorkingOutline,VoiceGlow}.tsx,
//! src/components/{AgentPicker,AgentLogo,StatusBadge,Permission,IconButton}, src/store/runtime.ts.
//!
//! Wiring: call [`start`] once with the deck (restores the saved sessions, starts the 1 s clock and the
//! event handlers). Layout renders one [`PaneHeader`] per pane, wraps header and terminal in
//! [`pane_frame`], shows [`AgentPicker`] and [`PermissionBanner`] as overlays.

mod frame;
mod header;
mod logo;
mod permission;
mod picker;
pub mod runtime;
mod widgets;

use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::Pane;
use agentdeck_core::logic::text::{agent_name, format_elapsed, short_action};
use agentdeck_core::logic::theme::Palette;
use gpui_kit::*;

// Public surface for layout and the feature modules; not every item is wired yet.
#[allow(unused_imports)]
pub use frame::{VoiceGlow, exit_bar, pane_border, pane_frame, pane_frame_border, pane_overlays, steps_bar};
#[allow(unused_imports)]
pub use header::{PANE_COLORS, PaneHeader, PaneHeaderEvent, confirm_close, display_title};
#[allow(unused_imports)]
pub use logo::agent_logo;
#[allow(unused_imports)]
pub use permission::{PermissionBanner, PermissionCard, PermissionCardEvent, answer, auto_allow_here, jump_to};
#[allow(unused_imports)]
pub use picker::{AgentPicker, last_agent, spawn_agent};
#[allow(unused_imports)]
pub use runtime::start;
#[allow(unused_imports)]
pub use widgets::{icon_button, status_badge, status_dot};

use crate::theme::{hsla, ui};

/// Status dot color (StatusBadge.tsx).
pub fn status_color(status: PaneStatus, p: &Palette) -> u32 {
    match status {
        PaneStatus::Idle => p.subtle,
        PaneStatus::Working => p.busy,
        PaneStatus::Waiting => p.wait,
        PaneStatus::Done => p.ok,
        PaneStatus::Error => p.bad,
    }
}

/// Status label (tooltip of the badge, read by screen readers).
pub fn status_label(status: PaneStatus) -> &'static str {
    match status {
        PaneStatus::Idle => "Idle",
        PaneStatus::Working => "Working",
        PaneStatus::Waiting => "Waiting for your reply",
        PaneStatus::Done => "Done",
        PaneStatus::Error => "Error",
    }
}

/// Static, non-interactive header row (detached windows, previews). The live one is [`PaneHeader`].
pub fn pane_header(pane: &Pane, p: &Palette, now: i64) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .h(ui(30.0))
        .px(ui(10.0))
        .bg(hsla(p.panel))
        .text_color(hsla(p.fg))
        .text_size(ui(13.0))
        .child(agent_logo(pane.agent, 14.0, None, p))
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .truncate()
                .child(display_title(pane)),
        )
        .child(status_dot(&pane.id, pane.status, p))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .child(short_action(pane.last_action.as_deref())),
        )
        .children(pane.task_started_at.map(|start| {
            div()
                .flex_shrink_0()
                .text_xs()
                .font_family(agentdeck_core::logic::theme::FONT_MONO)
                .text_color(hsla(p.muted))
                .child(format_elapsed(now - start))
        }))
        .child(div().text_xs().text_color(hsla(p.subtle)).child(agent_name(pane.agent)))
}
