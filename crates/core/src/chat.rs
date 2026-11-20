//! Chat mode: the JSONL event streams of `claude -p --output-format stream-json` and
//! `codex exec --json`, parsed into one small typed model the chat view renders.
//! Pure: no process, no UI. Unknown lines and fields are ignored (both formats grow over time).
//! Also the chat's small pure helpers: line diffs of edits, plans, saved transcripts, composer
//! triggers (`/`, `@`), and the model / permission flags of a chat agent.

use std::path::Path;
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agents::AgentKind;

/// Chat panes currently launched. They run outside the PTY map, so the hook server and the MCP
/// server check here too, or they would reject a chat pane's hooks (no Working status, no
/// permission cards) and its orchestrator tools.
static LIVE: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);

/// Marks a chat pane as launched (`true`) or gone (`false`).
pub fn set_live(pane_id: &str, live: bool) {
    let mut panes = LIVE.lock().unwrap_or_else(PoisonError::into_inner);
    if live {
        panes.insert(pane_id.to_owned());
    } else {
        panes.remove(pane_id);
    }
}

/// The pane is a launched chat pane.
pub fn is_live(pane_id: &str) -> bool {
    LIVE.lock().unwrap_or_else(PoisonError::into_inner).contains(pane_id)
}

/// Longest tool output kept in the transcript (bytes); the rest is cut.
const MAX_OUTPUT: usize = 16 * 1024;
/// Longest one-line summary of a tool call's input.
const MAX_SUMMARY: usize = 120;
/// Longest patch kept per edit (bytes).
const MAX_DIFF: usize = 8 * 1024;
/// Line-diff table size above which a change shows as all removed then all added.
const MAX_LCS: usize = 1_000_000;
/// Unchanged lines shown around a change.
const CONTEXT: usize = 2;
/// Items and tool output kept in a saved transcript.
const MAX_SAVED_ITEMS: usize = 400;
const MAX_SAVED_OUTPUT: usize = 2 * 1024;

/// One thing an agent reported.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    /// Conversation id, to resume it (Claude `session_id`, Codex `thread_id`).
    Session(String),
    /// Streamed piece of the assistant's current text block.
    TextDelta(String),
    /// A whole assistant text block (replaces the streamed pieces of that block).
    Text(String),
    ToolStart { id: String, name: String, input: String, edits: Vec<FileEdit>, plan: Option<Vec<PlanStep>> },
    ToolResult { id: String, output: String, error: bool },
    /// Non-fatal message from the CLI (config warnings...).
    Notice(String),
    /// The turn is over; `Some` when it failed.
    TurnEnd { error: Option<String> },
}

/// A row of the chat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Item {
    /// The user's message and the image files attached to it.
    User { text: String, images: Vec<String> },
    Assistant(String),
    Tool(Tool),
    Notice(String),
    Error(String),
    /// End of a turn that took `secs` seconds.
    Done { secs: u64 },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub id: String,
    pub name: String,
    /// Short one-line summary of the input (command, path...).
    pub input: String,
    /// None while running.
    pub output: Option<String>,
    pub error: bool,
    /// Files this call changes (Edit / MultiEdit / Write, Codex file_change).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<FileEdit>,
    /// The checklist this call sets (TodoWrite, update_plan, Codex todo_list).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Vec<PlanStep>>,
}

/// One file changed by a tool call: line counts and a capped patch (`+`, `-`, ` ` prefixed lines).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileEdit {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    pub diff: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanStep {
    pub text: String,
    pub status: StepStatus,
}

/// What the chat view shows, built by applying events in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    pub items: Vec<Item>,
    /// A turn is running.
    pub busy: bool,
    pub session: Option<String>,
    /// When the running turn started (`Working for 5s`).
    pub started: Option<Instant>,
    /// The last item is an assistant block still receiving deltas.
    streaming: bool,
}

impl Transcript {
    /// The user sent a message: a turn starts.
    pub fn push_user(&mut self, text: &str, images: &[String]) {
        self.items.push(Item::User { text: text.to_owned(), images: images.to_vec() });
        if !self.busy {
            self.started = Some(Instant::now());
        }
        self.busy = true;
        self.streaming = false;
    }

    pub fn apply(&mut self, event: ChatEvent) {
        match event {
            ChatEvent::Session(id) => self.session = Some(id),
            ChatEvent::TextDelta(text) => match self.items.last_mut() {
                Some(Item::Assistant(current)) if self.streaming => current.push_str(&text),
                _ => {
                    self.items.push(Item::Assistant(text));
                    self.streaming = true;
                }
            },
            ChatEvent::Text(text) => {
                match self.items.last_mut() {
                    Some(Item::Assistant(current)) if self.streaming => *current = text,
                    _ if text.trim().is_empty() => {}
                    _ => self.items.push(Item::Assistant(text)),
                }
                self.streaming = false;
            }
            ChatEvent::ToolStart { id, name, input, edits, plan } => {
                self.streaming = false;
                // Codex reports an item when it starts, updates and completes: keep the latest detail.
                let known = self.items.iter_mut().find_map(|item| match item {
                    Item::Tool(t) if t.id == id => Some(t),
                    _ => None,
                });
                match known {
                    Some(tool) => {
                        if !edits.is_empty() {
                            tool.edits = edits;
                        }
                        if plan.is_some() {
                            tool.plan = plan;
                        }
                    }
                    None => self.items.push(Item::Tool(Tool { id, name, input, output: None, error: false, edits, plan })),
                }
            }
            ChatEvent::ToolResult { id, output, error } => {
                let tool = self.items.iter_mut().rev().find_map(|item| match item {
                    Item::Tool(t) if t.id == id => Some(t),
                    _ => None,
                });
                if let Some(tool) = tool {
                    tool.output = Some(output);
                    tool.error = error;
                }
            }
            ChatEvent::Notice(text) => self.items.push(Item::Notice(text)),
            ChatEvent::TurnEnd { error } => {
                self.busy = false;
                self.streaming = false;
                if let Some(error) = error {
                    self.items.push(Item::Error(error));
                }
                if let Some(started) = self.started.take() {
                    self.items.push(Item::Done { secs: started.elapsed().as_secs() });
                }
            }
        }
    }

