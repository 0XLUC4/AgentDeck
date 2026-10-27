//! MCP server for the orchestrator pane, served at `/mcp` on the hook server.
//! Streamable HTTP transport, plain JSON responses only (no SSE, no session id: both optional).
//! Ref: https://modelcontextprotocol.io/specification/2025-06-18/basic/transports
//!
//! Only panes registered with `mcp_register` may call it, with the per-spawn secret that only their
//! process gets (env `AGENTDECK_MCP_SECRET`): the hook token alone, known to every agent, is not enough. Scope checks (same workspace, not itself,
//! not waiting on a permission) and pane actions run in the UI, which answers via `mcp_respond`.

pub mod desktop;
mod rpc;
mod status;

use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};
use tiny_http::{Header, Request, Response};

use crate::AppState;
use crate::hooks_server::permissions::wait_decision;
use crate::hooks_server::{MAX_BODY, PANE_HEADER, TOKEN_HEADER, header, token_ok};
pub use status::STATUS;

/// How long a tool call waits for the UI (spawn included).
const FRONTEND_HOLD: Duration = Duration::from_secs(30);
const MAIN_WINDOW: &str = "main";
const SECRET_HEADER: &str = "X-AgentDeck-Mcp";
const VERSION_HEADER: &str = "MCP-Protocol-Version";

/// Orchestrator pane id -> its current secret (regenerated on each spawn).
static ORCHESTRATORS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);
static PAUSED: AtomicBool = AtomicBool::new(false);
/// UI requests in flight: request id -> channel of `{result}` / `{error}`.
static WAITING: LazyLock<Mutex<HashMap<String, Sender<Option<Value>>>>> =
    LazyLock::new(Default::default);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Answers one HTTP request on `/mcp`.
pub fn serve(app: &AppHandle, mut req: Request, token: &str) {
    let reply = match parse(&mut req, token, SECRET_HEADER, &ORCHESTRATORS) {
        Err(code) => Response::empty(code).boxed(),
        Ok((_, msg)) if !msg.is_object() => {
            json_reply(400, &rpc::error(Value::Null, -32600, "Invalid Request"))
        }
        Ok((pane_id, msg)) => {
            let host = AppHost { app, pane_id };
            if !rpc::Host::alive(&host, &host.pane_id) {
                // Exited or closed: its secret dies with it.
                lock(&ORCHESTRATORS).remove(&host.pane_id);
                let _ = req.respond(Response::empty(403));
                return;
            }
            match rpc::handle(&msg, &host) {
                Some(body) => json_reply(200, &body),
                None => Response::empty(202).boxed(),
            }
        }
    };
    let _ = req.respond(reply);
}

fn json_reply(code: u16, body: &Value) -> tiny_http::ResponseBox {
    let ct = Header::from_bytes("Content-Type", "application/json").expect("static header");
    Response::from_string(body.to_string())
        .with_status_code(code)
        .with_header(ct)
        .boxed()
}

/// Authenticates a request from a pane registered in `registry` (pane id -> secret, sent in
/// `secret_header`); `Err` is the HTTP status to answer.
fn parse(
    req: &mut Request,
    token: &str,
    secret_header: &'static str,
    registry: &Mutex<HashMap<String, String>>,
) -> Result<(String, Value), u16> {
    if *req.method() != tiny_http::Method::Post {
        return Err(405); // no SSE stream (GET) nor session to delete (DELETE)
    }
    // Spec: validate Origin against DNS rebinding. Our clients (CLIs) send none; browsers always do.
    if header(req, "Origin").is_some() {
        return Err(403);
    }
    if !token_ok(header(req, TOKEN_HEADER).unwrap_or(""), token) {
        return Err(401);
    }
    let pane_id = header(req, PANE_HEADER).unwrap_or("").to_owned();
    let secret_ok = lock(registry)
        .get(&pane_id)
        .is_some_and(|s| token_ok(header(req, secret_header).unwrap_or(""), s));
    if !secret_ok {
        return Err(403);
    }
    // Spec: a present but unsupported protocol version gets 400 (absent: assume a supported one).
    if header(req, VERSION_HEADER).is_some_and(|v| !rpc::VERSIONS.contains(&v)) {
        return Err(400);
    }
    let mut body = Vec::new();
    req.as_reader()
        .take(MAX_BODY + 1)
        .read_to_end(&mut body)
        .map_err(|_| 400u16)?;
    if body.len() as u64 > MAX_BODY {
        return Err(413);
    }
    let msg = serde_json::from_slice(&body).map_err(|_| 400u16)?;
    Ok((pane_id, msg))
}

