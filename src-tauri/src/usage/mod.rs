//! Token usage and cost per agent, workspace and day, read from the agents' own transcripts.
//! Files are parsed incrementally (byte offset per file) and cached for the app's lifetime.

mod parse;
mod prices;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use chrono::{Days, Local, NaiveTime, TimeZone};
use serde::Serialize;

use parse::{Entry, FileState, Source};

#[derive(Serialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub tokens: u64,
    /// USD, priced models only.
    pub cost: f64,
    /// Tokens of models missing from the price table (not in `cost`).
    pub unpriced_tokens: u64,
}

impl Totals {
    fn add(&mut self, e: &Entry) {
        let t = &e.tokens;
        self.input += t.input;
        self.output += t.output;
        self.cache_write += t.cache_write_5m + t.cache_write_1h;
        self.cache_read += t.cache_read;
        self.tokens += t.total();
        match prices::price(&e.model) {
            Some(p) => {
                self.cost += (t.input as f64 * p.input
                    + t.output as f64 * p.output
                    + t.cache_read as f64 * p.cache_read
                    + t.cache_write_5m as f64 * p.cache_write_5m
                    + t.cache_write_1h as f64 * p.cache_write_1h)
                    / 1e6
            }
            None => self.unpriced_tokens += t.total(),
        }
    }
}

#[derive(Serialize, Debug)]
pub struct Keyed {
    pub key: String,
    #[serde(flatten)]
    pub totals: Totals,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub total: Totals,
    pub by_agent: Vec<Keyed>,
    pub by_workspace_cwd: Vec<Keyed>,
    /// Every day of the range, oldest first, `YYYY-MM-DD` (local time).
    pub by_day: Vec<Keyed>,
}

#[derive(Serialize, Debug)]
pub struct PaneUsage {
    #[serde(flatten)]
    pub totals: Totals,
    pub models: Vec<String>,
}

type Cache = Mutex<HashMap<PathBuf, FileState>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn claude_projects() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| Some(home()?.join(".claude")))
        .map(|d| d.join("projects"))
}

fn codex_sessions() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| Some(home()?.join(".codex")))
        .map(|d| d.join("sessions"))
}

/// Transcript files under `dir` (recursive) modified at or after `since_ms`, with (len, mtime ms).
fn walk(dir: &Path, since_ms: i64, out: &mut Vec<(PathBuf, u64, i64)>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            walk(&path, since_ms, out);
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.ends_with(".jsonl") || name.ends_with(".jsonl.zst")) {
            continue;
        }
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as i64);
        if mtime >= since_ms {
            out.push((path, meta.len(), mtime));
        }
    }
}

/// Refreshes `path` and hands its entries to `f`. The cache lock is not held while parsing,
/// so a long first scan doesn't block other callers (a concurrent refresh of the same file is harmless).
fn with_entries(path: &Path, len: u64, mtime: i64, source: Source, f: &mut impl FnMut(&[Entry])) {
    let state = cache().lock().ok().and_then(|mut c| c.remove(path));
    let mut state = state.unwrap_or_else(|| FileState::new(source));
    if let Err(e) = state.refresh(path, len, mtime) {
        eprintln!("usage: cannot read {}: {e}", path.display());
    }
    f(&state.entries);
    if let Ok(mut c) = cache().lock() {
        c.insert(path.to_path_buf(), state);
    }
}

/// Visits every entry of both agents in files touched since `since_ms`.
fn for_each_source(since_ms: i64, mut f: impl FnMut(Source, &[Entry])) {
    for (source, root) in [
        (Source::Claude, claude_projects()),
        (Source::Codex, codex_sessions()),
    ] {
        let Some(root) = root else { continue };
        let mut files = Vec::new();
        walk(&root, since_ms, &mut files);
        for (path, len, mtime) in files {
            with_entries(&path, len, mtime, source, &mut |e| f(source, e));
        }
    }
}

/// Local midnight `days_back` days ago, in UTC ms.
fn local_midnight(days_back: u64) -> i64 {
    let day = Local::now().date_naive() - Days::new(days_back);
    Local
        .from_local_datetime(&day.and_time(NaiveTime::MIN))
        .earliest()
        .map_or(0, |t| t.timestamp_millis())
}

fn day_key(ts: i64) -> String {
    Local
        .timestamp_millis_opt(ts)
        .single()
        .map_or_else(String::new, |t| t.format("%Y-%m-%d").to_string())
}

fn agent_name(source: Source) -> &'static str {
    match source {
        Source::Claude => "claude",
        Source::Codex => "codex",
    }
}

fn keyed(map: BTreeMap<String, Totals>) -> Vec<Keyed> {
    let mut v: Vec<Keyed> = map
        .into_iter()
        .map(|(key, totals)| Keyed { key, totals })
        .collect();
    v.sort_by(|a, b| {
        b.totals
            .cost
            .total_cmp(&a.totals.cost)
            .then(b.totals.tokens.cmp(&a.totals.tokens))
    });
    v
}

fn empty_days(days: u64) -> BTreeMap<String, Totals> {
    (0..days)
        .map(|back| (day_key(local_midnight(back)), Totals::default()))
        .collect()
}

