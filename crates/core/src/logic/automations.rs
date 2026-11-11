//! Automations: prompts that run on a schedule while AgentDeck is open. A due automation starts
//! a fresh agent in its folder and sends it the prompt (the app's `automations` module runs them).
//! Saved in local.json (`persist::save_automations`).

use chrono::{Duration, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

use crate::agents::AgentKind;

pub const MAX_PROMPT_BYTES: usize = 64 * 1024;
pub const MAX_HOURS: u32 = 168;
const HOUR_MS: i64 = 3_600_000;
const MINUTE_MS: i64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Schedule {
    /// Every N hours (1 to 168), counted from the last run.
    EveryHours(u32),
    /// Once a day at this local time.
    DailyAt { hour: u8, minute: u8 },
}

impl Default for Schedule {
    fn default() -> Self {
        Self::EveryHours(24)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Automation {
    pub id: String,
    pub name: String,
    pub agent: AgentKind,
    pub cwd: String,
    pub prompt: String,
    pub schedule: Schedule,
    pub enabled: bool,
    /// ms since the epoch, 0 = never.
    pub last_run: i64,
    /// ms since the epoch: a schedule never fires for a slot older than the automation itself.
    pub created_at: i64,
}

impl Default for Automation {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            agent: AgentKind::Claude,
            cwd: String::new(),
            prompt: String::new(),
            schedule: Schedule::default(),
            enabled: false,
            last_run: 0,
            created_at: 0,
        }
    }
}

/// Local `hour:minute` of `date` in ms; a time skipped by a clock change moves one hour later.
fn slot<Tz: TimeZone>(tz: &Tz, date: NaiveDate, hour: u8, minute: u8) -> Option<i64> {
    let naive = date.and_hms_opt(u32::from(hour.min(23)), u32::from(minute.min(59)), 0)?;
    tz.from_local_datetime(&naive)
        .earliest()
        .or_else(|| tz.from_local_datetime(&(naive + Duration::hours(1))).earliest())
        .map(|time| time.timestamp_millis())
}

/// When the automation next runs (ms); a time at or before `now` means it is due.
/// Never-run automations count from their creation, so nothing fires just because the app opened.
pub fn due_at_in<Tz: TimeZone>(automation: &Automation, now: i64, tz: &Tz) -> i64 {
    let last = automation.last_run.max(automation.created_at);
    match automation.schedule {
        Schedule::EveryHours(hours) => last + i64::from(hours.clamp(1, MAX_HOURS)) * HOUR_MS,
        Schedule::DailyAt { hour, minute } => {
            let Some(today) = tz.timestamp_millis_opt(now).single().map(|t| t.date_naive()) else {
                return i64::MAX;
            };
            match slot(tz, today, hour, minute) {
                Some(at) if last < at => at,
                _ => today.succ_opt().and_then(|tomorrow| slot(tz, tomorrow, hour, minute)).unwrap_or(i64::MAX),
            }
        }
    }
}

/// [`due_at_in`] in the computer's time zone.
pub fn due_at(automation: &Automation, now: i64) -> i64 {
    due_at_in(automation, now, &Local)
}

/// Enabled, and its time has come.
pub fn is_due(automation: &Automation, now: i64) -> bool {
    automation.enabled && due_at(automation, now) <= now
}

/// Already started less than a minute ago: a second start would be a duplicate.
pub fn ran_this_minute(automation: &Automation, now: i64) -> bool {
    automation.last_run != 0 && now - automation.last_run < MINUTE_MS
}

/// "Every 6 hours", "Daily at 06:00".
pub fn schedule_label(schedule: Schedule) -> String {
    match schedule {
        Schedule::EveryHours(1) => "Every hour".to_owned(),
        Schedule::EveryHours(hours) => format!("Every {hours} hours"),
        Schedule::DailyAt { hour, minute } => format!("Daily at {hour:02}:{minute:02}"),
    }
}

/// "Paused", "Due now", "Today 06:00", "Mon 16 Nov 06:00".
pub fn next_run_label_in<Tz: TimeZone>(automation: &Automation, now: i64, tz: &Tz) -> String {
    if !automation.enabled {
        return "Paused".to_owned();
    }
    let at = due_at_in(automation, now, tz);
    if at <= now {
        return "Due now".to_owned();
    }
    let (Some(next), Some(today)) = (tz.timestamp_millis_opt(at).single(), tz.timestamp_millis_opt(now).single()) else {
        return "Never".to_owned();
    };
    let next = next.naive_local();
    if next.date() == today.date_naive() {
        next.format("Today %H:%M").to_string()
    } else {
        next.format("%a %d %b %H:%M").to_string()
    }
}

/// [`next_run_label_in`] in the computer's time zone.
pub fn next_run_label(automation: &Automation, now: i64) -> String {
    next_run_label_in(automation, now, &Local)
}

/// What stops an automation from being saved, as a sentence for the form.
pub fn validate(automation: &Automation) -> Result<(), String> {
    if automation.name.trim().is_empty() {
        return Err("Give the automation a name.".to_owned());
    }
    if automation.agent == AgentKind::Shell {
        return Err("An automation needs a coding agent, not a plain shell.".to_owned());
    }
    if !std::path::Path::new(automation.cwd.trim()).is_dir() || automation.cwd.trim().is_empty() {
        return Err(format!("Folder not found: {}", automation.cwd.trim()));
    }
    match automation.schedule {
        Schedule::EveryHours(hours) if !(1..=MAX_HOURS).contains(&hours) => {
            return Err(format!("The interval must be between 1 and {MAX_HOURS} hours."));
        }
        Schedule::DailyAt { hour, minute } if hour > 23 || minute > 59 => {
            return Err("The time must be between 00:00 and 23:59.".to_owned());
        }
        _ => {}
    }
    if automation.prompt.trim().is_empty() {
        return Err("Write the prompt the agent will receive.".to_owned());
    }
    if automation.prompt.len() > MAX_PROMPT_BYTES {
        return Err("The prompt is too long (64 KB at most).".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    /// UTC+2, so local and UTC days differ around midnight.
    fn tz() -> FixedOffset {
        FixedOffset::east_opt(2 * 3600).expect("valid offset")
    }

    fn at(day: u32, hour: u32, minute: u32) -> i64 {
        tz().with_ymd_and_hms(2026, 11, day, hour, minute, 0).single().expect("valid time").timestamp_millis()
    }

    fn automation(schedule: Schedule, created_at: i64, last_run: i64) -> Automation {
        Automation {
            id: "a".into(),
            name: "Bump".into(),
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            prompt: "bump the dependencies".into(),
            schedule,
            enabled: true,
            last_run,
            created_at,
            ..Automation::default()
        }
    }

    #[test]
    fn every_hours_waits_one_interval_after_creation() {
        let a = automation(Schedule::EveryHours(6), at(10, 8, 0), 0);
        assert_eq!(due_at_in(&a, at(10, 8, 1), &tz()), at(10, 14, 0));
        assert!(due_at_in(&a, at(10, 13, 59), &tz()) > at(10, 13, 59));
        assert!(due_at_in(&a, at(10, 14, 0), &tz()) <= at(10, 14, 0));
    }

    #[test]
    fn every_hours_counts_from_the_last_run() {
        let a = automation(Schedule::EveryHours(1), at(10, 8, 0), at(10, 14, 0));
        assert_eq!(due_at_in(&a, at(10, 14, 30), &tz()), at(10, 15, 0));
    }

    #[test]
    fn daily_is_due_once_the_slot_passed() {
        let daily = Schedule::DailyAt { hour: 6, minute: 0 };
        let a = automation(daily, at(9, 12, 0), at(10, 6, 0));
        // Before today's slot: not due, next run is today.
        assert_eq!(due_at_in(&a, at(11, 5, 59), &tz()), at(11, 6, 0));
        // Slot passed and the last run is older: due.
        assert!(due_at_in(&a, at(11, 6, 0), &tz()) <= at(11, 6, 0));
        assert!(due_at_in(&a, at(11, 23, 0), &tz()) <= at(11, 23, 0));
        // Ran after today's slot: tomorrow.
        let ran = automation(daily, at(9, 12, 0), at(11, 6, 1));
        assert_eq!(due_at_in(&ran, at(11, 6, 2), &tz()), at(12, 6, 0));
    }

    #[test]
    fn daily_uses_the_local_day() {
        // 00:30 local on the 11th is still the 10th in UTC.
        let a = automation(Schedule::DailyAt { hour: 0, minute: 15 }, at(9, 12, 0), at(10, 0, 15));
        assert!(due_at_in(&a, at(11, 0, 30), &tz()) <= at(11, 0, 30));
    }

    #[test]
    fn daily_created_after_the_slot_waits_for_tomorrow() {
        let a = automation(Schedule::DailyAt { hour: 6, minute: 0 }, at(11, 15, 0), 0);
        assert_eq!(due_at_in(&a, at(11, 15, 1), &tz()), at(12, 6, 0));
    }

    #[test]
    fn disabled_is_never_due() {
        let mut a = automation(Schedule::EveryHours(1), 1, 0);
        assert!(is_due(&a, at(11, 12, 0)));
        a.enabled = false;
        assert!(!is_due(&a, at(11, 12, 0)));
    }

    #[test]
    fn one_start_per_minute() {
        let a = automation(Schedule::EveryHours(1), 0, at(11, 12, 0));
        assert!(ran_this_minute(&a, at(11, 12, 0) + 59_000));
        assert!(!ran_this_minute(&a, at(11, 12, 1)));
        assert!(!ran_this_minute(&automation(Schedule::EveryHours(1), 0, 0), 30_000));
    }

    #[test]
    fn labels() {
        assert_eq!(schedule_label(Schedule::EveryHours(1)), "Every hour");
        assert_eq!(schedule_label(Schedule::EveryHours(6)), "Every 6 hours");
        assert_eq!(schedule_label(Schedule::DailyAt { hour: 6, minute: 0 }), "Daily at 06:00");
        let mut a = automation(Schedule::DailyAt { hour: 6, minute: 0 }, at(9, 12, 0), at(11, 6, 1));
        assert_eq!(next_run_label_in(&a, at(11, 7, 0), &tz()), "Thu 12 Nov 06:00");
        assert_eq!(next_run_label_in(&a, at(12, 5, 0), &tz()), "Today 06:00");
        assert_eq!(next_run_label_in(&a, at(12, 6, 0), &tz()), "Due now");
        a.enabled = false;
        assert_eq!(next_run_label_in(&a, at(12, 6, 0), &tz()), "Paused");
    }

    #[test]
    fn validation() {
        let good = automation(Schedule::EveryHours(6), 0, 0);
        assert_eq!(validate(&good), Ok(()));
        let invalid = |edit: fn(&mut Automation)| {
            let mut a = automation(Schedule::EveryHours(6), 0, 0);
            edit(&mut a);
            validate(&a).is_err()
        };
        assert!(invalid(|a| a.name = "  ".into()));
        assert!(invalid(|a| a.prompt = "\n".into()));
        assert!(invalid(|a| a.prompt = "x".repeat(MAX_PROMPT_BYTES + 1)));
        assert!(invalid(|a| a.cwd = String::new()));
        assert!(invalid(|a| a.cwd = "Z:/no/such/folder/agentdeck".into()));
        assert!(invalid(|a| a.agent = AgentKind::Shell));
        assert!(invalid(|a| a.schedule = Schedule::EveryHours(0)));
        assert!(invalid(|a| a.schedule = Schedule::EveryHours(169)));
        assert!(invalid(|a| a.schedule = Schedule::DailyAt { hour: 24, minute: 0 }));
    }

    #[test]
    fn old_files_still_load() {
        let a: Automation = serde_json::from_str(r#"{"id":"x","name":"n","schedule":{"dailyAt":{"hour":6,"minute":30}}}"#)
            .expect("partial automation loads");
        assert_eq!(a.schedule, Schedule::DailyAt { hour: 6, minute: 30 });
        assert_eq!((a.agent, a.enabled, a.last_run), (AgentKind::Claude, false, 0));
        let round: Automation = serde_json::from_str(&serde_json::to_string(&a).expect("serializes")).expect("parses");
        assert_eq!(round, a);
    }
}
