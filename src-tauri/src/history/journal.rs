//! Daily journal storage: finished tasks appended to `app_data_dir/journal/YYYY-MM-DD.json`,
//! today's git commits per workspace, and the Markdown export.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::agents;

const MAX_ENTRY_BYTES: usize = 64 * 1024;

static DIR: OnceLock<PathBuf> = OnceLock::new();
/// Serializes read-modify-write of the day files.
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
    Ok(dir.join(format!("{date}.json")))
}

fn read_day(path: &Path) -> Vec<Value> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn append(dir: &Path, date: &str, entry: Value) -> Result<(), String> {
    if serde_json::to_vec(&entry).map_err(|e| e.to_string())?.len() > MAX_ENTRY_BYTES {
        return Err("journal entry too large".into());
    }
    let path = day_file(dir, date)?;
    let _guard = WRITE.lock().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut entries = read_day(&path);
    entries.push(entry);
    let json = serde_json::to_vec(&entries).map_err(|e| e.to_string())?;
    // Write then rename: a crash mid-write never leaves a truncated day file.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// Appends a finished task to today's journal (local date).
#[tauri::command(async)]
pub fn journal_append(entry: Value) -> Result<(), String> {
    let dir = DIR.get().ok_or("journal unavailable")?;
    append(
        dir,
        &chrono::Local::now().format("%Y-%m-%d").to_string(),
        entry,
    )
}

/// Tasks saved on `date` (YYYY-MM-DD); empty when none.
#[tauri::command(async)]
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
#[tauri::command(async)]
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

/// Asks where to save (native dialog, so the path is always user-chosen) and writes `contents`.
/// Returns the saved path, or None when cancelled.
#[tauri::command(async)]
pub fn write_text_file(
    app: AppHandle,
    default_name: String,
    contents: String,
) -> Result<Option<String>, String> {
    let Some(target) = app
        .dialog()
        .file()
        .set_file_name(&default_name)
        .add_filter("Markdown", &["md"])
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = target.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, contents).map_err(|e| format!("cannot write file: {e}"))?;
    Ok(Some(path.to_string_lossy().into_owned()))
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