fn summarize(days: u64) -> UsageSummary {
    let since = local_midnight(days - 1);
    let mut total = Totals::default();
    let mut by_agent = BTreeMap::<String, Totals>::new();
    let mut by_cwd = BTreeMap::<String, Totals>::new();
    let mut by_day = empty_days(days);
    for_each_source(since, |source, entries| {
        for e in entries.iter().filter(|e| e.ts >= since) {
            total.add(e);
            by_agent
                .entry(agent_name(source).into())
                .or_default()
                .add(e);
            by_cwd.entry(e.cwd.to_string()).or_default().add(e);
            by_day.entry(day_key(e.ts)).or_default().add(e);
        }
    });
    UsageSummary {
        total,
        by_agent: keyed(by_agent),
        by_workspace_cwd: keyed(by_cwd),
        by_day: by_day
            .into_iter()
            .map(|(key, totals)| Keyed { key, totals })
            .collect(),
    }
}

/// Usage totals for "today", "7d" or "30d" (calendar days, local time).
#[tauri::command(async)]
pub fn usage_summary(range: String) -> Result<UsageSummary, String> {
    let days = match range.as_str() {
        "today" => 1,
        "7d" => 7,
        "30d" => 30,
        _ => return Err(format!("unknown range: {range}")),
    };
    Ok(summarize(days))
}

fn norm(path: &str) -> String {
    path.trim_end_matches(['\\', '/'])
        .replace('\\', "/")
        .to_lowercase()
}

/// Claude Code's project folder name: every non-alphanumeric char becomes '-'.
fn claude_slug(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Usage of one pane since its process started. Matched by working directory (the pane's
/// worktree when it has one) and time: two agents of the same kind in the same folder share totals.
#[tauri::command(async)]
pub fn pane_usage(
    pane_id: String,
    cwd: String,
    started_at: i64,
    agent: String,
) -> Result<PaneUsage, String> {
    let dir = crate::git::git_pane_dir(cwd, pane_id, true);
    let mut totals = Totals::default();
    let mut models = Vec::<String>::new();
    let mut add = |e: &Entry| {
        totals.add(e);
        if !e.model.is_empty() && !models.iter().any(|m| **m == *e.model) {
            models.push(e.model.to_string());
        }
    };
    let (source, root) = match agent.as_str() {
        "claude" => (
            Source::Claude,
            claude_projects().map(|p| p.join(claude_slug(&dir))),
        ),
        "codex" => (Source::Codex, codex_sessions()),
        _ => return Ok(PaneUsage { totals, models }),
    };
    let Some(root) = root else {
        return Ok(PaneUsage { totals, models });
    };
    let mut files = Vec::new();
    walk(&root, started_at, &mut files);
    let want = norm(&dir);
    for (path, len, mtime) in files {
        with_entries(&path, len, mtime, source, &mut |entries| {
            entries
                .iter()
                // The Claude folder is already per-cwd; Codex files must be filtered.
                .filter(|e| {
                    e.ts >= started_at && (source == Source::Claude || norm(&e.cwd) == want)
                })
                .for_each(&mut add);
        });
    }
    Ok(PaneUsage { totals, models })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn entry(model: &str, input: u64, output: u64, read: u64, w5: u64, w1h: u64) -> Entry {
        Entry {
            ts: 0,
            model: Arc::from(model),
            cwd: Arc::from(""),
            tokens: parse::Tokens {
                input,
                output,
                cache_write_5m: w5,
                cache_write_1h: w1h,
                cache_read: read,
            },
        }
    }

    #[test]
    fn cost_uses_price_table_and_tracks_unpriced() {
        let mut t = Totals::default();
        // Opus 5.5: $4 in, $20 out, $0.20 read, $5 (5m) / $8 (1h) write, per MTok.
        t.add(&entry(
            "claude-opus-5-5",
            1_000_000,
            1_000_000,
            1_000_000,
            1_000_000,
            1_000_000,
        ));
        assert!((t.cost - (4.0 + 20.0 + 0.2 + 5.0 + 8.0)).abs() < 1e-9);
        assert_eq!(t.tokens, 5_000_000);
        assert_eq!(t.cache_write, 2_000_000);
        t.add(&entry("gpt-6-astra", 10, 5, 0, 0, 0));
        assert_eq!(t.unpriced_tokens, 15);
        assert!((t.cost - 37.2).abs() < 1e-9);
    }

    #[test]
    fn slug_and_path_normalization() {
        assert_eq!(claude_slug("D:\\agentdeck"), "D--agentdeck");
        assert_eq!(
            claude_slug("C:\\Users\\Luca\\my.app"),
            "C--Users-Luca-my-app"
        );
        assert_eq!(norm("C:\\Work\\App\\"), norm("c:/work/app"));
    }

    #[test]
    fn day_buckets_cover_the_range() {
        let days: Vec<String> = empty_days(7).into_keys().collect();
        assert_eq!(days.len(), 7);
        assert_eq!(days[6], Local::now().format("%Y-%m-%d").to_string());
    }

    /// Manual check on this machine's real transcripts: `cargo test --lib real_data -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_data_timing() {
        for (days, label) in [(1, "today"), (30, "30d"), (30, "30d cached")] {
            let t = std::time::Instant::now();
            let s = summarize(days);
            println!(
                "{label}: {:?} tokens={} cost={:.2} unpriced={} agents={:?}",
                t.elapsed(),
                s.total.tokens,
                s.total.cost,
                s.total.unpriced_tokens,
                s.by_agent
                    .iter()
                    .map(|k| (&k.key, k.totals.tokens))
                    .collect::<Vec<_>>()
            );
        }
    }
}
