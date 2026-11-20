//! Pure ranking of the finders: Quick Open (Ctrl+P) file search, Worktree Jump (Ctrl+J) panes and
//! sessions, and the new-tab omnibox input (URL, web search).

use std::cmp::Ordering;

use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::ActivityKey;

use super::fuzzy::fuzzy_score;

/// A project file, relative to the searched folder with forward slashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    /// Matched by `.gitignore`: listed after the tracked files.
    pub ignored: bool,
}

/// A Quick Open query: space-separated parts matched across the path, plus `:line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileQuery {
    pub parts: Vec<String>,
    /// 1-based line typed after the path (`main.rs:42`, `main.rs:42:7`).
    pub line: Option<u32>,
}

/// `text` ends with `:<digits>`: (before, number).
fn split_number(text: &str) -> Option<(&str, u32)> {
    let (head, tail) = text.rsplit_once(':')?;
    if tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((head, tail.parse().ok()?))
}

pub fn parse_file_query(query: &str) -> FileQuery {
    let text = query.trim();
    let (text, line) = match split_number(text) {
        // `path:line:col`: the first number is the line.
        Some((head, n)) => match split_number(head) {
            Some((path, line)) => (path, Some(line)),
            None => (head, Some(n)),
        },
        None => (text.trim_end_matches(':'), None),
    };
    let parts = text.split_whitespace().map(|part| part.replace('\\', "/")).collect();
    FileQuery { parts, line }
}

/// Score of `path` for `query`, None when a part does not match. The last part prefers the file
/// name; the other parts match anywhere in the path (folders), so "src btn" finds src/ui/button.rs.
pub fn score_path(query: &FileQuery, path: &str) -> Option<f64> {
    let Some((last, rest)) = query.parts.split_last() else {
        return Some(0.0);
    };
    let name = path.rsplit('/').next().unwrap_or(path);
    let mut score = match fuzzy_score(last, name) {
        Some(s) if !last.contains('/') => s + 10.0,
        _ => fuzzy_score(last, path)?,
    };
    for part in rest {
        score += fuzzy_score(part, path)?;
    }
    Some(score)
}

/// Recently opened files kept per folder.
pub const MAX_RECENT_FILES: usize = 20;

fn recency_bonus(recent: &[String], path: &str) -> f64 {
    recent
        .iter()
        .position(|r| r == path)
        .map_or(0.0, |i| (MAX_RECENT_FILES - i.min(MAX_RECENT_FILES)) as f64 * 0.5)
}

