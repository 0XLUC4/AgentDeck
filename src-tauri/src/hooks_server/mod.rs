//! Local HTTP server (127.0.0.1 only, secret token) receiving agent hook events
//! and re-emitting them to the UI as `agent-status`.

use std::io::Read;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tiny_http::{Request, Response, Server};

/// Header carrying the per-run secret; also how our Claude hook entries are recognized.
pub const TOKEN_HEADER: &str = "X-AgentDeck-Token";
const PANE_HEADER: &str = "X-AgentDeck-Pane";
/// Preferred port keeps `.claude/settings.local.json` stable across app runs.
const PREFERRED_PORT: u16 = 47800;
const MAX_BODY: u64 = 1024 * 1024;

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
}

/// Starts the hook server on a background thread.
pub fn start(app: AppHandle) -> HookServer {
    let token = uuid::Uuid::new_v4().to_string();
    // shortcut: a second AgentDeck instance falls back to a random port and rewrites the Claude settings.
    let server = Server::http(("127.0.0.1", PREFERRED_PORT)).or_else(|_| Server::http("127.0.0.1:0"));
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
            let code = match handle(&mut req, &expected) {
                Ok(Some(event)) => {
                    let _ = app.emit("agent-status", event);
                    204
                }
                Ok(None) => 204,
                Err(code) => code,
            };
            let _ = req.respond(Response::empty(code));
        }
    });
    HookServer { port, token }
}

fn header<'a>(req: &'a Request, name: &'static str) -> Option<&'a str> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
}

/// Validates and parses one request; `Err` is the HTTP status to answer.
fn handle(req: &mut Request, token: &str) -> Result<Option<AgentStatusEvent>, u16> {
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
    // shortcut: no percent-decoding; pane ids are UUIDs.
    let query_pane = query.split('&').find_map(|kv| kv.strip_prefix("pane=")).map(str::to_owned);
    let header_pane = header(req, PANE_HEADER).map(str::to_owned);

    let mut body = Vec::new();
    req.as_reader().take(MAX_BODY + 1).read_to_end(&mut body).map_err(|_| 400u16)?;
    if body.len() as u64 > MAX_BODY {
        return Err(413);
    }
    let json: Value = serde_json::from_slice(&body).map_err(|_| 400u16)?;

    let pane_id = [query_pane, header_pane, json["paneId"].as_str().map(str::to_owned)]
        .into_iter()
        .flatten()
        .find(|p| !p.is_empty())
        .ok_or(400u16)?;
    Ok(map_event(&json).map(|(status, last_action, event)| AgentStatusEvent { pane_id, status, last_action, event }))
}

/// Constant-time comparison (length leak is fine: tokens are fixed-length UUIDs).
fn token_ok(given: &str, expected: &str) -> bool {
    given.len() == expected.len()
        && given.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// Maps a Claude Code hook payload or a Codex `notify` payload to a pane status.
/// `None` means "no status change" (unknown or irrelevant event).
pub fn map_event(body: &Value) -> Option<(PaneStatus, Option<String>, String)> {
    let str_field = |k: &str| body[k].as_str().map(str::to_owned);
    if let Some(kind) = body["type"].as_str().filter(|_| body.get("hook_event_name").is_none()) {
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
        "PreToolUse" | "PostToolUse" | "PostToolUseFailure" => (PaneStatus::Working, describe_tool(body)),
        "Notification" => match body["notification_type"].as_str() {
            None | Some("permission_prompt" | "elicitation_dialog" | "agent_needs_input") => {
                (PaneStatus::Waiting, str_field("message"))
            }
            _ => return None, // idle_prompt etc. must not override "done"
        },
        "Stop" => (PaneStatus::Done, None),
        "StopFailure" => (PaneStatus::Error, str_field("error_type")),
        _ => return None,
    };
    Some((status, action, event))
}

/// "Bash: npm test", "Edit src/STYLE.md" (paths made relative to the session cwd).
fn describe_tool(body: &Value) -> Option<String> {
    let tool = body["tool_name"].as_str()?;
    let input = &body["tool_input"];
    let detail = ["command", "file_path", "notebook_path", "pattern", "url", "query"]
        .iter()
        .find_map(|k| input[*k].as_str());
    let Some(detail) = detail else { return Some(tool.to_owned()) };
    let detail = body["cwd"]
        .as_str()
        .and_then(|cwd| detail.strip_prefix(cwd))
        .map_or(detail, |rel| rel.trim_start_matches(['/', '\\']));
    let mut short: String = detail.lines().next().unwrap_or("").chars().take(60).collect();
    if short.len() < detail.len() {
        short.push('…');
    }
    Some(if tool == "Bash" { format!("Bash: {short}") } else { format!("{tool} {short}") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn status(v: Value) -> Option<(PaneStatus, Option<String>)> {
        map_event(&v).map(|(s, a, _)| (s, a))
    }

    #[test]
    fn maps_claude_events() {
        assert_eq!(status(json!({"hook_event_name": "SessionStart"})), Some((PaneStatus::Idle, None)));
        assert_eq!(status(json!({"hook_event_name": "UserPromptSubmit"})), Some((PaneStatus::Working, None)));
        assert_eq!(
            status(json!({"hook_event_name": "PreToolUse", "cwd": "/repo", "tool_name": "Edit",
                "tool_input": {"file_path": "/repo/src/STYLE.md"}})),
            Some((PaneStatus::Working, Some("Edit src/STYLE.md".into())))
        );
        let long = "x".repeat(100);
        let (_, action) = status(json!({"hook_event_name": "PreToolUse", "tool_name": "Bash",
            "tool_input": {"command": long}})).unwrap();
        assert_eq!(action.unwrap(), format!("Bash: {}…", "x".repeat(60)));
        assert_eq!(
            status(json!({"hook_event_name": "PostToolUse", "tool_name": "TodoWrite", "tool_input": {}})),
            Some((PaneStatus::Working, Some("TodoWrite".into())))
        );
        assert_eq!(
            status(json!({"hook_event_name": "Notification", "notification_type": "permission_prompt",
                "message": "Claude needs your permission"})),
            Some((PaneStatus::Waiting, Some("Claude needs your permission".into())))
        );
        assert_eq!(status(json!({"hook_event_name": "Notification", "notification_type": "idle_prompt"})), None);
        assert_eq!(status(json!({"hook_event_name": "Stop"})), Some((PaneStatus::Done, None)));
        assert_eq!(
            status(json!({"hook_event_name": "StopFailure", "error_type": "rate_limit"})),
            Some((PaneStatus::Error, Some("rate_limit".into())))
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
        assert_eq!(send("X-AgentDeck-Token: tok\r\n", "not json").unwrap_err(), 400);
        let ev = send("X-AgentDeck-Token: tok\r\n", r#"{"hook_event_name":"Stop"}"#).unwrap().unwrap();
        assert_eq!((ev.pane_id.as_str(), ev.status), ("p1", PaneStatus::Done));
    }

    #[test]
    fn token_check() {
        assert!(token_ok("abc", "abc"));
        assert!(!token_ok("abd", "abc"));
        assert!(!token_ok("ab", "abc"));
        assert!(!token_ok("", "abc"));
    }
}
