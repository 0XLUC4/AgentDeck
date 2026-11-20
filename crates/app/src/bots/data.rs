use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use agentdeck_core::agents::{AgentKind, find_repo_root};
use agentdeck_core::logic::automations::{MAX_HOURS, Schedule};
use agentdeck_core::logic::deck::{PaneSpec, new_id};
use agentdeck_core::logic::text::{now_ms, split_args};
use agentdeck_core::persist::write_atomic;
use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::panes::runtime;
use crate::state::Deck;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BotRun {
    pub pane_id: String,
    pub title: String,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bot {
    pub id: String,
    pub name: String,
    pub mission: String,
    #[serde(default)]
    pub memory: String,
    pub cwd: String,
    pub avatar_seed: String,
    pub agent: AgentKind,
    #[serde(default)]
    pub history: Vec<BotRun>,
    #[serde(default)]
    pub schedule: Option<Schedule>,
    /// Most recent scheduled dispatch. A manual task does not move the schedule.
    #[serde(default)]
    pub last_run: i64,
    pub created_at: i64,
}

impl Bot {
    pub fn new(cwd: String) -> Self {
        let id = new_id();
        Self {
            avatar_seed: id.clone(),
            id,
            name: String::new(),
            mission: String::new(),
            memory: String::new(),
            cwd,
            agent: AgentKind::Claude,
            history: Vec::new(),
            schedule: None,
            last_run: 0,
            created_at: 0,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Give the bot a name.".into());
        }
        if self.mission.trim().is_empty() {
            return Err("Write a mission for this bot.".into());
        }
        if self.agent == AgentKind::Shell {
            return Err("Choose a coding agent for this bot.".into());
        }
        if !Path::new(&self.cwd).is_dir() {
            return Err(format!("Folder not found: {}", self.cwd));
        }
        match self.schedule {
            Some(Schedule::EveryHours(hours)) if !(1..=MAX_HOURS).contains(&hours) => {
                Err(format!("Choose an interval between 1 and {MAX_HOURS} hours."))
            }
            Some(Schedule::DailyAt { hour, minute }) if hour > 23 || minute > 59 => {
                Err("Choose a local time between 00:00 and 23:59.".into())
            }
            _ => Ok(()),
        }
    }

    fn prompt(&self, task: Option<&str>) -> Result<String, String> {
        let mut prompt = format!("Bot: {}\n\nMission:\n{}", self.name.trim(), self.mission.trim());
        if !self.memory.trim().is_empty() {
            prompt.push_str("\n\nSaved memory:\n");
            prompt.push_str(self.memory.trim());
        }
        prompt.push_str("\n\nTask:\n");
        prompt.push_str(match task {
            Some(t) if t.trim().is_empty() => return Err("Write a task for this bot.".into()),
            Some(t) => t.trim(),
            None => "Carry out your standing mission.",
        });
        // Some agents receive the prompt through a terminal, including its bracketed paste delimiters.
        if prompt.chars().any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t')) {
            return Err("Remove terminal control characters from the bot instructions or task.".into());
        }
        Ok(prompt)
    }
}

struct Profiles {
    list: Vec<Bot>,
    runs: HashMap<String, (usize, usize)>,
}

impl From<Vec<Bot>> for Profiles {
    fn from(list: Vec<Bot>) -> Self {
        let mut runs = HashMap::new();
        for (profile, bot) in list.iter().enumerate() {
            for (index, run) in bot.history.iter().enumerate() {
                runs.insert(run.pane_id.clone(), (profile, index));
            }
        }
        Self { list, runs }
    }
}

#[derive(Default)]
pub struct Bots(RefCell<HashMap<PathBuf, Profiles>>);
impl Global for Bots {}

pub fn init(cx: &mut App) {
    cx.set_global(Bots::default());
}

fn read_profiles(dir: &Path) -> Result<Vec<Bot>, String> {
    let path = dir.join("bots.json");
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn with_profiles<T>(dir: &Path, cx: &App, read: impl FnOnce(&Profiles) -> T) -> Result<T, String> {
    let mut cache = cx.global::<Bots>().0.borrow_mut();
    if let Some(profiles) = cache.get(dir) {
        return Ok(read(profiles));
    }
    let profiles = Profiles::from(read_profiles(dir)?);
    let result = read(&profiles);
    cache.insert(dir.to_owned(), profiles);
    Ok(result)
}

pub fn list(deck: &Entity<Deck>, cx: &App) -> Result<Vec<Bot>, String> {
    with_profiles(&deck.read(cx).data_dir, cx, |p| p.list.clone())
}

fn store(dir: &Path, profiles: Vec<Bot>, cx: &mut App) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let bytes = serde_json::to_vec_pretty(&profiles).map_err(|e| format!("Saving bots: {e}"))?;
    write_atomic(&dir.join("bots.json"), &bytes)?;
    cx.global_mut::<Bots>().0.get_mut().insert(dir.to_owned(), Profiles::from(profiles));
    Ok(())
}

/// Receives completion events for every pane. Only bot tasks have an entry in the index.
pub(crate) fn record_result(dir: &Path, pane_id: &str, summary: &str, cx: &mut App) -> Result<(), String> {
    if summary.trim().is_empty() {
        return Ok(());
    }
    let changed = with_profiles(dir, cx, |p| {
        let &(bot, run) = p.runs.get(pane_id)?;
        if p.list[bot].history[run].result.as_deref() == Some(summary) {
            return None;
        }
        let mut list = p.list.clone();
        list[bot].history[run].result = Some(summary.to_owned());
        Some(list)
    })?;
    if let Some(profiles) = changed {
        store(dir, profiles, cx)?;
    }
    Ok(())
}