    /// Last assistant text (the turn's answer, for notifications).
    pub fn last_answer(&self) -> Option<&str> {
        self.items.iter().rev().find_map(|item| match item {
            Item::Assistant(text) => Some(text.as_str()),
            _ => None,
        })
    }

    /// The conversation as saved on disk: the session and the last items, tool output cut short.
    pub fn to_json(&self) -> String {
        let from = self.items.len().saturating_sub(MAX_SAVED_ITEMS);
        // Start on a user message so the first saved turn is whole.
        let from = self.items[from..].iter().position(|it| matches!(it, Item::User { .. })).map_or(from, |n| from + n);
        let items: Vec<Item> = self.items[from..]
            .iter()
            .map(|item| match item {
                Item::Tool(t) => Item::Tool(Tool { output: t.output.as_deref().map(|o| cut(o, MAX_SAVED_OUTPUT)), ..t.clone() }),
                other => other.clone(),
            })
            .collect();
        json!({"session": self.session, "items": items}).to_string()
    }

    /// A transcript saved by [`Self::to_json`]. A turn that was running when it was saved ends
    /// with one "Interrupted by restart" line.
    pub fn from_json(text: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(text).ok()?;
        let mut items: Vec<Item> = serde_json::from_value(v["items"].clone()).ok()?;
        let last_user = items.iter().rposition(|it| matches!(it, Item::User { .. }));
        let unfinished = last_user.is_some_and(|u| !items[u..].iter().any(|it| matches!(it, Item::Done { .. })));
        if unfinished {
            for item in &mut items {
                if let Item::Tool(t) = item
                    && t.output.is_none()
                {
                    t.output = Some(String::new());
                }
            }
            items.push(Item::Notice("Interrupted by restart".into()));
        }
        Some(Self { items, session: str_of(&v["session"]), ..Self::default() })
    }
}

/// The latest checklist of the conversation (TodoWrite / update_plan).
pub fn latest_plan(items: &[Item]) -> Option<&[PlanStep]> {
    items.iter().rev().find_map(|item| match item {
        Item::Tool(Tool { plan: Some(plan), .. }) => Some(plan.as_slice()),
        _ => None,
    })
}

/// Files changed by the successful tool calls of `items`, one entry per path in order of first
/// change: counts summed, patches joined.
pub fn turn_changes(items: &[Item]) -> Vec<FileEdit> {
    let mut out: Vec<FileEdit> = Vec::new();
    let edits = items.iter().filter_map(|it| match it {
        Item::Tool(t) if !t.error => Some(&t.edits),
        _ => None,
    });
    for e in edits.flatten() {
        match out.iter_mut().find(|o| o.path == e.path) {
            Some(o) => {
                o.added += e.added;
                o.removed += e.removed;
                if !e.diff.is_empty() {
                    o.diff = cut(&format!("{}\n@@\n{}", o.diff, e.diff), MAX_DIFF * 2);
                }
            }
            None => out.push(e.clone()),
        }
    }
    out
}