struct AppHost<'a> {
    app: &'a AppHandle,
    pane_id: String,
}

impl rpc::Host for AppHost<'_> {
    fn frontend(&self, tool: &str, args: &Value) -> Result<Value, String> {
        let (tx, rx) = channel();
        let request_id = uuid::Uuid::new_v4().to_string();
        lock(&WAITING).insert(request_id.clone(), tx);
        let event = json!({
            "requestId": request_id,
            "orchestratorId": self.pane_id,
            "tool": tool,
            "args": args,
        });
        let sent = self.app.emit_to(MAIN_WINDOW, "mcp-request", event);
        let answer = sent
            .ok()
            .and_then(|()| wait_decision(&rx, FRONTEND_HOLD, || true));
        lock(&WAITING).remove(&request_id);
        let answer = answer.ok_or("AgentDeck did not answer in time")?;
        match answer["error"].as_str() {
            Some(e) => Err(e.to_owned()),
            None => Ok(answer["result"].clone()),
        }
    }

    fn paused(&self) -> bool {
        PAUSED.load(Ordering::Relaxed)
    }

    fn alive(&self, pane_id: &str) -> bool {
        self.app
            .try_state::<AppState>()
            .is_some_and(|s| s.panes.0.lock().is_ok_and(|p| p.contains_key(pane_id)))
    }

    fn statuses(&self) -> &status::Statuses {
        &STATUS
    }

    fn activity(&self, tool: &str, args: &Value, error: Option<&str>) {
        let _ = self.app.emit_to(
            MAIN_WINDOW,
            "mcp-activity",
            json!({ "orchestratorId": self.pane_id, "tool": tool, "args": args, "error": error }),
        );
    }
}

/// Answers a `mcp-request` event. Exactly one of `result` / `error` is expected.
#[tauri::command]
pub fn mcp_respond(
    request_id: String,
    result: Option<Value>,
    error: Option<String>,
) -> Result<(), String> {
    let tx = lock(&WAITING)
        .remove(&request_id)
        .ok_or("Request expired or already handled")?;
    let _ = tx.send(Some(json!({ "result": result, "error": error })));
    Ok(())
}

#[tauri::command]
pub fn mcp_set_paused(paused: bool) {
    PAUSED.store(paused, Ordering::Relaxed);
}

#[derive(Serialize)]
pub struct Registration {
    /// Claude Code args to prepend (files, not inline JSON: spawn args forbid `"` on Windows).
    args: Vec<String>,
    /// To pass to `spawn_pane` as env `AGENTDECK_MCP_SECRET`; never written anywhere.
    secret: String,
}

/// Marks `pane_id` as an orchestrator with a fresh secret and writes its MCP config and role prompt.
/// Called before every spawn: a restart invalidates the previous process's secret.
#[tauri::command]
pub fn mcp_register(
    app: AppHandle,
    state: State<'_, AppState>,
    pane_id: String,
) -> Result<Registration, String> {
    if pane_id.is_empty()
        || !pane_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("invalid pane id".into());
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("orchestrator");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let config = dir.join("mcp.json");
    let prompt = dir.join("role.md");
    let write = |path: &std::path::Path, text: &str| {
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    };
    write(
        &config,
        &serde_json::to_string_pretty(&mcp_config(state.hooks.port)).unwrap_or_default(),
    )?;
    write(&prompt, ROLE_PROMPT)?;
    let secret = uuid::Uuid::new_v4().to_string();
    lock(&ORCHESTRATORS).insert(pane_id, secret.clone());
    let path = |p: std::path::PathBuf| p.to_string_lossy().into_owned();
    // Variadic flags (--mcp-config, --allowedTools) are each closed by the next flag.
    let args = vec![
        "--mcp-config".into(),
        path(config),
        "--allowedTools".into(),
        "mcp__agentdeck".into(),
        "--append-system-prompt-file".into(),
        path(prompt),
    ];
    Ok(Registration { args, secret })
}

/// Token, pane id and secret are expanded by Claude Code from the pane's env: the file holds no secret.
fn mcp_config(port: u16) -> Value {
    json!({ "mcpServers": { "agentdeck": {
        "type": "http",
        "url": format!("http://127.0.0.1:{port}/mcp"),
        "headers": {
            TOKEN_HEADER: "${AGENTDECK_HOOK_TOKEN}",
            PANE_HEADER: "${AGENTDECK_PANE_ID}",
            SECRET_HEADER: "${AGENTDECK_MCP_SECRET}",
        },
    }}})
}