fn upsert(profiles: &mut Vec<Bot>, mut bot: Bot, now: i64) -> Result<(), String> {
    if let Some(old) = profiles.iter_mut().find(|b| b.id == bot.id) {
        bot.history = std::mem::take(&mut old.history);
        bot.last_run = old.last_run;
        bot.created_at = if bot.schedule != old.schedule { now } else { old.created_at };
        *old = bot;
    } else if bot.created_at == 0 {
        bot.created_at = now;
        profiles.push(bot);
    } else {
        return Err("This bot has been deleted.".into());
    }
    Ok(())
}

pub fn save(deck: &Entity<Deck>, mut bot: Bot, cx: &mut App) -> Result<(), String> {
    bot.name = bot.name.trim().to_owned();
    bot.cwd = bot.cwd.trim().to_owned();
    bot.validate()?;
    let mut profiles = list(deck, cx)?;
    upsert(&mut profiles, bot, now_ms())?;
    store(&deck.read(cx).data_dir.clone(), profiles, cx)
}

pub fn remove(deck: &Entity<Deck>, id: &str, cx: &mut App) -> Result<(), String> {
    let mut profiles = list(deck, cx)?;
    profiles.retain(|b| b.id != id);
    store(&deck.read(cx).data_dir.clone(), profiles, cx)
}

pub fn run(deck: &Entity<Deck>, id: &str, task: &str, cx: &mut App) -> Result<String, String> {
    dispatch(deck, id, Some(task), cx)
}

pub(super) fn run_scheduled(deck: &Entity<Deck>, id: &str, cx: &mut App) -> Result<String, String> {
    dispatch(deck, id, None, cx)
}

fn dispatch(deck: &Entity<Deck>, id: &str, task: Option<&str>, cx: &mut App) -> Result<String, String> {
    let mut profiles = list(deck, cx)?;
    let bot = profiles.iter_mut().find(|b| b.id == id).ok_or("This bot no longer exists.")?;
    bot.validate()?;
    let prompt = bot.prompt(task)?;
    let title: String = task
        .unwrap_or(&bot.mission)
        .trim()
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(100)
        .collect();
    let d = deck.read(cx);
    let tab_id = crate::automations::session_for(&d.state, &bot.cwd).ok_or("Open a session to run a bot task.")?;
    let name = bot.name.clone();
    let mut spec = PaneSpec::new(bot.agent, format!("{name} · {title}"), bot.cwd.clone());
    spec.args = split_args(&d.settings.agent(bot.agent).args);
    spec.font_size = d.settings.font_size;
    spec.worktree = find_repo_root(Path::new(&bot.cwd)).is_some();
    spec.chat = matches!(bot.agent, AgentKind::Claude | AgentKind::Codex);
    spec.worker = true;
    let chat = spec.chat;
    let pane_id = spec.id.clone();
    let started_at = now_ms();
    bot.history.push(BotRun {
        pane_id: pane_id.clone(),
        title,
        started_at,
        result: None,
    });
    if task.is_none() {
        bot.last_run = started_at;
    }
    // Reserve the occurrence and its pane ID before starting an external process.
    store(&deck.read(cx).data_dir.clone(), profiles, cx)?;
    runtime::create_pane(spec, &tab_id, false, cx);
    if chat {
        // Chat queues input while the agent and its worktree start.
        runtime::send_prompt(&pane_id, &prompt, cx);
    } else {
        let deck = deck.clone();
        let id = pane_id.clone();
        cx.spawn(async move |cx| {
            if !crate::palette::send_when_ready(deck.clone(), id, prompt, cx).await {
                deck.update(cx, |d, cx| d.toast(format!("{name}: the agent could not receive its task."), cx));
            }
        })
        .detach();
    }
    Ok(pane_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_uses_the_current_profile_and_preserves_multiline_memory() {
        let mut bot = Bot::new(String::new());
        bot.name = "Helper".into();
        bot.mission = "Review dependencies".into();
        bot.memory = "First line\n第二行".into();
        let prompt = bot.prompt(Some("Review this patch")).unwrap();
        for part in ["Helper", "Review dependencies", "First line\n第二行", "Review this patch"] {
            assert!(prompt.contains(part));
        }
        assert!(bot.prompt(None).unwrap().contains("Review dependencies"));
        assert!(bot.prompt(Some(" \n")).is_err());
        assert!(bot.prompt(Some("\x1b[201~")).is_err());
    }

    #[test]
    fn editing_a_profile_keeps_runs_and_restarts_a_changed_schedule() {
        let mut profiles = vec![];
        let new = Bot::new(String::new());
        upsert(&mut profiles, new, 1).unwrap();
        let mut draft = profiles[0].clone();
        profiles[0].history.push(BotRun {
            pane_id: "p".into(),
            title: "Task".into(),
            started_at: 2,
            result: Some("Done".into()),
        });
        profiles[0].last_run = 2;
        draft.schedule = Some(Schedule::EveryHours(1));
        upsert(&mut profiles, draft, 3).unwrap();
        assert_eq!(profiles[0].history.len(), 1);
        assert_eq!((profiles[0].last_run, profiles[0].created_at), (2, 3));
        assert!(upsert(&mut vec![], profiles[0].clone(), 4).is_err());
    }

    #[test]
    fn missing_and_corrupt_profile_files_are_distinct() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_profiles(tmp.path()).unwrap().is_empty());
        let bot = Bot::new(tmp.path().display().to_string());
        let path = tmp.path().join("bots.json");
        write_atomic(&path, &serde_json::to_vec(&[&bot]).unwrap()).unwrap();
        assert_eq!(read_profiles(tmp.path()).unwrap(), vec![bot]);
        fs::write(path, b"[").unwrap();
        assert!(read_profiles(tmp.path()).is_err());
    }
}
