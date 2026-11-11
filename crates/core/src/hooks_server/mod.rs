//! Local HTTP server (127.0.0.1 only, secret token) receiving agent hook events
//! and re-emitting them to the UI as `agent-status`.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

pub mod permissions;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use crate::AppHandle;
use tiny_http::{Request, Response, Server};

use crate::AppState;
use permissions::{PENDING, PERMISSION_HOLD, wait_decision};

/// Header carrying the per-run secret; also how our Claude hook entries are recognized.
pub const TOKEN_HEADER: &str = "X-AgentDeck-Token";
pub(crate) const PANE_HEADER: &str = "X-AgentDeck-Pane";
/// Preferred port keeps `.claude/settings.local.json` stable across app runs.
const PREFERRED_PORT: u16 = 47800;
pub(crate) const MAX_BODY: u64 = 1024 * 1024;
/// Cap on prompt/summary sent to the UI (bytes).
const MAX_TEXT: usize = 8 * 1024;
/// Only the end of a transcript is read when looking for the last assistant message.
const TRANSCRIPT_TAIL: u64 = 256 * 1024;
/// How long a browser tool call waits for its panel to open; under the Claude hooks' 5 s timeout.
const BROWSER_HOLD: Duration = Duration::from_secs(4);

pub struct HookServer {
    pub port: u16,
    pub token: String,
}

impl HookServer {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/hook", self.port)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneStatus {
    Idle,
    Working,
    Waiting,
    Done,
    Error,
}

/// Payload of the `agent-status` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatusEvent {
    pub pane_id: String,
    pub status: PaneStatus,
    pub last_action: Option<String>,
    /// Raw hook name, e.g. "PreToolUse".
    pub event: String,
    /// User request that started the turn (Claude UserPromptSubmit, Codex input-messages).
    pub prompt: Option<String>,
    /// Agent's final message of the turn (Claude Stop, Codex agent-turn-complete).
    pub summary: Option<String>,
    /// Claude conversation id, once the conversation has a message (so `--resume` can find it).
    pub session_id: Option<String>,
    /// Absolute path of a file the agent just edited (Claude PostToolUse on an edit tool).
    pub edited_file: Option<String>,
}

/// Starts the hook server on a background thread.
pub fn start(app: AppHandle) -> HookServer {
    let token = uuid::Uuid::new_v4().to_string();
    // Note: a second AgentDeck instance falls back to a random port and rewrites the Claude settings.
    let server =
        Server::http(("127.0.0.1", PREFERRED_PORT)).or_else(|_| Server::http("127.0.0.1:0"));
    let server = match server {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hooks server disabled: {e}");
            return HookServer { port: 0, token };
        }
    };
    let port = server.server_addr().to_ip().map_or(0, |a| a.port());
    let expected = token.clone();
    std::thread::spawn(move || {
        for mut req in server.incoming_requests() {
            // One thread per request: a slow body (or a held permission request) must not stall other panes.
            let (app, expected) = (app.clone(), expected.clone());
            std::thread::spawn(move || {
                let path = req.url().split('?').next().unwrap_or("").to_owned();
                if path == "/mcp" {
                    return crate::mcp::serve(&app, req, &expected);
                }
                if path == "/api" {
                    return crate::remote::serve(&app, req);
                }
                if let Some(name) = path.strip_prefix("/hub/") {
                    return crate::mcp::hub::serve(&app, req, &expected, name);
                }
                let reply = match handle(&mut req, &expected) {
                    Ok(hook) => process(&app, &hook),
                    Err(code) => Response::empty(code).boxed(),
                };
                let _ = req.respond(reply);
            });
        }
    });
    HookServer { port, token }
}

/// Emits the status event; a Claude PermissionRequest is held until answered (see `Permissions`).
fn process(app: &AppHandle, hook: &Hook) -> tiny_http::ResponseBox {
    let no_decision = || Response::empty(204).boxed();
    if !pane_exists(app, &hook.pane_id) {
        return no_decision();
    }
    PENDING.cancel_stale(&hook.pane_id, &hook.json);
    let (running, changed) = track_subagents(&hook.pane_id, &hook.json, crate::logic::text::now_ms());
    if let Some(steps) = track_steps(&hook.pane_id, &hook.json, changed.as_deref()) {
        let _ = app.emit("agent-steps", json!({ "paneId": hook.pane_id, "steps": steps }));
    }
    if let Some(subagents) = changed {
        let _ = app.emit(
            "agent-subagents",
            json!({ "paneId": hook.pane_id, "subagents": subagents }),
        );
    }
    if let Some(event) = status_event_with(hook, running) {
        emit_status(app, event);
    }
    if calls_browser(&hook.json) {
        open_browser(app, &hook.pane_id);
    }
    if hook.json["hook_event_name"] != "PermissionRequest" {
        return no_decision();
    }
    let (request, rx) = PENDING.register(&hook.pane_id, &hook.json);
    let id = request.request_id.clone();
    let _ = app.emit("agent-permission", request);
    let decision = wait_decision(&rx, PERMISSION_HOLD, || pane_exists(app, &hook.pane_id));
    PENDING.remove(&id);
    let _ = app.emit("agent-permission-resolved", json!({ "requestId": id }));
    // Allowed or denied, Claude goes on with its turn and sends no hook before the tool ends:
    // without this the pane stays "waiting" for the whole command. No decision: the terminal asks.
    if decision.is_some() && !PENDING.has_pane(&hook.pane_id) {
        emit_status(app, resolved_event(hook));
    }
    match decision {
        Some(body) => {
            let ct = tiny_http::Header::from_bytes("Content-Type", "application/json")
                .expect("static header");
            Response::from_string(body.to_string())
                .with_header(ct)
                .boxed()
        }
        None => no_decision(),
    }
}

