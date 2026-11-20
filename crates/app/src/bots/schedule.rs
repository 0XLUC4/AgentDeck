use agentdeck_core::logic::automations::{Automation, is_due};
use agentdeck_core::logic::text::now_ms;
use gpui_kit::{App, Entity};

use super::data;
use crate::state::Deck;

pub(crate) fn run_due(deck: &Entity<Deck>, cx: &mut App) {
    let profiles = match data::list(deck, cx) {
        Ok(profiles) => profiles,
        Err(error) => {
            eprintln!("Reading bot schedules: {error}");
            return;
        }
    };
    let now = now_ms();
    for bot in profiles {
        let Some(schedule) = bot.schedule else { continue };
        let timing = Automation {
            schedule,
            enabled: true,
            created_at: bot.created_at,
            last_run: bot.last_run,
            ..Automation::default()
        };
        if is_due(&timing, now) {
            if let Err(error) = data::run_scheduled(deck, &bot.id, cx) {
                eprintln!("Scheduled bot {}: {error}", bot.name);
            }
        }
    }
}