/// Indices of the best `limit` files: tracked before gitignored, then match score plus recency.
/// Empty query: recently opened files first, then the list order. None once `cancelled()` (a newer
/// search replaced this one).
pub fn rank_files(
    files: &[FileEntry],
    query: &FileQuery,
    recent: &[String],
    limit: usize,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<usize>> {
    if query.parts.is_empty() {
        let mut out: Vec<usize> = recent.iter().filter_map(|r| files.iter().position(|f| &f.path == r)).collect();
        let rest: Vec<usize> = (0..files.len()).filter(|i| !out.contains(i)).take(limit).collect();
        out.extend(rest);
        out.truncate(limit);
        return Some(out);
    }
    let mut scored = Vec::new();
    for (i, file) in files.iter().enumerate() {
        if i % 1024 == 0 && cancelled() {
            return None;
        }
        if let Some(score) = score_path(query, &file.path) {
            scored.push((i, file.ignored, score + recency_bonus(recent, &file.path)));
        }
    }
    scored.sort_by(|a, b| a.1.cmp(&b.1).then(b.2.total_cmp(&a.2)));
    Some(scored.into_iter().take(limit).map(|(i, _, _)| i).collect())
}

/// Parent folder of `path` ("" at the root), cut from the left to `max` chars: "…/ui/button".
pub fn parent_label(path: &str, max: usize) -> String {
    let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let count = dir.chars().count();
    if count <= max {
        return dir.to_owned();
    }
    let keep = max.saturating_sub(1);
    format!("…{}", dir.chars().skip(count - keep).collect::<String>())
}

/// Jump order of a status: needs you (waiting, error), working, done, idle, stopped.
pub fn status_rank(key: ActivityKey) -> u8 {
    match key {
        ActivityKey::Status(PaneStatus::Waiting | PaneStatus::Error) => 0,
        ActivityKey::Status(PaneStatus::Working) => 1,
        ActivityKey::Status(PaneStatus::Done) => 2,
        ActivityKey::Status(PaneStatus::Idle) => 3,
        ActivityKey::Exited => 4,
    }
}

/// What the empty Jump palette knows of a pane.
#[derive(Debug, Clone)]
pub struct PaneInfo {
    pub id: String,
    pub key: ActivityKey,
    /// Last time it had the focus (ms), 0 when never.
    pub focused_at: i64,
}

/// "Recent chats & terminals": by status (needs you first), then most recently focused. The current
/// pane is left out while idle: there is nothing to jump to.
pub fn recent_panes(panes: &[PaneInfo], current: Option<&str>) -> Vec<String> {
    let mut list: Vec<&PaneInfo> = panes
        .iter()
        .filter(|p| !(Some(p.id.as_str()) == current && p.key == ActivityKey::Status(PaneStatus::Idle)))
        .collect();
    list.sort_by(|a, b| status_rank(a.key).cmp(&status_rank(b.key)).then(b.focused_at.cmp(&a.focused_at)));
    list.into_iter().map(|p| p.id.clone()).collect()
}

/// Above any location-only score: a match on a pane's own title or content always ranks first.
const OWN_MATCH: f64 = 1000.0;

/// Pane score: its title and content (`own`) rank above its location (`project · session`).
pub fn pane_score(query: &str, own: &str, location: &str) -> Option<f64> {
    fuzzy_score(query, own).map(|s| s + OWN_MATCH).or_else(|| fuzzy_score(query, location))
}

/// Best score first; equal (rounded) scores: most recently focused first.
pub fn by_score_then_recency(a: (f64, i64), b: (f64, i64)) -> Ordering {
    b.0.round().total_cmp(&a.0.round()).then(b.1.cmp(&a.1))
}

/// What the omnibox can do with its text besides tabs, files and agents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebInput {
    /// The text is an address: open it.
    pub url: Option<String>,
    /// Search the web for this text (several words, or forced with a leading `?`).
    pub search: Option<String>,
    /// `?` prefix: only the web search row.
    pub web_only: bool,
}

pub fn classify_web(query: &str) -> WebInput {
    let text = query.trim();
    if let Some(rest) = text.strip_prefix('?') {
        let rest = rest.trim();
        return WebInput { url: None, search: (!rest.is_empty()).then(|| rest.to_owned()), web_only: true };
    }
    WebInput { url: as_url(text), search: text.contains(char::is_whitespace).then(|| text.to_owned()), web_only: false }
}

/// File extensions that look like top-level domains: "main.rs" is a file, not a site.
const FILE_EXTS: &[&str] = &[
    "rs", "md", "ts", "js", "py", "json", "toml", "txt", "tsx", "jsx", "go", "java", "css", "html", "yml", "yaml",
    "lock", "sh", "log", "cs", "kt", "rb", "php", "sql", "svg", "png", "mjs", "cjs", "vue",
];

/// A typed address as a URL: http(s) as is, "localhost:3000" over http, "example.com/x" over https.
pub fn as_url(text: &str) -> Option<String> {
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let lower = text.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(text.to_owned());
    }
    let host = text.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("").to_lowercase();
    if host == "localhost" || host == "127.0.0.1" {
        return Some(format!("http://{text}"));
    }
    let (name, tld) = host.rsplit_once('.')?;
    let ok = !name.is_empty()
        && (2..=24).contains(&tld.len())
        && tld.chars().all(|c| c.is_ascii_alphabetic())
        && !FILE_EXTS.contains(&tld);
    ok.then(|| format!("https://{text}"))
}

/// DuckDuckGo results for `text` (no search engine setting in the app).
pub fn web_search_url(text: &str) -> String {
    let query: String = url::form_urlencoded::byte_serialize(text.as_bytes()).collect();
    format!("https://duckduckgo.com/?q={query}")
}

#[cfg(test)]
mod tests {
    use super::{
        ActivityKey, FileEntry, FileQuery, PaneInfo, PaneStatus, as_url, by_score_then_recency, classify_web,
        pane_score, parent_label, parse_file_query, rank_files, recent_panes, score_path, web_search_url,
    };

    fn q(parts: &[&str]) -> FileQuery {
        FileQuery { parts: parts.iter().map(|p| (*p).to_owned()).collect(), line: None }
    }

    fn files(paths: &[(&str, bool)]) -> Vec<FileEntry> {
        paths.iter().map(|(p, ignored)| FileEntry { path: (*p).to_owned(), ignored: *ignored }).collect()
    }

