//! Daily journal storage: finished tasks appended, one JSON per line, to `app_data_dir/journal/YYYY-MM-DD.jsonl`
//! (older `YYYY-MM-DD.json` arrays are still read), today's git commits per workspace, and the Markdown export.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use serde_json::Value;

use crate::agents;

const MAX_ENTRY_BYTES: usize = 64 * 1024;

static DIR: OnceLock<PathBuf> = OnceLock::new();
/// Serializes appends: lines of two tasks never interleave.
static WRITE: Mutex<()> = Mutex::new(());

pub fn init(app_data: &Path) {
    let _ = DIR.set(app_data.join("journal"));
}

fn valid_date(date: &str) -> bool {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok() && date.len() == 10
}

fn day_file(dir: &Path, date: &str) -> Result<PathBuf, String> {
    if !valid_date(date) {
        return Err("invalid date (YYYY-MM-DD)".into());
    }
    Ok(dir.join(format!("{date}.jsonl")))
}

/// Entries of a day: the legacy `.json` array first, then the `.jsonl` lines (unreadable lines skipped).
fn read_day(path: &Path) -> Vec<Value> {
    let mut entries: Vec<Value> = std::fs::read(path.with_extension("json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let lines = std::fs::read_to_string(path).unwrap_or_default();
    entries.extend(lines.lines().filter_map(|l| serde_json::from_str(l).ok()));
    entries
}

/// Deletes day files (any extension) dated more than `keep_days` before `today`.
pub(super) fn purge(dir: &Path, today: chrono::NaiveDate, keep_days: u64) {
    let Some(cutoff) = today.checked_sub_days(chrono::Days::new(keep_days)) else {
        return;
    };
    for file in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = file.file_name();
        let date = name.to_str().and_then(|n| n.get(..10));
        if date.and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).is_some_and(|d| d < cutoff) {
            let _ = std::fs::remove_file(file.path());
        }
    }
}

fn append(dir: &Path, date: &str, entry: Value) -> Result<(), String> {
    let mut line = serde_json::to_vec(&entry).map_err(|e| e.to_string())?;
    if line.len() > MAX_ENTRY_BYTES {
        return Err("journal entry too large".into());
    }
    let path = day_file(dir, date)?;
    line.push(b'\n');
    let _guard = WRITE.lock().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(&line))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Appends a finished task to today's journal (local date).
pub fn journal_append(entry: Value) -> Result<(), String> {
    let dir = DIR.get().ok_or("journal unavailable")?;
    append(
        dir,
        &chrono::Local::now().format("%Y-%m-%d").to_string(),
        entry,
    )
}

/// Tasks saved on `date` (YYYY-MM-DD); empty when none.
pub fn journal_read(date: String) -> Result<Vec<Value>, String> {
    let dir = DIR.get().ok_or("journal unavailable")?;
    Ok(read_day(&day_file(dir, &date)?))
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Commit {
    pub hash: String,
    /// ISO 8601, author date.
    pub time: String,
    pub author: String,
    pub subject: String,
}

fn parse_log(out: &str) -> Vec<Commit> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\x1f');
            Some(Commit {
                hash: f.next()?.into(),
                time: f.next()?.into(),
                author: f.next()?.into(),
                subject: f.next()?.into(),
            })
        })
        .collect()
}

/// Commits made since local midnight on any local branch (agents' worktree branches included).
/// Not a git repository: empty list.
pub fn journal_git_commits(cwd: String) -> Result<Vec<Commit>, String> {
    let Some(root) = agents::find_repo_root(Path::new(&cwd)) else {
        return Ok(Vec::new());
    };
    let out = agents::git(
        &root,
        &[
            "log",
            "--branches",
            "--since=midnight",
            "--max-count=200",
            "--pretty=format:%h%x1f%aI%x1f%an%x1f%s",
        ],
    )?;
    Ok(parse_log(&out))
}

/// Writes `contents` to a path the user picked in the native save dialog (the UI shows it).
pub fn write_text_file(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| format!("cannot write file: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_and_reads_back_a_day() {
        let dir = std::env::temp_dir().join(format!("agentdeck-journal-{}", std::process::id()));
        append(&dir, "2026-10-26", serde_json::json!({ "prompt": "a" })).unwrap();
        append(&dir, "2026-10-26", serde_json::json!({ "prompt": "b" })).unwrap();
        let day = read_day(&day_file(&dir, "2026-10-26").unwrap());
        assert_eq!(day.len(), 2);
        assert_eq!(day[1]["prompt"], "b");
        // A day still in the old array format: read first, new lines after it.
        std::fs::write(dir.join("2026-10-25.json"), r#"[{"prompt":"old"}]"#).unwrap();
        append(&dir, "2026-10-25", serde_json::json!({ "prompt": "new" })).unwrap();
        let day = read_day(&day_file(&dir, "2026-10-25").unwrap());
        assert_eq!(day.iter().map(|e| e["prompt"].as_str().unwrap()).collect::<Vec<_>>(), ["old", "new"]);
        // 90-day purge: by the date in the name, any extension; other files stay.
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        purge(&dir, chrono::NaiveDate::from_ymd_opt(2027, 1, 24).unwrap(), 90);
        assert!(!dir.join("2026-10-25.json").exists() && !dir.join("2026-10-25.jsonl").exists());
        assert!(dir.join("2026-10-26.jsonl").exists() && dir.join("notes.txt").exists());
        assert!(day_file(&dir, "../../etc").is_err());
        assert!(day_file(&dir, "2026-1-1").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_git_log_format() {
        let out = "abc123\x1f2026-10-26T10:00:00+02:00\x1fLuca\x1ffeat: x | y\nbad line";
        assert_eq!(
            parse_log(out),
            vec![Commit {
                hash: "abc123".into(),
                time: "2026-10-26T10:00:00+02:00".into(),
                author: "Luca".into(),
                subject: "feat: x | y".into(),
            }]
        );
    }
}