const ROLE_PROMPT: &str = "\
# Role: AgentDeck orchestrator

You coordinate the other coding agents in this workspace (Claude Code, Codex, Gemini, Grok, shell) \
through the `agentdeck` MCP tools. You do not write code yourself except for small checks.

Method:
1. Understand the request, then split it into independent, well-scoped subtasks.
2. Check existing agents with `list_agents`; start new ones with `spawn_agent` \
(one clear title per subtask, `worktree: true` if several agents change the same repository).
3. Send each one a self-contained, precise prompt with `send_prompt` (context, files, definition of done).
4. Wait with `wait_for` (until: \"done\", or \"any\" to react to the first one) instead of polling in a loop.
5. Check with `get_result` and `get_diff` (and `read_output` if needed); retry or correct if the result \
is not right, or undo an agent's last task with `restore_checkpoint`.
6. Integrate finished worktree work with `merge_worktree`, close agents you no longer need with `close_agent`.
7. End with a summary: what was done, by which agent, what is left to verify.

Rules:
- If an agent has status \"waiting\", it is waiting on a permission: the user answers, not you. \
Report it and wait.
- You cannot act outside your workspace. Only restore, merge or close when the work calls for it.
- If a tool answers \"paused by the user\", stop and wait for their instructions.
- Reply in the user's language.
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_uses_env_expansion_and_no_secret() {
        let c = mcp_config(47800);
        let server = &c["mcpServers"]["agentdeck"];
        assert_eq!(server["type"], "http");
        assert_eq!(server["url"], "http://127.0.0.1:47800/mcp");
        assert_eq!(server["headers"][TOKEN_HEADER], "${AGENTDECK_HOOK_TOKEN}");
        assert_eq!(server["headers"][PANE_HEADER], "${AGENTDECK_PANE_ID}");
        assert_eq!(server["headers"][SECRET_HEADER], "${AGENTDECK_MCP_SECRET}");
    }

    #[test]
    fn respond_unknown_request_fails() {
        assert!(mcp_respond("nope".into(), None, None).is_err());
        let (tx, rx) = channel();
        lock(&WAITING).insert("r1".into(), tx);
        mcp_respond("r1".into(), Some(json!({ "paneId": "p" })), None).unwrap();
        assert_eq!(rx.recv().unwrap().unwrap()["result"]["paneId"], "p");
    }

    #[test]
    fn http_auth_and_routing() {
        use std::io::Write;
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        lock(&ORCHESTRATORS).insert("orch-1".into(), "s3cret".into());
        let send = |method: &str, headers: &str| {
            let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
            let mut s = std::net::TcpStream::connect(addr).unwrap();
            write!(s, "{method} /mcp HTTP/1.1\r\nHost: x\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            let mut req = server.recv().unwrap();
            parse(&mut req, "tok", SECRET_HEADER, &ORCHESTRATORS)
                .map(|(p, m)| (p, m["method"].clone()))
        };
        // Hook token + orchestrator pane id without its secret: what any other agent could forge.
        let no_secret = "X-AgentDeck-Token: tok\r\nX-AgentDeck-Pane: orch-1\r\n";
        let ok = &format!("{no_secret}X-AgentDeck-Mcp: s3cret\r\n");
        assert_eq!(send("POST", no_secret).unwrap_err(), 403);
        assert_eq!(
            send("POST", &format!("{no_secret}X-AgentDeck-Mcp: s3creT\r\n")).unwrap_err(),
            403
        );
        assert_eq!(
            send("POST", &format!("{ok}MCP-Protocol-Version: 1999-01-01\r\n")).unwrap_err(),
            400
        );
        assert!(send("POST", &format!("{ok}MCP-Protocol-Version: 2025-06-18\r\n")).is_ok());
        assert_eq!(send("GET", ok).unwrap_err(), 405);
        assert_eq!(
            send(
                "POST",
                "X-AgentDeck-Token: bad\r\nX-AgentDeck-Pane: orch-1\r\n"
            )
            .unwrap_err(),
            401
        );
        assert_eq!(
            send(
                "POST",
                "X-AgentDeck-Token: tok\r\nX-AgentDeck-Pane: other\r\n"
            )
            .unwrap_err(),
            403
        );
        assert_eq!(
            send("POST", &format!("{ok}Origin: http://evil.example\r\n")).unwrap_err(),
            403
        );
        assert_eq!(
            send("POST", ok).unwrap(),
            ("orch-1".to_owned(), json!("ping"))
        );
    }
}