    #[test]
    fn query_splits_parts_and_line() {
        assert_eq!(parse_file_query("src btn"), q(&["src", "btn"]));
        assert_eq!(parse_file_query("main.rs:42"), FileQuery { parts: vec!["main.rs".into()], line: Some(42) });
        assert_eq!(parse_file_query("main.rs:42:7").line, Some(42));
        assert_eq!(parse_file_query("main.rs:"), q(&["main.rs"]));
        assert_eq!(parse_file_query(r"src\ui"), q(&["src/ui"]));
        assert_eq!(parse_file_query("  "), q(&[]));
    }

    #[test]
    fn multi_part_matches_across_segments() {
        assert!(score_path(&q(&["src", "button"]), "src/ui/button.rs").is_some());
        assert!(score_path(&q(&["lib", "button"]), "src/ui/button.rs").is_none());
        let by_name = score_path(&q(&["button"]), "src/ui/button.rs").unwrap();
        let by_dir = score_path(&q(&["button"]), "button/src/index.rs").unwrap();
        assert!(by_name > by_dir);
    }

    #[test]
    fn tracked_files_rank_before_ignored_ones() {
        let list = files(&[("dist/app.js", true), ("src/app.ts", false)]);
        let got = rank_files(&list, &q(&["app"]), &[], 10, &|| false).unwrap();
        assert_eq!(got, [1, 0]);
    }

    #[test]
    fn recency_lifts_equal_matches_and_leads_empty_queries() {
        let list = files(&[("a/mod.rs", false), ("b/mod.rs", false)]);
        let recent = vec!["b/mod.rs".to_owned()];
        assert_eq!(rank_files(&list, &q(&["mod"]), &recent, 10, &|| false).unwrap()[0], 1);
        assert_eq!(rank_files(&list, &q(&[]), &recent, 10, &|| false).unwrap(), [1, 0]);
    }

    #[test]
    fn stale_searches_stop() {
        let list = files(&[("a.rs", false)]);
        assert!(rank_files(&list, &q(&["a"]), &[], 10, &|| true).is_none());
    }

    #[test]
    fn parent_label_cuts_from_the_left() {
        assert_eq!(parent_label("main.rs", 10), "");
        assert_eq!(parent_label("src/ui/button.rs", 10), "src/ui");
        assert_eq!(parent_label("crates/app/src/palette/x.rs", 10), "…c/palette");
    }

    fn pane(id: &str, status: PaneStatus, focused_at: i64) -> PaneInfo {
        PaneInfo { id: id.into(), key: ActivityKey::Status(status), focused_at }
    }

    #[test]
    fn recent_panes_put_needs_you_first_and_skip_the_idle_current_one() {
        let panes = [
            pane("idle", PaneStatus::Idle, 5),
            pane("work", PaneStatus::Working, 1),
            pane("wait", PaneStatus::Waiting, 0),
            pane("done", PaneStatus::Done, 2),
            pane("cur", PaneStatus::Idle, 9),
        ];
        assert_eq!(recent_panes(&panes, Some("cur")), ["wait", "work", "done", "idle"]);
        assert_eq!(recent_panes(&panes, None)[3], "cur");
    }

    #[test]
    fn own_matches_rank_above_location_and_recency_breaks_ties() {
        let own = pane_score("api", "Fix api tests", "web · Session 1").unwrap();
        let location = pane_score("api", "Claude", "api · Session 1").unwrap();
        assert!(own > location);
        assert_eq!(by_score_then_recency((3.0, 1), (3.0, 9)), std::cmp::Ordering::Greater);
    }

    #[test]
    fn web_input_detects_urls_and_searches() {
        assert_eq!(as_url("localhost:3000/x").as_deref(), Some("http://localhost:3000/x"));
        assert_eq!(as_url("example.com").as_deref(), Some("https://example.com"));
        assert_eq!(as_url("HTTPS://a.dev").as_deref(), Some("HTTPS://a.dev"));
        assert_eq!(as_url("main.rs"), None);
        assert_eq!(as_url("button"), None);
        let multi = classify_web("rust borrow checker");
        assert_eq!((multi.url, multi.search.as_deref(), multi.web_only), (None, Some("rust borrow checker"), false));
        let forced = classify_web("? example.com");
        assert_eq!((forced.url, forced.search.as_deref(), forced.web_only), (None, Some("example.com"), true));
        assert_eq!(web_search_url("a b&c"), "https://duckduckgo.com/?q=a+b%26c");
    }
}