/// A line diff of two texts: (added, removed, patch). Unchanged lines far from a change are left out.
pub fn line_diff(old: &str, new: &str) -> (u32, u32, String) {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let mut lines: Vec<(char, &str)> = a[pre.saturating_sub(CONTEXT)..pre].iter().map(|l| (' ', *l)).collect();
    if am.len() * bm.len() <= MAX_LCS {
        // dp[i * w + j]: longest common subsequence of am[i..] and bm[j..].
        let w = bm.len() + 1;
        let mut dp = vec![0u32; (am.len() + 1) * w];
        for i in (0..am.len()).rev() {
            for j in (0..bm.len()).rev() {
                dp[i * w + j] = if am[i] == bm[j] { dp[(i + 1) * w + j + 1] + 1 } else { dp[(i + 1) * w + j].max(dp[i * w + j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < am.len() || j < bm.len() {
            if i < am.len() && j < bm.len() && am[i] == bm[j] {
                lines.push((' ', am[i]));
                (i, j) = (i + 1, j + 1);
            } else if j == bm.len() || (i < am.len() && dp[(i + 1) * w + j] >= dp[i * w + j + 1]) {
                lines.push(('-', am[i]));
                i += 1;
            } else {
                lines.push(('+', bm[j]));
                j += 1;
            }
        }
    } else {
        lines.extend(am.iter().map(|l| ('-', *l)).chain(bm.iter().map(|l| ('+', *l))));
    }
    lines.extend(a[a.len() - suf..].iter().take(CONTEXT).map(|l| (' ', *l)));
    let count = |k: char| lines.iter().filter(|(c, _)| *c == k).count() as u32;
    let diff: Vec<String> = lines.iter().map(|(c, l)| format!("{c}{l}")).collect();
    (count('+'), count('-'), cut(&diff.join("\n"), MAX_DIFF))
}

/// Files a Claude tool call changes: Edit (old/new string), MultiEdit (edits[]), Write (content).
fn claude_edits(name: &str, input: &Value) -> Vec<FileEdit> {
    let Some(path) = str_of(&input["file_path"]) else { return Vec::new() };
    let pairs: Vec<(&str, &str)> = match name {
        "Edit" => vec![(input["old_string"].as_str().unwrap_or(""), input["new_string"].as_str().unwrap_or(""))],
        "MultiEdit" => blocks(&input["edits"])
            .map(|e| (e["old_string"].as_str().unwrap_or(""), e["new_string"].as_str().unwrap_or("")))
            .collect(),
        "Write" => vec![("", input["content"].as_str().unwrap_or(""))],
        _ => return Vec::new(),
    };
    let mut edit = FileEdit { path, ..FileEdit::default() };
    for (old, new) in pairs {
        let (added, removed, diff) = line_diff(old, new);
        edit.added += added;
        edit.removed += removed;
        edit.diff = if edit.diff.is_empty() { diff } else { cut(&format!("{}\n@@\n{diff}", edit.diff), MAX_DIFF) };
    }
    vec![edit]
}

/// Files of a Codex `file_change` item; counts come from a unified diff when the item has one.
fn codex_edits(item: &Value) -> Vec<FileEdit> {
    blocks(&item["changes"])
        .filter_map(|c| {
            let path = str_of(&c["path"])?;
            let patch = c["diff"].as_str().or_else(|| c["unified_diff"].as_str()).unwrap_or("");
            let lines: Vec<&str> = patch
                .lines()
                .filter(|l| !["+++", "---", "diff ", "index "].iter().any(|p| l.starts_with(p)))
                .collect();
            let count = |p: char| lines.iter().filter(|l| l.starts_with(p)).count() as u32;
            Some(FileEdit { path, added: count('+'), removed: count('-'), diff: cut(&lines.join("\n"), MAX_DIFF) })
        })
        .collect()
}

/// The checklist of a TodoWrite (`todos[{content, status}]`) or update_plan (`plan[{step, status}]`) input.
fn plan_of(input: &Value) -> Option<Vec<PlanStep>> {
    let (list, key) = if input["todos"].is_array() { (&input["todos"], "content") } else { (&input["plan"], "step") };
    let steps = list
        .as_array()?
        .iter()
        .filter_map(|s| Some(PlanStep { text: str_of(&s[key])?, status: step_status(s["status"].as_str().unwrap_or("")) }))
        .collect();
    Some(steps)
}

fn step_status(status: &str) -> StepStatus {
    match status {
        "completed" | "done" => StepStatus::Completed,
        "in_progress" | "in-progress" | "active" => StepStatus::InProgress,
        _ => StepStatus::Pending,
    }
}

/// Claude stream-json input line carrying a user message.
pub fn claude_user_line(text: &str) -> String {
    json!({"type": "user", "message": {"role": "user", "content": text}}).to_string()
}

/// Claude stream-json control request that interrupts the running turn (the process stays).
pub fn claude_interrupt_line(request_id: &str) -> String {
    json!({"type": "control_request", "request_id": request_id, "request": {"subtype": "interrupt"}}).to_string()
}

/// Events of one `claude -p --output-format stream-json --verbose --include-partial-messages` line.
pub fn parse_claude(line: &str) -> Vec<ChatEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Vec::new() };
    // Subagent traffic (Task tool) is not the main conversation.
    if v["parent_tool_use_id"].is_string() {
        return Vec::new();
    }
    match v["type"].as_str().unwrap_or("") {
        "system" if v["subtype"] == "init" => str_of(&v["session_id"]).map(ChatEvent::Session).into_iter().collect(),
        "stream_event" => {
            let e = &v["event"];
            match (e["type"].as_str(), e["delta"]["type"].as_str()) {
                (Some("content_block_delta"), Some("text_delta")) => {
                    str_of(&e["delta"]["text"]).map(ChatEvent::TextDelta).into_iter().collect()
                }
                _ => Vec::new(),
            }
        }
        "assistant" => blocks(&v["message"]["content"])
            .filter_map(|b| match b["type"].as_str()? {
                "text" => Some(ChatEvent::Text(str_of(&b["text"])?)),
                "tool_use" => {
                    let name = str_of(&b["name"]).unwrap_or_default();
                    let input = &b["input"];
                    let short = name.rsplit("__").next().unwrap_or(&name);
                    Some(ChatEvent::ToolStart {
                        id: str_of(&b["id"])?,
                        edits: claude_edits(&name, input),
                        plan: matches!(short, "TodoWrite" | "update_plan").then(|| plan_of(input)).flatten(),
                        input: summarize_input(input),
                        name,
                    })
                }
                _ => None,
            })
            .collect(),
        "user" => blocks(&v["message"]["content"])
            .filter(|b| b["type"] == "tool_result")
            .filter_map(|b| {
                Some(ChatEvent::ToolResult {
                    id: str_of(&b["tool_use_id"])?,
                    output: cap(&content_text(&b["content"])),
                    error: b["is_error"].as_bool().unwrap_or(false),
                })
            })
            .collect(),
        "result" => {
            let failed = v["is_error"].as_bool().unwrap_or(false) || v["subtype"].as_str().is_some_and(|s| s != "success");
            let error = failed.then(|| {
                str_of(&v["result"])
                    .filter(|r| !r.is_empty())
                    .or_else(|| str_of(&v["subtype"]))
                    .unwrap_or_else(|| "Turn failed".into())
            });
            vec![ChatEvent::TurnEnd { error }]
        }
        _ => Vec::new(),
    }
}

/// Events of one `codex exec --json` line.
pub fn parse_codex(line: &str) -> Vec<ChatEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Vec::new() };
    match v["type"].as_str().unwrap_or("") {
        "thread.started" => str_of(&v["thread_id"]).map(ChatEvent::Session).into_iter().collect(),
        "turn.completed" => vec![ChatEvent::TurnEnd { error: None }],
        "turn.failed" => vec![ChatEvent::TurnEnd {
            error: Some(str_of(&v["error"]["message"]).unwrap_or_else(|| "Turn failed".into())),
        }],
        "error" => str_of(&v["message"]).map(ChatEvent::Notice).into_iter().collect(),
        kind @ ("item.started" | "item.updated" | "item.completed") => codex_item(&v["item"], kind == "item.completed"),
        _ => Vec::new(),
    }
}

fn codex_item(item: &Value, completed: bool) -> Vec<ChatEvent> {
    let Some(id) = str_of(&item["id"]) else { return Vec::new() };
    let (mut edits, mut plan) = (Vec::new(), None);
    let (name, input) = match item["type"].as_str().unwrap_or("") {
        "agent_message" if completed => return str_of(&item["text"]).map(ChatEvent::Text).into_iter().collect(),
        "error" if completed => return str_of(&item["message"]).map(ChatEvent::Notice).into_iter().collect(),
        "command_execution" => ("shell".to_owned(), one_line(&str_of(&item["command"]).unwrap_or_default())),
        "file_change" => {
            let paths: Vec<&str> = item["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["path"].as_str())
                .collect();
            edits = codex_edits(item);
            ("edit".to_owned(), one_line(&paths.join(", ")))
        }
        "todo_list" => {
            let steps = blocks(&item["items"])
                .filter_map(|s| {
                    let done = s["completed"].as_bool().unwrap_or(false);
                    Some(PlanStep { text: str_of(&s["text"])?, status: if done { StepStatus::Completed } else { StepStatus::Pending } })
                })
                .collect();
            plan = Some(steps);
            ("update_plan".to_owned(), String::new())
        }
        "mcp_tool_call" => {
            let tool = item["tool"].as_str().unwrap_or("?");
            if tool == "update_plan" {
                plan = plan_of(&item["arguments"]);
            }
            (format!("{}.{tool}", item["server"].as_str().unwrap_or("mcp")), summarize_input(&item["arguments"]))
        }
        "web_search" => ("web_search".to_owned(), one_line(&str_of(&item["query"]).unwrap_or_default())),
        _ => return Vec::new(),
    };
    let mut events = vec![ChatEvent::ToolStart { id: id.clone(), name, input, edits, plan }];
    // A plan has no result: it is done once reported.
    if completed || item["type"] == "todo_list" {
        let status = item["status"].as_str().unwrap_or("");
        let exit_code = item["exit_code"].as_i64().unwrap_or(0);
        let output = str_of(&item["aggregated_output"])
            .or_else(|| item["result"].as_object().map(|_| content_text(&item["result"]["content"])))
            .or_else(|| str_of(&item["error"]["message"]))
            .unwrap_or_default();
        events.push(ChatEvent::ToolResult { id, output: cap(&output), error: status == "failed" || exit_code != 0 });
    }
    events
}

fn str_of(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}

fn blocks(content: &Value) -> impl Iterator<Item = &Value> {
    content.as_array().into_iter().flatten()
}

/// Text of a tool result: a string, or an array of `{type: "text", text}` blocks.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The most telling field of a tool input on one line: command, path, pattern, url... else the JSON.
fn summarize_input(input: &Value) -> String {
    const KEYS: [&str; 9] = ["command", "file_path", "path", "pattern", "url", "query", "description", "prompt", "title"];
    let text = KEYS
        .iter()
        .find_map(|k| input[k].as_str().map(str::to_owned))
        .unwrap_or_else(|| match input {
            Value::Object(map) if map.is_empty() => String::new(),
            Value::Null => String::new(),
            other => other.to_string(),
        });
    one_line(&text)
}

fn one_line(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(MAX_SUMMARY) {
        Some((i, _)) => format!("{}…", &flat[..i]),
        None => flat,
    }
}

fn cap(text: &str) -> String {
    cut(text, MAX_OUTPUT)
}

/// `text` cut to at most `max` bytes (on a char boundary), marked when cut.
fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (cut)", &text[..end])
}

/// A `/command` or `@file` being typed before the cursor: its sigil, where it starts (byte) and
/// what follows the sigil. Only at the start of the text or after whitespace.
#[derive(Debug, Clone, PartialEq)]
pub struct Trigger {
    pub sigil: char,
    pub start: usize,
    pub query: String,
}

pub fn trigger_at(text: &str, cursor: usize) -> Option<Trigger> {
    let before = text.get(..cursor)?;
    let start = before.char_indices().rev().find(|(_, c)| c.is_whitespace()).map_or(0, |(i, c)| i + c.len_utf8());
    let token = &before[start..];
    let sigil = token.chars().next().filter(|c| matches!(c, '/' | '@'))?;
    Some(Trigger { sigil, start, query: token[1..].to_owned() })
}

/// Files under `root` as `/`-separated relative paths (hidden, dependency and build folders
/// skipped), at most `max`.
pub fn project_files(root: &Path, max: usize) -> Vec<String> {
    const SKIP: [&str; 7] = ["node_modules", "target", "dist", "build", "__pycache__", "vendor", "out"];
    fn walk(root: &Path, dir: &Path, depth: usize, max: usize, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if out.len() >= max {
                return;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => {
                    if depth < 12 && !name.starts_with('.') && !SKIP.contains(&name.as_str()) {
                        walk(root, &path, depth + 1, max, out);
                    }
                }
                Ok(t) if t.is_file() => {
                    out.push(path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/"));
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, 0, max, &mut out);
    out
}

/// Arguments without the given flags: `valued` ones also lose their value (`--x v` or `--x=v`).
pub fn strip_flags(args: &[String], valued: &[&str], bare: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if valued.contains(&arg.as_str()) {
            rest.next();
        } else if !bare.contains(&arg.as_str()) && !valued.iter().any(|f| arg.starts_with(&format!("{f}="))) {
            out.push(arg.clone());
        }
    }
    out
}

/// `args` with `model` as the only model flag (none: the agent's default).
pub fn with_model(args: &[String], model: Option<&str>) -> Vec<String> {
    let mut out = strip_flags(args, &["--model", "-m"], &[]);
    if let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) {
        out.extend(["--model".to_owned(), model.to_owned()]);
    }
    out
}

/// Claude `args` resuming `session` (forked into a new conversation when `fork`), or starting fresh.
pub fn with_resume(args: &[String], session: Option<&str>, fork: bool) -> Vec<String> {
    let mut out = strip_flags(args, &["--resume", "-r"], &["--continue", "-c", "--fork-session"]);
    if let Some(session) = session {
        out.extend(["--resume".to_owned(), session.to_owned()]);
        if fork {
            out.push("--fork-session".to_owned());
        }
    }
    out
}

/// What a chat agent may do without asking: Claude's permission mode, Codex's sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Ask,
    AutoEdit,
    Full,
    /// Claude only: read and plan, no edits.
    Plan,
}

const CLAUDE_MODE_FLAGS: (&[&str], &[&str]) = (&["--permission-mode"], &["--dangerously-skip-permissions"]);
const CODEX_MODE_FLAGS: (&[&str], &[&str]) =
    (&["-s", "--sandbox"], &["--full-auto", "--yolo", "--dangerously-bypass-approvals-and-sandbox"]);

impl Mode {
    /// The modes the agent offers, in menu order.
    pub fn all(agent: AgentKind) -> &'static [Mode] {
        match agent {
            AgentKind::Claude => &[Mode::Ask, Mode::AutoEdit, Mode::Full, Mode::Plan],
            _ => &[Mode::Ask, Mode::AutoEdit, Mode::Full],
        }
    }

    pub fn label(self, agent: AgentKind) -> &'static str {
        match (self, agent) {
            (Mode::Ask, AgentKind::Codex) => "Read only",
            (Mode::Ask, _) => "Ask first",
            (Mode::AutoEdit, _) => "Auto-edit",
            (Mode::Full, _) => "Full access",
            (Mode::Plan, _) => "Plan mode",
        }
    }

    /// The mode `args` give; defaults match what chat mode adds (Claude acceptEdits, Codex workspace-write).
    pub fn of(agent: AgentKind, args: &[String]) -> Mode {
        let has = |flag: &str| args.iter().any(|a| a == flag);
        let value = |flags: &[&str]| {
            args.iter().enumerate().find_map(|(i, a)| {
                flags.iter().find_map(|f| if a == f { args.get(i + 1).cloned() } else { a.strip_prefix(&format!("{f}=")).map(str::to_owned) })
            })
        };
        match agent {
            AgentKind::Claude if has("--dangerously-skip-permissions") => Mode::Full,
            AgentKind::Claude => match value(&["--permission-mode"]).as_deref() {
                Some("bypassPermissions") => Mode::Full,
                Some("plan") => Mode::Plan,
                Some("default") => Mode::Ask,
                _ => Mode::AutoEdit,
            },
            _ if has("--yolo") || has("--dangerously-bypass-approvals-and-sandbox") => Mode::Full,
            _ => match value(&["-s", "--sandbox"]).as_deref() {
                Some("danger-full-access") => Mode::Full,
                Some("read-only") => Mode::Ask,
                _ => Mode::AutoEdit,
            },
        }
    }

    /// `args` with this mode as the only permission flag.
    pub fn apply(self, agent: AgentKind, args: &[String]) -> Vec<String> {
        let ((valued, bare), flag, value) = match (agent, self) {
            (AgentKind::Claude, Mode::Ask) => (CLAUDE_MODE_FLAGS, "--permission-mode", "default"),
            (AgentKind::Claude, Mode::AutoEdit) => (CLAUDE_MODE_FLAGS, "--permission-mode", "acceptEdits"),
            (AgentKind::Claude, Mode::Full) => (CLAUDE_MODE_FLAGS, "--permission-mode", "bypassPermissions"),
            (AgentKind::Claude, Mode::Plan) => (CLAUDE_MODE_FLAGS, "--permission-mode", "plan"),
            (_, Mode::Ask) => (CODEX_MODE_FLAGS, "-s", "read-only"),
            (_, Mode::Full) => (CODEX_MODE_FLAGS, "-s", "danger-full-access"),
            (_, Mode::AutoEdit | Mode::Plan) => (CODEX_MODE_FLAGS, "-s", "workspace-write"),
        };
        let mut out = strip_flags(args, valued, bare);
        out.extend([flag.to_owned(), value.to_owned()]);
        out
    }
}

