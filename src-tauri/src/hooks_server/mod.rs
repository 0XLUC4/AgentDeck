//! Local HTTP server (127.0.0.1 only, secret token) receiving agent hook events
//! and re-emitting them to the UI as `agent-status`.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub mod permissions;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
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

pub struct HookServer {
    pub port: u16,
    pub token: String,
}

impl HookServer {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/hook", self.port)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneStatus {
    Idle,
    Working,
    Waiting,
    Done,
    Error,
}

/// Payload of the `agent-status` event.
#[derive(Debug, Clone, Serialize)]
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
    if let Some(event) = status_event(hook) {
        crate::mcp::STATUS.set(&event.pane_id, event.status);
        let _ = app.emit("agent-status", event);
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

fn status_event(hook: &Hook) -> Option<AgentStatusEvent> {
    let (status, last_action, event) = map_event(&hook.json)?;
    let (prompt, summary) = details(&hook.json);
    Some(AgentStatusEvent {
        pane_id: hook.pane_id.clone(),
        status,
        last_action,
        event,
        prompt,
        summary,
    })
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

/// Maps a Claude Code hook payload or a Codex `notify` payload to a pane status.
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
        "UserPromptSubmit" => (PaneStatus::Working, None),
        "PreToolUse" | "PostToolUse" | "PostToolUseFailure" => {
            (PaneStatus::Working, describe_tool(body))
        }
        "Notification" => match body["notification_type"].as_str() {
            None | Some("permission_prompt" | "elicitation_dialog" | "agent_needs_input") => {
                (PaneStatus::Waiting, str_field("message"))
            }
            _ => return None, // idle_prompt etc. must not override "done"
        },
        "PermissionRequest" => (PaneStatus::Waiting, describe_tool(body)),
        "Stop" => (PaneStatus::Done, None),
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
        Some("UserPromptSubmit") => (text("prompt"), None),
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
