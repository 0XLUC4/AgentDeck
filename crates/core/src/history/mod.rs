//! Terminal history: every pane run's output saved as plain text under
//! `app_data_dir/history/<paneId>/<runStart>[.<part>].log` (5 MB per part, kept 30 days),
//! with `<runStart>.json` metadata. Plus a global substring search over it.

pub(crate) mod ansi;
pub mod journal;

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use crate::AppHandle;

const PART_BYTES: u64 = 5 * 1024 * 1024;
const KEEP: Duration = Duration::from_secs(30 * 24 * 3600);
const MAX_TOTAL_BYTES: u64 = 500 * 1024 * 1024;
const JOURNAL_KEEP_DAYS: u64 = 90;
const PURGE_EVERY: Duration = Duration::from_secs(3600);
const SEARCH_BUDGET: Duration = Duration::from_secs(3);
const FLUSH_EVERY: Duration = Duration::from_secs(2);
const DISABLED_MARKER: &str = ".disabled";
const MAX_HIT_CHARS: usize = 400;
const READ_WINDOW: usize = 2000;

static ENABLED: AtomicBool = AtomicBool::new(true);
static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Resolves the history folder, reads the on/off marker and purges expired or excess runs
/// in a background thread, at startup then hourly.
pub fn init(app: &AppHandle) {
    let Ok(data) = app.app_data_dir() else {
        eprintln!("history: no app data dir, recording disabled");
        return;
    };
    journal::init(&data);
    let journal_dir = data.join("journal");
    let root = data.join("history");
    if let Err(e) = fs::create_dir_all(&root) {
        eprintln!("history: cannot create {}: {e}", root.display());
        return;
    }
    ENABLED.store(!root.join(DISABLED_MARKER).exists(), Ordering::Relaxed);
    let _ = ROOT.set(root.clone());
    std::thread::spawn(move || {
        loop {
            purge(&root, |modified| {
                modified.elapsed().is_ok_and(|age| age > KEEP)
            });
            purge_oversize(&root, MAX_TOTAL_BYTES);
            journal::purge(&journal_dir, chrono::Local::now().date_naive(), JOURNAL_KEEP_DAYS);
            std::thread::sleep(PURGE_EVERY);
        }
    });
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Deletes files matching `expired(mtime)` in every pane folder, then empty folders.
/// Files still open by a running pane fail to delete on Windows: ignored.
fn purge(root: &Path, expired: impl Fn(SystemTime) -> bool) {
    let Ok(panes) = fs::read_dir(root) else {
        return;
    };
    for pane in panes.flatten().filter(|e| e.path().is_dir()) {
        for file in fs::read_dir(pane.path()).into_iter().flatten().flatten() {
            let old = file
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(&expired);
            if old {
                let _ = fs::remove_file(file.path());
            }
        }
        let _ = fs::remove_dir(pane.path()); // only succeeds when empty
    }
}

/// Keeps the newest log parts totalling at most `max_bytes`, deletes the older ones.
/// Orphan `.json` metadata is tiny and goes with the age purge.
fn purge_oversize(root: &Path, max_bytes: u64) {
    let mut total = 0u64;
    for (path, _, len) in log_files(root) {
        total += len;
        if total > max_bytes {
            let _ = fs::remove_file(path);
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunMeta {
    pub pane_id: String,
    pub title: String,
    pub agent: String,
    pub cwd: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

/// Writes one pane run to disk. Lives in the PTY batching thread; every I/O error is logged
/// once and stops the recording, never the terminal.
pub struct Recorder {
    dir: PathBuf,
    meta: RunMeta,
    part: u32,
    file: Option<BufWriter<File>>,
    written: u64,
    strip: ansi::Stripper,
    text: String,
    last_flush: Instant,
    dirty: bool,
}

impl Recorder {
    /// None when recording is off or the folder can't be created.
    pub fn start(pane_id: &str, agent: &str, cwd: &str, title: Option<String>) -> Option<Self> {
        if !ENABLED.load(Ordering::Relaxed) {
            return None;
        }
        Self::start_in(ROOT.get()?, pane_id, agent, cwd, title)
    }

    fn start_in(
        root: &Path,
        pane_id: &str,
        agent: &str,
        cwd: &str,
        title: Option<String>,
    ) -> Option<Self> {
        let dir = root.join(pane_id);
        if let Err(e) = fs::create_dir_all(&dir) {
            eprintln!("history: cannot create {}: {e}", dir.display());
            return None;
        }
        let mut rec = Self {
            dir,
            meta: RunMeta {
                pane_id: pane_id.into(),
                title: title.unwrap_or_default(),
                agent: agent.into(),
                cwd: cwd.into(),
                started_at: now_ms(),
                ended_at: None,
            },
            part: 0,
            file: None,
            written: 0,
            strip: ansi::Stripper::default(),
            text: String::new(),
            last_flush: Instant::now(),
            dirty: false,
        };
        rec.save_meta();
        rec.open_part();
        Some(rec)
    }

    fn part_path(&self) -> PathBuf {
        let stem = self.meta.started_at;
        match self.part {
            0 => self.dir.join(format!("{stem}.log")),
            n => self.dir.join(format!("{stem}.{n}.log")),
        }
    }

    fn open_part(&mut self) {
        let path = self.part_path();
        self.file = File::options()
            .create(true)
            .append(true)
            .open(&path)
            .map(|f| BufWriter::with_capacity(64 * 1024, f))
            .map_err(|e| eprintln!("history: cannot open {}: {e}", path.display()))
            .ok();
        self.written = 0;
    }

    fn save_meta(&self) {
        let path = self.dir.join(format!("{}.json", self.meta.started_at));
        let json = serde_json::to_vec(&self.meta).unwrap_or_default();
        if let Err(e) = fs::write(&path, json) {
            eprintln!("history: cannot write {}: {e}", path.display());
        }
    }

    pub fn write(&mut self, output: &str) {
        if !ENABLED.load(Ordering::Relaxed) {
            self.file = None; // turned off mid-run: stop here
            return;
        }
        let Some(file) = self.file.as_mut() else {
            return;
        };
        self.text.clear();
        self.strip.push(output, &mut self.text);
        if let Err(e) = file.write_all(self.text.as_bytes()) {
            eprintln!("history: write failed, recording stopped: {e}");
            self.file = None;
            return;
        }
        self.written += self.text.len() as u64;
        self.dirty = true;
        if self.written >= PART_BYTES {
            let _ = file.flush();
            self.part += 1;
            self.open_part();
        }
    }

    /// Flushes at most every 2 s so the search sees recent output without a write per frame.
    pub fn flush_if_due(&mut self) {
        if self.dirty && self.last_flush.elapsed() >= FLUSH_EVERY {
            if let Some(file) = self.file.as_mut() {
                let _ = file.flush();
            }
            self.dirty = false;
            self.last_flush = Instant::now();
        }
    }

    pub fn finish(mut self) {
        if let Some(mut file) = self.file.take() {
            let _ = file.flush();
        }
        self.meta.ended_at = Some(now_ms());
        self.save_meta();
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// `<paneId>/<file stem>`: one log part.
    pub run_id: String,
    pub meta: Option<RunMeta>,
    /// 0-based line in the part.
    pub line: usize,
    pub text: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

fn clip(line: &str) -> String {
    line.chars().take(MAX_HIT_CHARS).collect()
}

fn read_meta(pane_dir: &Path, part_stem: &str) -> Option<RunMeta> {
    let run = part_stem.split('.').next()?;
    serde_json::from_slice(&fs::read(pane_dir.join(format!("{run}.json"))).ok()?).ok()
}

/// Every log part with its mtime and size, newest first.
fn log_files(root: &Path) -> Vec<(PathBuf, SystemTime, u64)> {
    let mut files: Vec<_> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .flat_map(|pane| fs::read_dir(pane.path()).into_iter().flatten().flatten())
        .filter(|f| f.path().extension().is_some_and(|x| x == "log"))
        .filter_map(|f| {
            let meta = f.metadata().ok()?;
            Some((f.path(), meta.modified().ok()?, meta.len()))
        })
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1));
    files
}

fn run_id(path: &Path) -> Option<String> {
    let pane = path.parent()?.file_name()?.to_str()?;
    let stem = path.file_stem()?.to_str()?;
    Some(format!("{pane}/{stem}"))
}

/// `needle` is already lowercase. ASCII lines are matched in place, no allocation.
fn contains_ci(line: &str, needle: &str) -> bool {
    if line.is_ascii() && needle.is_ascii() {
        let n = needle.as_bytes();
        return n.is_empty()
            || line
                .as_bytes()
                .windows(n.len())
                .any(|w| w.eq_ignore_ascii_case(n));
    }
    line.to_lowercase().contains(needle)
}

/// Case-insensitive substring search, newest runs first, stopping at `limit` hits
/// or after `SEARCH_BUDGET`.
fn search(root: &Path, query: &str, limit: usize) -> Vec<SearchHit> {
    let needle = query.to_lowercase();
    let mut hits = Vec::new();
    if needle.trim().is_empty() || limit == 0 {
        return hits;
    }
    let started = Instant::now();
    let mut metas: HashMap<PathBuf, Option<RunMeta>> = HashMap::new();
    'files: for (path, _, _) in log_files(root) {
        let Ok(file) = File::open(&path) else {
            continue;
        };
        let (Some(id), Some(dir), Some(stem)) = (
            run_id(&path),
            path.parent(),
            path.file_stem().and_then(|s| s.to_str()),
        ) else {
            continue;
        };
        let run = stem.split('.').next().unwrap_or(stem);
        let meta = metas
            .entry(dir.join(run))
            .or_insert_with(|| read_meta(dir, stem))
            .clone();
        let mut reader = BufReader::new(file);
        let mut raw = Vec::new();
        let mut prev_raw = Vec::new();
        let mut awaiting_after = false;
        let mut line_no = 0;
        while reader.read_until(b'\n', &mut raw).is_ok_and(|n| n > 0) {
            let text = String::from_utf8_lossy(&raw);
            let line = text.trim_end_matches(['\n', '\r']);
            if awaiting_after {
                if let Some(hit) = hits.last_mut() {
                    hit.after = Some(clip(line));
                }
                awaiting_after = false;
                if hits.len() >= limit {
                    break 'files;
                }
            }
            if contains_ci(line, &needle) {
                let prev = String::from_utf8_lossy(&prev_raw);
                let prev = prev.trim_end_matches(['\n', '\r']);
                hits.push(SearchHit {
                    run_id: id.clone(),
                    meta: meta.clone(),
                    line: line_no,
                    text: clip(line),
                    before: (!prev.trim().is_empty()).then(|| clip(prev)),
                    after: None,
                });
                awaiting_after = true;
            }
            drop(text);
            std::mem::swap(&mut raw, &mut prev_raw);
            raw.clear();
            line_no += 1;
            if line_no % 4096 == 0 && started.elapsed() > SEARCH_BUDGET {
                break 'files;
            }
        }
        if hits.len() >= limit || started.elapsed() > SEARCH_BUDGET {
            break;
        }
    }
    hits
}

pub fn history_search(query: String, limit: usize) -> Vec<SearchHit> {
    ROOT.get()
        .map(|root| search(root, &query, limit.min(500)))
        .unwrap_or_default()
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RunWindow {
    pub meta: Option<RunMeta>,
    pub start_line: usize,
    pub total_lines: usize,
    pub lines: Vec<String>,
}

/// `<paneId>/<stem>` with safe characters only: never escapes the history folder.
fn part_path(root: &Path, run_id: &str) -> Result<PathBuf, String> {
    let safe = |s: &str| {
        !s.is_empty()
            && !s.starts_with('.')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    };
    match run_id.split_once('/') {
        Some((pane, stem)) if safe(pane) && safe(stem) && !stem.contains("..") => {
            Ok(root.join(pane).join(format!("{stem}.log")))
        }
        _ => Err("invalid run id".into()),
    }
}

fn read_window(root: &Path, run_id: &str, around: usize) -> Result<RunWindow, String> {
    let path = part_path(root, run_id)?;
    let bytes = fs::read(&path).map_err(|e| format!("cannot read file: {e}"))?;
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    let start = around
        .saturating_sub(READ_WINDOW / 2)
        .min(all.len().saturating_sub(READ_WINDOW));
    let dir = path.parent().ok_or("invalid run id")?;
    Ok(RunWindow {
        meta: read_meta(dir, run_id.split_once('/').map_or("", |(_, s)| s)),
        start_line: start,
        total_lines: all.len(),
        lines: all
            .iter()
            .skip(start)
            .take(READ_WINDOW)
            .map(|l| l.to_string())
            .collect(),
    })
}

/// A window of up to 2000 lines of one log part around `around_line`.
pub fn history_read(run_id: String, around_line: usize) -> Result<RunWindow, String> {
    read_window(
        ROOT.get().ok_or("history unavailable")?,
        &run_id,
        around_line,
    )
}

pub fn history_get_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Persisted as a marker file; applies immediately to running panes.
pub fn history_set_enabled(enabled: bool) -> Result<(), String> {
    let root = ROOT.get().ok_or("history unavailable")?;
    let marker = root.join(DISABLED_MARKER);
    if enabled {
        if marker.exists() {
            fs::remove_file(&marker).map_err(|e| e.to_string())?;
        }
    } else {
        fs::write(&marker, b"").map_err(|e| e.to_string())?;
    }
    ENABLED.store(enabled, Ordering::Relaxed);
    Ok(())
}

/// Deletes every saved run (files of running panes may survive until they exit).
pub fn history_clear() -> Result<(), String> {
    let root = ROOT.get().ok_or("history unavailable")?;
    purge(root, |_| true);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("agentdeck-history-{name}-{}", now_ms()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn records_plain_text_and_rotates_parts() {
        let root = temp_root("rotate");
        let mut rec =
            Recorder::start_in(&root, "pane-1", "claude", "D:\\p", Some("T".into())).unwrap();
        rec.write("\x1b[32mhello\x1b[0m\r\n");
        let chunk = "x".repeat(1024 * 1024) + "\n";
        for _ in 0..6 {
            rec.write(&chunk); // crosses 5 MB once
        }
        rec.write("tail line\n");
        let started = rec.meta.started_at;
        rec.finish();
        let dir = root.join("pane-1");
        let first = fs::read_to_string(dir.join(format!("{started}.log"))).unwrap();
        assert!(first.starts_with("hello\n"));
        assert!(first.len() as u64 >= PART_BYTES);
        let second = fs::read_to_string(dir.join(format!("{started}.1.log"))).unwrap();
        assert!(second.ends_with("tail line\n"));
        let meta: RunMeta =
            serde_json::from_slice(&fs::read(dir.join(format!("{started}.json"))).unwrap())
                .unwrap();
        assert_eq!(meta.title, "T");
        assert!(meta.ended_at.is_some());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn search_finds_with_context_and_stops_at_limit() {
        let root = temp_root("search");
        let mut rec = Recorder::start_in(&root, "p2", "codex", "C:\\w", None).unwrap();
        rec.write("before\nError: Boom\nafter\nnothing\nerror again\n");
        rec.finish();
        let hits = search(&root, "ERROR", 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].text, "Error: Boom");
        assert_eq!(hits[0].before.as_deref(), Some("before"));
        assert_eq!(hits[0].after.as_deref(), Some("after"));
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[0].meta.as_ref().unwrap().agent, "codex");
        assert_eq!(search(&root, "error", 1).len(), 1);
        assert!(search(&root, "   ", 10).is_empty());

        let window = read_window(&root, &hits[1].run_id, 4).unwrap();
        assert_eq!(window.start_line, 0);
        assert_eq!(window.lines[4], "error again");
        assert!(read_window(&root, "../x", 0).is_err());
        assert!(read_window(&root, "p2/..", 0).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn purge_removes_expired_files_and_empty_dirs() {
        let root = temp_root("purge");
        let rec = Recorder::start_in(&root, "p3", "shell", "", None).unwrap();
        rec.finish();
        purge(&root, |_| true);
        assert!(!root.join("p3").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn size_purge_deletes_oldest_parts_first() {
        let root = temp_root("oversize");
        let dir = root.join("p4");
        fs::create_dir_all(&dir).unwrap();
        let base = SystemTime::now() - Duration::from_secs(3600);
        for i in 0..4u64 {
            let path = dir.join(format!("{i}.log"));
            fs::write(&path, vec![b'x'; 100]).unwrap();
            // Distinct mtimes: part 0 oldest, part 3 newest.
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(base + Duration::from_secs(i * 60))
                .unwrap();
        }
        purge_oversize(&root, 250);
        let left: Vec<bool> = (0..4)
            .map(|i| dir.join(format!("{i}.log")).exists())
            .collect();
        assert_eq!(left, [false, false, true, true]);
        purge_oversize(&root, 1000);
        assert!(dir.join("3.log").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn matches_case_insensitively_ascii_and_unicode() {
        assert!(contains_ci("Error: Boom", "boom"));
        assert!(!contains_ci("Error", "errors"));
        assert!(contains_ci("ÉCHEC total", &"échec".to_lowercase()));
        assert!(contains_ci("plain", ""));
    }
}