/// Codex's configured model (`model = "..."` at the top of `~/.codex/config.toml`).
pub fn codex_default_model() -> Option<String> {
    let dir = std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))?;
    toml_model(&std::fs::read_to_string(dir.join("config.toml")).ok()?)
}

fn toml_model(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .take_while(|l| !l.starts_with('['))
        .find_map(|l| {
            let (key, value) = l.split_once('=')?;
            (key.trim() == "model").then(|| value.trim().trim_matches(|c| c == '"' || c == '\'').to_owned())
        })
        .filter(|m| !m.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lines: &[&str], parse: fn(&str) -> Vec<ChatEvent>) -> Transcript {
        let mut t = Transcript::default();
        t.push_user("hi", &[]);
        for line in lines {
            for event in parse(line) {
                t.apply(event);
            }
        }
        t
    }

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    // Trimmed from a real `claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages` run.
    const CLAUDE: &[&str] = &[
        r#"{"type":"system","subtype":"hook_started","hook_name":"SessionStart:startup","session_id":"s1"}"#,
        r#"{"type":"system","subtype":"init","cwd":"C:\\x","session_id":"0de9e883","tools":["Bash"]}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"Bash","input":{}}},"session_id":"0de9e883","parent_tool_use_id":null}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"echo hi","description":"Print hi"}}]},"parent_tool_use_id":null}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_1","type":"tool_result","content":"hi","is_error":false}]},"parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"do"}},"parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"n"}},"parent_tool_use_id":null}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"done"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"0de9e883"}"#,
    ];

    #[test]
    fn chat_panes_are_live_until_removed() {
        assert!(!is_live("chat-test"));
        set_live("chat-test", true);
        assert!(is_live("chat-test"));
        set_live("chat-test", false);
        assert!(!is_live("chat-test"));
    }

    #[test]
    fn claude_turn() {
        let t = run(CLAUDE, parse_claude);
        assert_eq!(t.session.as_deref(), Some("0de9e883"));
        assert!(!t.busy);
        assert_eq!(
            t.items,
            vec![
                Item::User { text: "hi".into(), images: Vec::new() },
                Item::Tool(Tool {
                    id: "toolu_1".into(),
                    name: "Bash".into(),
                    input: "echo hi".into(),
                    output: Some("hi".into()),
                    ..Tool::default()
                }),
                // Deltas streamed "don", the final block replaced them.
                Item::Assistant("done".into()),
                Item::Done { secs: 0 },
            ]
        );
    }

    #[test]
    fn claude_streaming_then_error() {
        let mut t = Transcript::default();
        t.push_user("x", &[]);
        for e in parse_claude(CLAUDE[5]).into_iter().chain(parse_claude(CLAUDE[6])) {
            t.apply(e);
        }
        assert_eq!(t.items.last(), Some(&Item::Assistant("don".into())));
        assert!(t.busy);
        let failed = r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#;
        for e in parse_claude(failed) {
            t.apply(e);
        }
        assert_eq!(t.items[t.items.len() - 2], Item::Error("error_during_execution".into()));
        assert!(matches!(t.items.last(), Some(Item::Done { .. })));
        assert!(!t.busy);
    }

    #[test]
    fn claude_ignores_subagents_and_junk() {
        let sub = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"inner"}]},"parent_tool_use_id":"toolu_9"}"#;
        assert!(parse_claude(sub).is_empty());
        assert!(parse_claude("not json").is_empty());
        assert!(parse_claude("").is_empty());
    }

    #[test]
    fn claude_input_lines() {
        let v: Value = serde_json::from_str(&claude_user_line("a \"quoted\"\nline")).unwrap();
        assert_eq!(v["message"]["content"], "a \"quoted\"\nline");
        let v: Value = serde_json::from_str(&claude_interrupt_line("r1")).unwrap();
        assert_eq!(v["request"]["subtype"], "interrupt");
    }

    #[test]
    fn claude_edits_and_plans() {
        let edit = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"e","name":"Edit","input":{"file_path":"src/a.rs","old_string":"a\nb\nc","new_string":"a\nB\nc\nd"}}]}}"#;
        let ChatEvent::ToolStart { edits, plan, input, .. } = &parse_claude(edit)[0] else { panic!() };
        assert_eq!(input, "src/a.rs");
        assert!(plan.is_none());
        assert_eq!((edits[0].added, edits[0].removed), (2, 1));
        assert_eq!(edits[0].diff, " a\n-b\n+B\n c\n+d");

        let write = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"w","name":"Write","input":{"file_path":"n.txt","content":"x\ny\n"}}]}}"#;
        let ChatEvent::ToolStart { edits, .. } = &parse_claude(write)[0] else { panic!() };
        assert_eq!((edits[0].added, edits[0].removed), (2, 0));

        let multi = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"m","name":"MultiEdit","input":{"file_path":"m.rs","edits":[{"old_string":"a","new_string":"b"},{"old_string":"c","new_string":""}]}}]}}"#;
        let ChatEvent::ToolStart { edits, .. } = &parse_claude(multi)[0] else { panic!() };
        assert_eq!((edits[0].added, edits[0].removed), (1, 2));

        let todo = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t","name":"TodoWrite","input":{"todos":[{"content":"Read","status":"completed"},{"content":"Fix","status":"in_progress"},{"content":"Test","status":"pending"}]}}]}}"#;
        let mut t = Transcript::default();
        t.push_user("x", &[]);
        for e in parse_claude(todo) {
            t.apply(e);
        }
        let plan = latest_plan(&t.items).unwrap();
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[1], PlanStep { text: "Fix".into(), status: StepStatus::InProgress });
    }

    #[test]
    fn turn_changes_merge_by_file_and_skip_failures() {
        let tool = |path: &str, added, removed, error| {
            Item::Tool(Tool { edits: vec![FileEdit { path: path.into(), added, removed, diff: "+x".into() }], error, ..Tool::default() })
        };
        let items = vec![tool("a", 2, 1, false), tool("b", 5, 0, false), tool("a", 1, 1, false), tool("c", 9, 9, true)];
        let changes = turn_changes(&items);
        assert_eq!(changes.len(), 2);
        assert_eq!((changes[0].path.as_str(), changes[0].added, changes[0].removed), ("a", 3, 2));
        assert_eq!(changes[0].diff, "+x\n@@\n+x");
    }

    #[test]
    fn line_diff_shapes() {
        assert_eq!(line_diff("", ""), (0, 0, String::new()));
        assert_eq!(line_diff("same", "same").0, 0);
        let (added, removed, diff) = line_diff("1\n2\n3\n4\n5\n6", "1\n2\n3\nX\n5\n6");
        assert_eq!((added, removed), (1, 1));
        assert_eq!(diff, " 2\n 3\n-4\n+X\n 5\n 6");
    }

    // From a real `codex exec --json` run (codex-cli 0.167).
    const CODEX: &[&str] = &[
        r#"{"type":"thread.started","thread_id":"01a1d0d8-79a5"}"#,
        r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Codex is ignoring 1 unrecognized configuration setting."}}"#,
        r#"{"type":"turn.started"}"#,
        r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"I'll run the command now.\n"}}"#,
        r#"{"type":"item.started","item":{"id":"item_3","type":"command_execution","command":"powershell.exe -Command 'echo hi'","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#,
        r#"{"type":"item.completed","item":{"id":"item_3","type":"command_execution","command":"powershell.exe -Command 'echo hi'","aggregated_output":"hi\r\n","exit_code":0,"status":"completed"}}"#,
        r#"{"type":"item.completed","item":{"id":"item_4","type":"agent_message","text":"done"}}"#,
        r#"{"type":"turn.completed","usage":{"input_tokens":40084,"output_tokens":46}}"#,
    ];

    #[test]
    fn codex_turn() {
        let t = run(CODEX, parse_codex);
        assert_eq!(t.session.as_deref(), Some("01a1d0d8-79a5"));
        assert!(!t.busy);
        assert_eq!(t.items.len(), 6, "{:?}", t.items);
        assert!(matches!(&t.items[1], Item::Notice(n) if n.starts_with("Codex is ignoring")));
        assert_eq!(t.items[2], Item::Assistant("I'll run the command now.\n".into()));
        assert_eq!(
            t.items[3],
            Item::Tool(Tool {
                id: "item_3".into(),
                name: "shell".into(),
                input: "powershell.exe -Command 'echo hi'".into(),
                output: Some("hi\r\n".into()),
                ..Tool::default()
            })
        );
        assert_eq!(t.last_answer(), Some("done"));
    }

    #[test]
    fn codex_failures() {
        let failed_cmd = r#"{"type":"item.completed","item":{"id":"i","type":"command_execution","command":"false","aggregated_output":"","exit_code":1,"status":"failed"}}"#;
        assert!(matches!(parse_codex(failed_cmd).last(), Some(ChatEvent::ToolResult { error: true, .. })));
        let edit = r#"{"type":"item.completed","item":{"id":"f","type":"file_change","changes":[{"path":"src/a.rs","kind":"update"}],"status":"completed"}}"#;
        assert!(matches!(&parse_codex(edit)[0], ChatEvent::ToolStart { name, input, edits, .. } if name == "edit" && input == "src/a.rs" && edits[0].path == "src/a.rs"));
        let turn = r#"{"type":"turn.failed","error":{"message":"stream disconnected"}}"#;
        assert_eq!(parse_codex(turn), vec![ChatEvent::TurnEnd { error: Some("stream disconnected".into()) }]);
    }

    #[test]
    fn codex_plans_update_in_place() {
        let started = r#"{"type":"item.started","item":{"id":"p","type":"todo_list","items":[{"text":"a","completed":false},{"text":"b","completed":false}]}}"#;
        let updated = r#"{"type":"item.updated","item":{"id":"p","type":"todo_list","items":[{"text":"a","completed":true},{"text":"b","completed":false}]}}"#;
        let t = run(&[started, updated], parse_codex);
        assert_eq!(t.items.iter().filter(|i| matches!(i, Item::Tool(_))).count(), 1);
        assert_eq!(latest_plan(&t.items).unwrap()[0].status, StepStatus::Completed);
        let diff = r#"{"type":"item.completed","item":{"id":"d","type":"file_change","changes":[{"path":"x","diff":"--- a/x\n+++ b/x\n@@ -1 +1,2 @@\n-a\n+b\n+c"}]}}"#;
        let ChatEvent::ToolStart { edits, .. } = &parse_codex(diff)[0] else { panic!() };
        assert_eq!((edits[0].added, edits[0].removed), (2, 1));
    }

    #[test]
    fn long_inputs_are_cut() {
        let long = "x".repeat(500);
        assert!(summarize_input(&json!({"command": long})).chars().count() <= MAX_SUMMARY + 1);
        assert!(cap(&"é".repeat(MAX_OUTPUT)).ends_with("(cut)"));
    }

    #[test]
    fn saved_transcripts_round_trip_and_mark_interrupted_turns() {
        let t = run(CLAUDE, parse_claude);
        let back = Transcript::from_json(&t.to_json()).unwrap();
        assert_eq!(back.items, t.items);
        assert_eq!(back.session.as_deref(), Some("0de9e883"));
        assert!(!back.busy);

        let mut running = Transcript::default();
        running.push_user("go", &[]);
        running.apply(parse_claude(CLAUDE[3]).remove(0));
        let back = Transcript::from_json(&running.to_json()).unwrap();
        assert_eq!(back.items.last(), Some(&Item::Notice("Interrupted by restart".into())));
        assert!(matches!(&back.items[1], Item::Tool(t) if t.output.is_some()));
        assert!(Transcript::from_json("junk").is_none());
    }

    #[test]
    fn triggers_before_the_cursor() {
        assert_eq!(trigger_at("/cl", 3), Some(Trigger { sigil: '/', start: 0, query: "cl".into() }));
        assert_eq!(trigger_at("look at @src/ma rest", 15), Some(Trigger { sigil: '@', start: 8, query: "src/ma".into() }));
        assert_eq!(trigger_at("a/b", 3), None);
        assert_eq!(trigger_at("hi there", 8), None);
        assert_eq!(trigger_at("é @", 4), Some(Trigger { sigil: '@', start: 3, query: String::new() }));
        assert_eq!(trigger_at("x", 9), None);
    }

    #[test]
    fn model_mode_and_resume_flags() {
        let args = strings(&["-p", "--model", "opus", "--permission-mode=plan", "--resume", "s1", "--fork-session"]);
        assert_eq!(with_model(&args, Some("haiku"))[..2], strings(&["-p", "--permission-mode=plan"])[..]);
        assert!(with_model(&args, Some("haiku")).ends_with(&strings(&["--model", "haiku"])));
        assert!(!with_model(&args, None).iter().any(|a| a == "--model"));
        assert_eq!(Mode::of(AgentKind::Claude, &args), Mode::Plan);
        let full = Mode::Full.apply(AgentKind::Claude, &args);
        assert_eq!(Mode::of(AgentKind::Claude, &full), Mode::Full);
        assert_eq!(full.iter().filter(|a| a.starts_with("--permission-mode")).count(), 1);
        let resumed = with_resume(&args, Some("s2"), false);
        assert!(resumed.ends_with(&strings(&["--resume", "s2"])));
        assert!(!resumed.iter().any(|a| a == "--fork-session" || a == "s1"));
        assert!(with_resume(&args, Some("s2"), true).ends_with(&strings(&["--fork-session"])));

        let codex = strings(&["--yolo"]);
        assert_eq!(Mode::of(AgentKind::Codex, &codex), Mode::Full);
        let ask = Mode::Ask.apply(AgentKind::Codex, &codex);
        assert_eq!(ask, strings(&["-s", "read-only"]));
        assert_eq!(Mode::of(AgentKind::Codex, &[]), Mode::AutoEdit);
    }

    #[test]
    fn reads_the_codex_model_from_config() {
        assert_eq!(toml_model("# c\nmodel = \"gpt-5\"\n[profiles.x]\nmodel = \"o3\""), Some("gpt-5".into()));
        assert_eq!(toml_model("[profiles.x]\nmodel = \"o3\""), None);
        assert_eq!(toml_model("model_provider = \"x\""), None);
    }
}