/// A Claude pane is about to call a tool of its browser panel (MCP server "browser").
fn calls_browser(body: &Value) -> bool {
    body["hook_event_name"] == "PreToolUse"
        && body["tool_name"].as_str().is_some_and(|tool| tool.starts_with("mcp__browser__"))
}

/// The port a pane's browser MCP config (`<data>/browser/<pane>.json`) points to: the one its
/// running agent connects to.
pub fn browser_config_port(text: &str) -> Option<u16> {
    let config: Value = serde_json::from_str(text).ok()?;
    let url = config["mcpServers"]["browser"]["args"].as_array()?.last()?.as_str()?;
    url.rsplit(':').next()?.parse().ok()
}

/// Asks the UI to open the pane's browser panel (`agent-browser`) when its webview is not
/// listening, and holds the hook until it is: Claude runs the tool after the reply, so
/// chrome-devtools-mcp finds the browser. Gives up after `BROWSER_HOLD`: the tool then fails to connect.
fn open_browser(app: &AppHandle, pane_id: &str) {
    let config = app.app_data_dir().map(|dir| dir.join("browser").join(format!("{pane_id}.json")));
    let Some(port) = config.ok().and_then(|path| std::fs::read_to_string(path).ok()).and_then(|t| browser_config_port(&t))
    else {
        return;
    };
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listening = || std::net::TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok();
    if listening() {
        return;
    }
    let _ = app.emit("agent-browser", json!({ "paneId": pane_id }));
    let deadline = Instant::now() + BROWSER_HOLD;
    while Instant::now() < deadline && !listening() {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The UI gets the event before `wait_for` wakes: its get_result then sees this turn's summary.
fn emit_status(app: &AppHandle, event: AgentStatusEvent) {
    let (pane_id, status) = (event.pane_id.clone(), event.status);
    let _ = app.emit("agent-status", event);
    crate::mcp::STATUS.set(&pane_id, status);
}

/// Status of a pane whose permission request was just answered: back to work on that tool.
fn resolved_event(hook: &Hook) -> AgentStatusEvent {
    AgentStatusEvent {
        pane_id: hook.pane_id.clone(),
        status: PaneStatus::Working,
        last_action: describe_tool(&hook.json),
        event: "PermissionResolved".into(),
        prompt: None,
        summary: None,
        session_id: session_id(&hook.json),
        edited_file: None,
    }
}

/// Events for unknown panes (stale or forged ids) are dropped.
fn pane_exists(app: &AppHandle, pane_id: &str) -> bool {
    app.try_state::<AppState>()
        .is_some_and(|s| s.panes.0.lock().is_ok_and(|p| p.contains_key(pane_id)))
}

pub(crate) fn header<'a>(req: &'a Request, name: &'static str) -> Option<&'a str> {
    req.headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}

/// An authenticated hook payload and the pane it belongs to.
#[derive(Debug)]
struct Hook {
    pane_id: String,
    json: Value,
}

/// A Claude subagent running inside a pane (`agent-subagents` event: `{ paneId, subagents }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subagent {
    pub agent_id: String,
    /// "Explore", "general-purpose"...
    pub agent_type: String,
    /// Description the main agent gave the Agent tool, else the agent type.
    pub label: String,
    pub started_at: i64,
    /// Its latest tool event (`describe_tool`).
    pub last_action: Option<String>,
}

#[derive(Default)]
struct PaneSubagents {
    /// SubagentStart minus SubagentStop, oldest first.
    running: Vec<Subagent>,
    /// (subagent_type, description) of Agent tool calls no SubagentStart has claimed yet.
    unclaimed: Vec<(String, String)>,
}

/// Agent tool calls kept for pairing: a denied call never starts a subagent.
const MAX_UNCLAIMED: usize = 16;
const DEFAULT_AGENT_TYPE: &str = "general-purpose";

static SUBAGENTS: LazyLock<Mutex<HashMap<String, PaneSubagents>>> = LazyLock::new(Default::default);

/// Records what a hook says about the pane's subagents: how many still run, and the new list when it changed.
fn track_subagents(pane_id: &str, body: &Value, now: i64) -> (usize, Option<Vec<Subagent>>) {
    let mut map = SUBAGENTS.lock().unwrap_or_else(PoisonError::into_inner);
    let pane = map.entry(pane_id.to_owned()).or_default();
    let before = pane.running.clone();
    let event = body["hook_event_name"].as_str().unwrap_or("");
    let agent_id = body["agent_id"].as_str().filter(|id| !id.is_empty());
    match (event, agent_id) {
        ("SubagentStart", Some(id)) if !pane.running.iter().any(|s| s.agent_id == id) => {
            let agent_type = body["agent_type"].as_str().unwrap_or(DEFAULT_AGENT_TYPE);
            // Best effort: hooks carry no link between the Agent tool call and the subagent it starts.
            let label = pane
                .unclaimed
                .iter()
                .rposition(|(kind, _)| kind == agent_type)
                .map_or_else(|| agent_type.to_owned(), |i| pane.unclaimed.remove(i).1);
            pane.running.push(Subagent {
                agent_id: id.to_owned(),
                agent_type: agent_type.to_owned(),
                label,
                started_at: now,
                last_action: None,
            });
        }
        ("SubagentStop", Some(id)) => pane.running.retain(|s| s.agent_id != id),
        // shortcut: a subagent killed without SubagentStop stays counted until the session restarts.
        ("SessionStart", _) => *pane = PaneSubagents::default(),
        ("PreToolUse" | "PostToolUse" | "PostToolUseFailure", Some(id)) => {
            if let Some(sub) = pane.running.iter_mut().find(|s| s.agent_id == id)
                && let Some(action) = describe_tool(body)
            {
                sub.last_action = Some(action);
            }
        }
        ("PreToolUse", None) if matches!(body["tool_name"].as_str(), Some("Agent" | "Task")) => {
            let input = &body["tool_input"];
            if let Some(description) = input["description"].as_str().map(str::trim).filter(|d| !d.is_empty()) {
                let kind = input["subagent_type"].as_str().unwrap_or(DEFAULT_AGENT_TYPE);
                if pane.unclaimed.len() == MAX_UNCLAIMED {
                    pane.unclaimed.remove(0);
                }
                pane.unclaimed.push((kind.to_owned(), truncate(description, 80)));
            }
        }
        _ => {}
    }
    let running = pane.running.len();
    let changed = (pane.running != before).then(|| pane.running.clone());
    if running == 0 && pane.unclaimed.is_empty() {
        map.remove(pane_id);
    }
    (running, changed)
}

/// One step of the plan a Claude pane works through (`agent-steps` event: `{ paneId, steps }`):
/// an entry of its task list, or a subagent it launched when it keeps no list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: String,
    pub label: String,
    pub state: StepState,
    /// A subagent, not an entry of the task list.
    pub agent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepState {
    Pending,
    Active,
    Done,
}

const MAX_STEPS: usize = 60;

static STEPS: LazyLock<Mutex<HashMap<String, Vec<Step>>>> = LazyLock::new(Default::default);

fn task_step(id: String, label: &Value, status: Option<&str>) -> Step {
    let state = match status {
        Some("completed") => StepState::Done,
        Some("in_progress") => StepState::Active,
        _ => StepState::Pending,
    };
    Step { id, label: truncate(label.as_str().unwrap_or("").trim(), 80), state, agent: false }
}

/// Id Claude gave a task it just created: `{task: {id}}`, else the "#3" of its text answer.
fn created_task_id(response: &Value) -> Option<String> {
    match &response["task"]["id"] {
        Value::String(id) => Some(id.clone()),
        Value::Number(id) => Some(id.to_string()),
        _ => {
            let text = response.to_string();
            let digits: String = text.split_once('#')?.1.chars().take_while(char::is_ascii_digit).collect();
            (!digits.is_empty()).then_some(digits)
        }
    }
}

/// The task list when Claude keeps one, else the subagents it launched.
fn shown_steps(steps: &[Step]) -> Vec<Step> {
    let tasks = steps.iter().any(|s| !s.agent);
    steps.iter().filter(|s| !tasks || !s.agent).cloned().collect()
}

/// Records what a hook says about the pane's plan; the new step list when it changed.
/// `running`: the pane's subagents, when this hook changed them.
fn track_steps(pane_id: &str, body: &Value, running: Option<&[Subagent]>) -> Option<Vec<Step>> {
    let mut map = STEPS.lock().unwrap_or_else(PoisonError::into_inner);
    let steps = map.entry(pane_id.to_owned()).or_default();
    let before = shown_steps(steps);
    if let Some(running) = running {
        for step in steps.iter_mut().filter(|s| s.agent && s.state == StepState::Active) {
            if !running.iter().any(|sub| sub.agent_id == step.id) {
                step.state = StepState::Done;
            }
        }
        for sub in running {
            if !steps.iter().any(|s| s.agent && s.id == sub.agent_id) {
                steps.push(Step { id: sub.agent_id.clone(), label: sub.label.clone(), state: StepState::Active, agent: true });
            }
        }
    }
    let input = &body["tool_input"];
    // Subagents keep their own lists: only the main agent's plan is shown.
    let main = body["agent_id"].as_str().is_none_or(str::is_empty);
    match (body["hook_event_name"].as_str().unwrap_or(""), body["tool_name"].as_str().unwrap_or("")) {
        ("SessionStart", _) => steps.clear(),
        // A new request once everything is done: the old plan is history.
        ("UserPromptSubmit", _) if steps.iter().all(|s| s.state == StepState::Done) => steps.clear(),
        ("PostToolUse", "TodoWrite") if main => {
            steps.retain(|s| s.agent);
            let todos = input["todos"].as_array().map(Vec::as_slice).unwrap_or_default();
            steps.extend(todos.iter().enumerate().map(|(i, t)| task_step(i.to_string(), &t["content"], t["status"].as_str())));
        }
        ("PostToolUse", "TaskCreate") if main => {
            // shortcut: without an id in the answer, tasks are numbered in creation order like Claude does.
            let id = created_task_id(&body["tool_response"])
                .unwrap_or_else(|| (steps.iter().filter(|s| !s.agent).count() + 1).to_string());
            steps.push(task_step(id, &input["subject"], None));
        }
        ("PostToolUse", "TaskUpdate") if main => {
            let is_task = |s: &Step| !s.agent && Some(s.id.as_str()) == input["taskId"].as_str();
            if input["status"] == "deleted" {
                steps.retain(|s| !is_task(s));
            } else if let Some(step) = steps.iter_mut().find(|s| is_task(s)) {
                if let Some(status) = input["status"].as_str() {
                    step.state = task_step(String::new(), &Value::Null, Some(status)).state;
                }
                if let Some(subject) = input["subject"].as_str() {
                    step.label = truncate(subject.trim(), 80);
                }
            }
        }
        _ => {}
    }
    steps.truncate(MAX_STEPS);
    let after = shown_steps(steps);
    if steps.is_empty() {
        map.remove(pane_id);
    }
    (after != before).then_some(after)
}

#[cfg(test)]
fn status_event(hook: &Hook) -> Option<AgentStatusEvent> {
    let (running, _) = track_subagents(&hook.pane_id, &hook.json, 0);
    status_event_with(hook, running)
}

/// `running`: subagents of the pane still running after this hook.
fn status_event_with(hook: &Hook, running: usize) -> Option<AgentStatusEvent> {
    let (mut status, mut last_action, event) = map_event(&hook.json)?;
    // Claude ends its turn while background agents still run, and resumes when they report back:
    // not finished yet, so no "finished" notification.
    if status == PaneStatus::Done && event == "Stop" && running > 0 {
        status = PaneStatus::Working;
        let plural = if running == 1 { "" } else { "s" };
        last_action = Some(format!("Waiting for {running} background agent{plural}"));
    }
    let (prompt, summary) = details(&hook.json);
    Some(AgentStatusEvent {
        pane_id: hook.pane_id.clone(),
        status,
        last_action,
        event,
        prompt,
        summary,
        session_id: session_id(&hook.json),
        edited_file: edited_file(&hook.json),
    })
}

/// SessionStart fires before the transcript exists; any later Claude event means it can be resumed.
fn session_id(body: &Value) -> Option<String> {
    let event = body["hook_event_name"].as_str()?;
    if event == "SessionStart" {
        return None;
    }
    body["session_id"]
        .as_str()
        .filter(|id| {
            // Goes on a command line: a UUID-like id only.
            id.len() <= 64
                && id.starts_with(|c: char| c.is_ascii_alphanumeric())
                && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .map(str::to_owned)
}

fn edited_file(body: &Value) -> Option<String> {
    if body["hook_event_name"] != "PostToolUse" {
        return None;
    }
    match body["tool_name"].as_str()? {
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {}
        _ => return None,
    }
    let input = &body["tool_input"];
    input["file_path"]
        .as_str()
        .or_else(|| input["notebook_path"].as_str())
        .map(str::to_owned)
}

/// Validates and parses one request; `Err` is the HTTP status to answer.
fn handle(req: &mut Request, token: &str) -> Result<Hook, u16> {
    let (path, query) = req.url().split_once('?').unwrap_or((req.url(), ""));
    if path != "/hook" {
        return Err(404);
    }
    if *req.method() != tiny_http::Method::Post {
        return Err(405);
    }
    if !token_ok(header(req, TOKEN_HEADER).unwrap_or(""), token) {
        return Err(401);
    }
    if req.body_length().is_some_and(|n| n as u64 > MAX_BODY) {
        return Err(413);
    }
    // Note: no percent-decoding; pane ids are UUIDs.
    let query_pane = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("pane="))
        .map(str::to_owned);
    let header_pane = header(req, PANE_HEADER).map(str::to_owned);

    let mut body = Vec::new();
    req.as_reader()
        .take(MAX_BODY + 1)
        .read_to_end(&mut body)
        .map_err(|_| 400u16)?;
    if body.len() as u64 > MAX_BODY {
        return Err(413);
    }
    let json: Value = serde_json::from_slice(&body).map_err(|_| 400u16)?;

    let pane_id = [
        query_pane,
        header_pane,
        json["paneId"].as_str().map(str::to_owned),
    ]
    .into_iter()
    .flatten()
    .find(|p| !p.is_empty())
    .ok_or(400u16)?;
    Ok(Hook { pane_id, json })
}

/// Constant-time comparison (length leak is fine: tokens are fixed-length UUIDs).
pub(crate) fn token_ok(given: &str, expected: &str) -> bool {
    given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// Maps a Claude Code / Gemini CLI hook payload or a Codex `notify` payload to a pane status.
/// `None` means "no status change" (unknown or irrelevant event).
pub fn map_event(body: &Value) -> Option<(PaneStatus, Option<String>, String)> {
    let str_field = |k: &str| body[k].as_str().map(str::to_owned);
    if let Some(kind) = body["type"]
        .as_str()
        .filter(|_| body.get("hook_event_name").is_none())
    {
        // Codex notify.
        let status = match kind {
            "agent-turn-complete" => PaneStatus::Done,
            k if k.contains("approval") => PaneStatus::Waiting,
            _ => return None,
        };
        return Some((status, None, kind.to_owned()));
    }
    let event = str_field("hook_event_name")?;
    let (status, action) = match event.as_str() {
        "SessionStart" => (PaneStatus::Idle, None),
        "UserPromptSubmit" | "BeforeAgent" => (PaneStatus::Working, None),
        "PreToolUse" | "PostToolUse" | "PostToolUseFailure" | "BeforeTool" | "AfterTool" => {
            (PaneStatus::Working, describe_tool(body))
        }
        // Gemini: ToolPermission is its only notification type.
        "Notification" => match body["notification_type"].as_str() {
            None
            | Some(
                "permission_prompt" | "elicitation_dialog" | "agent_needs_input" | "ToolPermission",
            ) => (PaneStatus::Waiting, str_field("message")),
            _ => return None, // idle_prompt etc. must not override "done"
        },
        "PermissionRequest" => (PaneStatus::Waiting, describe_tool(body)),
        "Stop" | "AfterAgent" => (PaneStatus::Done, None),
        "StopFailure" => (
            PaneStatus::Error,
            str_field("error").or_else(|| str_field("error_type")),
        ),
        _ => return None,
    };
    Some((status, action, event))
}

/// (prompt, summary) carried by a payload, capped to MAX_TEXT. Never fails.
fn details(body: &Value) -> (Option<String>, Option<String>) {
    let text = |k: &str| body[k].as_str().filter(|s| !s.trim().is_empty()).map(cap);
    match body["hook_event_name"].as_str() {
        Some("UserPromptSubmit" | "BeforeAgent") => (text("prompt"), None),
        Some("AfterAgent") => (None, text("prompt_response")),
        Some("Stop") => (
            None,
            text("last_assistant_message").or_else(|| {
                // Older Claude Code versions lack last_assistant_message.
                let root = home_dir()?.join(".claude");
                let path = guarded_transcript(body["transcript_path"].as_str()?, &root)?;
                last_assistant_text(&read_tail(&path)?).map(|s| cap(&s))
            }),
        ),
        None if body["type"] == "agent-turn-complete" => {
            let inputs: Vec<&str> = body["input-messages"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let prompt = inputs.join("\n\n");
            let prompt = (!prompt.trim().is_empty()).then(|| cap(&prompt));
            (prompt, text("last-assistant-message"))
        }
        _ => (None, None),
    }
}

/// Truncates to MAX_TEXT bytes on a char boundary.
fn cap(text: &str) -> String {
    truncate(text, MAX_TEXT)
}

/// Truncates to `max` bytes on a char boundary.
fn truncate(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// The hook body is untrusted: only a `.jsonl` file under `root` (after resolving links and `..`) is read.
fn guarded_transcript(path: &str, root: &Path) -> Option<PathBuf> {
    let path = std::fs::canonicalize(path).ok()?;
    let root = std::fs::canonicalize(root).ok()?;
    (path.starts_with(&root) && path.extension()? == "jsonl" && path.is_file()).then_some(path)
}

fn read_tail(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TRANSCRIPT_TAIL)))
        .ok()?;
    let mut buf = Vec::new();
    file.take(TRANSCRIPT_TAIL).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Text of the last assistant message in a Claude transcript (JSONL). A cut first line is skipped.
fn last_assistant_text(jsonl: &str) -> Option<String> {
    jsonl
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| v["type"] == "assistant")
        .find_map(|v| {
            let content = &v["message"]["content"];
            let text = match content.as_array() {
                Some(parts) => parts
                    .iter()
                    .filter(|p| p["type"] == "text")
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                None => content.as_str()?.to_owned(),
            };
            Some(text).filter(|t| !t.trim().is_empty())
        })
}

/// "Bash: npm test", "Edit src/STYLE.md" (paths made relative to the session cwd).
fn describe_tool(body: &Value) -> Option<String> {
    let tool = body["tool_name"].as_str()?;
    let input = &body["tool_input"];
    let detail = [
        "command",
        "file_path",
        "notebook_path",
        "pattern",
        "url",
        "query",
    ]
    .iter()
    .find_map(|k| input[*k].as_str());
    let Some(detail) = detail else {
        return Some(tool.to_owned());
    };
    let detail = body["cwd"]
        .as_str()
        .and_then(|cwd| detail.strip_prefix(cwd))
        .map_or(detail, |rel| rel.trim_start_matches(['/', '\\']));
    let mut short: String = detail
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(60)
        .collect();
    if short.len() < detail.len() {
        short.push('…');
    }
    Some(if tool == "Bash" {
        format!("Bash: {short}")
    } else {
        format!("{tool} {short}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_browser_tool_calls() {
        let call = |event: &str, tool: &str| calls_browser(&json!({"hook_event_name": event, "tool_name": tool}));
        assert!(call("PreToolUse", "mcp__browser__navigate_page"));
        assert!(!call("PostToolUse", "mcp__browser__navigate_page"));
        assert!(!call("PreToolUse", "mcp__other__navigate_page"));
        assert!(!call("PreToolUse", "WebFetch"));
        let config = r#"{"mcpServers":{"browser":{"command":"npx","args":["--browserUrl","http://127.0.0.1:9333"]}}}"#;
        assert_eq!(browser_config_port(config), Some(9333));
        assert_eq!(browser_config_port("{}"), None);
    }

    #[test]
    fn tracks_steps() {
        use StepState::{Active, Done, Pending};
        let pane = "steps-pane";
        let states = |steps: Option<Vec<Step>>| steps.map(|s| s.iter().map(|s| s.state).collect::<Vec<_>>());
        let post = |tool: &str, input: Value, response: Value| {
            json!({"hook_event_name": "PostToolUse", "tool_name": tool, "tool_input": input, "tool_response": response})
        };
        // No list yet: a launched subagent is the step, done when it stops.
        let sub = Subagent { agent_id: "a1".into(), agent_type: "Explore".into(), label: "Find it".into(), started_at: 0, last_action: None };
        assert_eq!(states(track_steps(pane, &json!({}), Some(&[sub]))), Some(vec![Active]));
        assert_eq!(states(track_steps(pane, &json!({}), Some(&[]))), Some(vec![Done]));
        // A task list replaces the subagents; ids come from the answer, object or text.
        let created = track_steps(pane, &post("TaskCreate", json!({"subject": "Parse"}), json!({"task": {"id": "7"}})), None);
        assert_eq!(created.map(|s| (s.len(), s[0].label.clone())), Some((1, "Parse".to_owned())));
        track_steps(pane, &post("TaskCreate", json!({"subject": "Test"}), json!("Task #8 created successfully")), None);
        let update = |id: &str, status: &str| post("TaskUpdate", json!({"taskId": id, "status": status}), json!({}));
        assert_eq!(states(track_steps(pane, &update("7", "completed"), None)), Some(vec![Done, Pending]));
        assert_eq!(states(track_steps(pane, &update("8", "in_progress"), None)), Some(vec![Done, Active]));
        // A subagent's own tasks and unknown ids change nothing.
        let mut inner = update("8", "completed");
        inner["agent_id"] = json!("a1");
        assert_eq!(track_steps(pane, &inner, None), None);
        assert_eq!(track_steps(pane, &update("99", "completed"), None), None);
        // An unfinished plan survives the next prompt, a finished one does not.
        let prompt = json!({"hook_event_name": "UserPromptSubmit"});
        assert_eq!(track_steps(pane, &prompt, None), None);
        track_steps(pane, &update("8", "completed"), None);
        assert_eq!(track_steps(pane, &prompt, None), Some(vec![]));
        // TodoWrite carries the whole list.
        let todos = json!({"todos": [{"content": "A", "status": "completed"}, {"content": "B", "status": "pending"}]});
        assert_eq!(states(track_steps(pane, &post("TodoWrite", todos, json!({})), None)), Some(vec![Done, Pending]));
        track_steps(pane, &json!({"hook_event_name": "SessionStart"}), None);
        assert!(!STEPS.lock().unwrap().contains_key(pane));
    }

    #[test]
    fn session_id_and_edited_file() {
        let start = json!({"hook_event_name": "SessionStart", "session_id": "abc-1"});
        assert_eq!(session_id(&start), None);
        let prompt = json!({"hook_event_name": "UserPromptSubmit", "session_id": "abc-1"});
        assert_eq!(session_id(&prompt).as_deref(), Some("abc-1"));
        let forged = json!({"hook_event_name": "Stop", "session_id": "x --flag value"});
        assert_eq!(session_id(&forged), None);
        let flag = json!({"hook_event_name": "Stop", "session_id": "--some-flag"});
        assert_eq!(session_id(&flag), None);
        let edit = json!({"hook_event_name": "PostToolUse", "tool_name": "Edit",
            "tool_input": {"file_path": "/repo/a.rs"}});
        assert_eq!(edited_file(&edit).as_deref(), Some("/repo/a.rs"));
        let read = json!({"hook_event_name": "PostToolUse", "tool_name": "Read",
            "tool_input": {"file_path": "/repo/a.rs"}});
        assert_eq!(edited_file(&read), None);
    }
    use serde_json::json;

    fn status(v: Value) -> Option<(PaneStatus, Option<String>)> {
        map_event(&v).map(|(s, a, _)| (s, a))
    }

    #[test]
    fn ignores_subagent_lifecycle() {
        // The parent pane's status is untouched by subagent lifecycle events.
        assert_eq!(
            status(json!({"hook_event_name": "SubagentStart", "agent_id": "a1"})),
            None
        );
        assert_eq!(
            status(json!({"hook_event_name": "SubagentStop", "agent_id": "a1"})),
            None
        );
    }

    #[test]
    fn stop_with_background_agents_keeps_working() {
        let hook = |event: &str, agent: &str| Hook {
            pane_id: "bg-test".into(),
            json: json!({"hook_event_name": event, "agent_id": agent}),
        };
        let stop = || status_event(&hook("Stop", "")).map(|e| (e.status, e.last_action));
        assert!(status_event(&hook("SubagentStart", "a1")).is_none());
        status_event(&hook("SubagentStart", "a2"));
        assert_eq!(
            stop(),
            Some((PaneStatus::Working, Some("Waiting for 2 background agents".into())))
        );
        status_event(&hook("SubagentStop", "a1"));
        status_event(&hook("SubagentStop", "a2"));
        assert_eq!(stop(), Some((PaneStatus::Done, None)));
        status_event(&hook("SubagentStart", "a3"));
        status_event(&hook("SessionStart", ""));
        assert_eq!(stop(), Some((PaneStatus::Done, None)));
    }

    #[test]
    fn tracks_subagents() {
        let pane = "sub-test";
        let track = |body: Value, now: i64| track_subagents(pane, &body, now);
        // The main agent's Agent call labels the subagent that starts next with the same type.
        let (n, changed) = track(
            json!({"hook_event_name": "PreToolUse", "tool_name": "Agent",
                "tool_input": {"description": "Find the parser", "subagent_type": "Explore", "prompt": "x"}}),
            1,
        );
        assert_eq!((n, changed), (0, None));
        let start = |id: &str| json!({"hook_event_name": "SubagentStart", "agent_id": id, "agent_type": "Explore"});
        let (n, list) = track(start("a1"), 10);
        let list = list.unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            (list[0].label.as_str(), list[0].agent_type.as_str(), list[0].started_at),
            ("Find the parser", "Explore", 10)
        );
        // Claimed once: the next one falls back to its type.
        let (_, list) = track(start("a2"), 20);
        assert_eq!(list.unwrap()[1].label, "Explore");

        let tool = json!({"hook_event_name": "PreToolUse", "agent_id": "a1", "agent_type": "Explore",
            "tool_name": "Grep", "tool_input": {"pattern": "fn parse"}});
        let (n, list) = track(tool.clone(), 30);
        assert_eq!(n, 2);
        assert_eq!(list.unwrap()[0].last_action.as_deref(), Some("Grep fn parse"));
        // Same action again, a main-agent tool, an unknown subagent: nothing to emit.
        assert_eq!(track(tool, 31).1, None);
        let main = json!({"hook_event_name": "PreToolUse", "tool_name": "Read", "tool_input": {}});
        assert_eq!(track(main, 32).1, None);
        let unknown = json!({"hook_event_name": "PreToolUse", "agent_id": "zz", "tool_name": "Read"});
        assert_eq!(track(unknown, 33).1, None);

        let (n, list) = track(json!({"hook_event_name": "SubagentStop", "agent_id": "a1"}), 40);
        let list = list.unwrap();
        assert_eq!((n, list.len(), list[0].agent_id.as_str()), (1, 1, "a2"));
        let (n, list) = track(json!({"hook_event_name": "SessionStart"}), 50);
        assert_eq!((n, list), (0, Some(Vec::new())));
        assert!(!SUBAGENTS.lock().unwrap().contains_key(pane));
        assert_eq!(track(json!({"hook_event_name": "SessionStart"}), 51).1, None);
    }

    #[test]
    fn answered_permission_goes_back_to_working() {
        let hook = Hook {
            pane_id: "perm-test".into(),
            json: json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash",
                "tool_input": {"command": "cargo test"}}),
        };
        let waiting = status_event(&hook).unwrap();
        assert_eq!(waiting.status, PaneStatus::Waiting);
        let resolved = resolved_event(&hook);
        assert_eq!(resolved.status, PaneStatus::Working);
        assert_eq!(resolved.last_action, waiting.last_action);
        assert_eq!(resolved.event, "PermissionResolved");
        // A second request still open on the pane keeps it waiting.
        let (request, _rx) = PENDING.register(&hook.pane_id, &hook.json);
        assert!(PENDING.has_pane(&hook.pane_id));
        PENDING.remove(&request.request_id);
        assert!(!PENDING.has_pane(&hook.pane_id));
    }

    #[test]
    fn maps_claude_events() {
        assert_eq!(
            status(json!({"hook_event_name": "SessionStart"})),
            Some((PaneStatus::Idle, None))
        );
        assert_eq!(
            status(json!({"hook_event_name": "UserPromptSubmit"})),
            Some((PaneStatus::Working, None))
        );
        assert_eq!(
            status(
                json!({"hook_event_name": "PreToolUse", "cwd": "/repo", "tool_name": "Edit",
                "tool_input": {"file_path": "/repo/src/STYLE.md"}})
            ),
            Some((PaneStatus::Working, Some("Edit src/STYLE.md".into())))
        );
        let long = "x".repeat(100);
        let (_, action) = status(json!({"hook_event_name": "PreToolUse", "tool_name": "Bash",
            "tool_input": {"command": long}}))
        .unwrap();
        assert_eq!(action.unwrap(), format!("Bash: {}…", "x".repeat(60)));
        assert_eq!(
            status(
                json!({"hook_event_name": "PostToolUse", "tool_name": "TodoWrite", "tool_input": {}})
            ),
            Some((PaneStatus::Working, Some("TodoWrite".into())))
        );
        assert_eq!(
            status(
                json!({"hook_event_name": "Notification", "notification_type": "permission_prompt",
                "message": "Claude needs your permission"})
            ),
            Some((
                PaneStatus::Waiting,
                Some("Claude needs your permission".into())
            ))
        );
        assert_eq!(
            status(json!({"hook_event_name": "Notification", "notification_type": "idle_prompt"})),
            None
        );
        assert_eq!(
            status(json!({"hook_event_name": "Stop"})),
            Some((PaneStatus::Done, None))
        );
        assert_eq!(
            status(json!({"hook_event_name": "StopFailure", "error": "rate_limit"})),
            Some((PaneStatus::Error, Some("rate_limit".into())))
        );
        assert_eq!(
            status(
                json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash",
                "tool_input": {"command": "rm -rf build"}})
            ),
            Some((PaneStatus::Waiting, Some("Bash: rm -rf build".into())))
        );
        assert_eq!(status(json!({"hook_event_name": "PreCompact"})), None);
        assert_eq!(status(json!({})), None);
    }

    #[test]
    fn maps_codex_notify() {
        let (s, _, e) = map_event(&json!({"type": "agent-turn-complete", "turn-id": "1"})).unwrap();
        assert_eq!((s, e.as_str()), (PaneStatus::Done, "agent-turn-complete"));
        assert_eq!(status(json!({"type": "something-else"})), None);
    }

    #[test]
    fn maps_gemini_events() {
        let st = |v: Value| status(v).map(|(s, _)| s);
        assert_eq!(
            st(json!({"hook_event_name": "SessionStart", "source": "startup"})),
            Some(PaneStatus::Idle)
        );
        assert_eq!(
            st(json!({"hook_event_name": "BeforeAgent", "prompt": "hi"})),
            Some(PaneStatus::Working)
        );
        assert_eq!(
            status(
                json!({"hook_event_name": "BeforeTool", "tool_name": "run_shell_command",
                "tool_input": {"command": "npm test"}})
            ),
            Some((
                PaneStatus::Working,
                Some("run_shell_command npm test".into())
            ))
        );
        assert_eq!(
            st(json!({"hook_event_name": "AfterTool", "tool_name": "x"})),
            Some(PaneStatus::Working)
        );
        assert_eq!(
            status(
                json!({"hook_event_name": "Notification", "notification_type": "ToolPermission",
                "message": "Allow run_shell_command?"})
            ),
            Some((PaneStatus::Waiting, Some("Allow run_shell_command?".into())))
        );
        assert_eq!(
            st(json!({"hook_event_name": "AfterAgent"})),
            Some(PaneStatus::Done)
        );
        assert_eq!(
            details(&json!({"hook_event_name": "BeforeAgent", "prompt": "fix bug"})),
            (Some("fix bug".into()), None)
        );
        assert_eq!(
            details(
                &json!({"hook_event_name": "AfterAgent", "prompt": "fix bug", "prompt_response": "Fixed."})
            ),
            (None, Some("Fixed.".into()))
        );
    }

    #[test]
    fn handle_over_http() {
        use std::io::Write;
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        let send = |headers: &str, body: &str| {
            let mut s = std::net::TcpStream::connect(addr).unwrap();
            write!(s, "POST /hook?pane=p1&agent=claude HTTP/1.1\r\nHost: x\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            let mut req = server.recv().unwrap();
            handle(&mut req, "tok")
        };
        assert_eq!(send("", "{}").unwrap_err(), 401);
        assert_eq!(send("X-AgentDeck-Token: bad\r\n", "{}").unwrap_err(), 401);
        assert_eq!(
            send("X-AgentDeck-Token: tok\r\n", "not json").unwrap_err(),
            400
        );
        let hook = send(
            "X-AgentDeck-Token: tok\r\n",
            r#"{"hook_event_name":"Stop"}"#,
        )
        .unwrap();
        let ev = status_event(&hook).unwrap();
        assert_eq!((ev.pane_id.as_str(), ev.status), ("p1", PaneStatus::Done));
    }

    #[test]
    fn extracts_prompt_and_summary() {
        assert_eq!(
            details(&json!({"hook_event_name": "UserPromptSubmit", "prompt": "fix bug"})),
            (Some("fix bug".into()), None)
        );
        assert_eq!(
            details(&json!({"hook_event_name": "Stop", "last_assistant_message": "Done."})),
            (None, Some("Done.".into()))
        );
        assert_eq!(
            details(&json!({"hook_event_name": "PreToolUse", "prompt": "x"})),
            (None, None)
        );
        assert_eq!(
            details(
                &json!({"type": "agent-turn-complete", "input-messages": ["a", "b"],
                "last-assistant-message": "ok"})
            ),
            (Some("a\n\nb".into()), Some("ok".into()))
        );
        assert_eq!(
            details(&json!({"type": "agent-turn-complete"})),
            (None, None)
        );
    }

    #[test]
    fn cap_is_char_safe() {
        let long = "é".repeat(MAX_TEXT); // 2 bytes each
        assert_eq!(cap(&long).len(), MAX_TEXT);
        assert_eq!(cap(&format!("a{long}")).len(), MAX_TEXT - 1);
        assert_eq!(cap("abc"), "abc");
    }

    #[test]
    fn parses_transcript_tail() {
        let jsonl = [
            r#"tial line {"#,
            r#"{"type":"user","message":{"role":"user","content":"hi"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"First"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Line 1"},{"type":"tool_use","name":"Bash"},{"type":"text","text":"Line 2"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read"}]}}"#,
            r#"{"type":"system"}"#,
        ]
        .join("\n");
        assert_eq!(last_assistant_text(&jsonl), Some("Line 1\nLine 2".into()));
        assert_eq!(last_assistant_text("garbage"), None);
    }

    #[test]
    fn transcript_path_guard() {
        let dir = std::env::temp_dir().join(format!("agentdeck-guard-{}", uuid::Uuid::new_v4()));
        let root = dir.join(".claude");
        std::fs::create_dir_all(root.join("projects")).unwrap();
        let ok = root.join("projects").join("t.jsonl");
        let txt = root.join("projects").join("t.txt");
        let outside = dir.join("t.jsonl");
        for f in [&ok, &txt, &outside] {
            std::fs::write(f, "{}").unwrap();
        }
        let s = |p: &Path| p.to_str().unwrap().to_owned();
        assert!(guarded_transcript(&s(&ok), &root).is_some());
        assert!(guarded_transcript(&s(&txt), &root).is_none());
        assert!(guarded_transcript(&s(&outside), &root).is_none());
        let escape = root.join("projects").join("..").join("..").join("t.jsonl");
        assert!(guarded_transcript(&s(&escape), &root).is_none());
        assert!(guarded_transcript(&s(&root.join("missing.jsonl")), &root).is_none());
        assert_eq!(read_tail(&ok).as_deref(), Some("{}"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn token_check() {
        assert!(token_ok("abc", "abc"));
        assert!(!token_ok("abd", "abc"));
        assert!(!token_ok("ab", "abc"));
        assert!(!token_ok("", "abc"));
    }
}
