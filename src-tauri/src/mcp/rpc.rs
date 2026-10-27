//! JSON-RPC 2.0 dispatch and the orchestrator tools. Transport-free so it can be tested with a fake host.
//! Ref: https://modelcontextprotocol.io/specification/2025-06-18/server/tools

use std::time::Duration;

use serde_json::{Value, json};

use super::status::{self, Statuses, Until};
use crate::agents::AgentKind;
use crate::hooks_server::PaneStatus;

/// Versions we can speak (we only use initialize/ping/tools, unchanged across these).
pub(super) const VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
pub const PAUSED_MSG: &str = "Orchestrator paused by the user";
const MUTATING: [&str; 6] = [
    "spawn_agent",
    "send_prompt",
    "interrupt",
    "merge_worktree",
    "restore_checkpoint",
    "close_agent",
];
const MAX_PROMPT: usize = 64 * 1024;
const MAX_WAIT_S: u64 = 600;
const MAX_LINES: u64 = 200;

/// What the tools need from the app. The real host bridges to the frontend; tests fake it.
pub trait Host {
    /// Runs `tool` in the UI (scope checks, store reads, PTY writes). The UI answers with JSON or an error.
    fn frontend(&self, tool: &str, args: &Value) -> Result<Value, String>;
    fn paused(&self) -> bool;
    fn alive(&self, pane_id: &str) -> bool;
    fn statuses(&self) -> &Statuses;
    /// Called once per tool call, for the UI activity log.
    fn activity(&self, tool: &str, args: &Value, error: Option<&str>);
}

/// Handles one JSON-RPC message. `None`: notification or client response (HTTP 202, no body).
pub fn handle(msg: &Value, host: &impl Host) -> Option<Value> {
    let (Some(id), Some(method)) = (msg.get("id"), msg.get("method")) else {
        return None;
    };
    let result = match method.as_str().unwrap_or("") {
        "initialize" => Ok(initialize(&msg["params"])),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": definitions() })),
        "tools/call" => call(&msg["params"], host),
        other => Err((-32601, format!("Method not found: {other}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => error(id.clone(), code, &message),
    })
}

pub fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize(params: &Value) -> Value {
    let asked = params["protocolVersion"].as_str().unwrap_or("");
    let version = VERSIONS
        .iter()
        .find(|v| **v == asked)
        .unwrap_or(&VERSIONS[1]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "agentdeck", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "AgentDeck tools to drive the other agents in your workspace.",
    })
}

fn definitions() -> Value {
    let pane = json!({ "type": "string", "description": "paneId (see list_agents)" });
    let tool = |name: &str, description: &str, properties: Value, required: &[&str]| {
        json!({
            "name": name,
            "description": description,
            "inputSchema": { "type": "object", "properties": properties, "required": required },
        })
    };
    json!([
        tool(
            "list_agents",
            "Lists the agents in your workspace: paneId, agent, title, status, last action, session, folder.",
            json!({}),
            &[]
        ),
        tool(
            "spawn_agent",
            "Starts a new agent in your session. Default cwd: the workspace folder (must stay inside it).",
            json!({
                "agent": { "type": "string", "enum": ["claude", "codex", "gemini", "grok", "shell"] },
                "title": { "type": "string" },
                "cwd": { "type": "string" },
                "worktree": { "type": "boolean", "description": "Dedicated Git worktree" },
                "args": { "type": "array", "items": { "type": "string" } },
            }),
            &["agent", "title"]
        ),
        tool(
            "send_prompt",
            "Sends a prompt to an agent (submitted by default). Refused if the agent is waiting on a permission.",
            json!({ "paneId": pane, "text": { "type": "string" }, "submit": { "type": "boolean", "default": true } }),
            &["paneId", "text"]
        ),
        tool(
            "get_status",
            "An agent's status, last action and time spent in that status.",
            json!({ "paneId": pane }),
            &["paneId"]
        ),
        tool(
            "get_result",
            "An agent's last finished task: prompt, summary, actions (last 30), status.",
            json!({ "paneId": pane }),
            &["paneId"]
        ),
        tool(
            "wait_for",
            "Waits until agents are finished (done), waiting for an answer (waiting), or either (any).",
            json!({
                "paneIds": { "type": "array", "items": { "type": "string" } },
                "until": { "type": "string", "enum": ["done", "waiting", "any"] },
                "timeoutSec": { "type": "integer", "minimum": 1, "maximum": MAX_WAIT_S, "default": 300 },
            }),
            &["paneIds", "until"]
        ),
        tool(
            "read_output",
            "Last lines of an agent's terminal, as plain text.",
            json!({ "paneId": pane, "lines": { "type": "integer", "minimum": 1, "maximum": MAX_LINES, "default": 50 } }),
            &["paneId"]
        ),
        tool(
            "interrupt",
            "Interrupts an agent's current task (Esc / Ctrl+C). The session stays open.",
            json!({ "paneId": pane }),
            &["paneId"]
        ),
        tool(
            "get_diff",
            "An agent's Git changes vs HEAD (untracked files included): branch, changed files, restore points, and the unified diff (of `path` only when given; capped).",
            json!({ "paneId": pane, "path": { "type": "string", "description": "One file, relative to the repo" } }),
            &["paneId"]
        ),
        tool(
            "merge_worktree",
            "Merges an agent's AgentDeck worktree branch into the main checkout (--no-ff). Uncommitted changes are committed first with commitMessage, or the merge is refused.",
            json!({ "paneId": pane, "commitMessage": { "type": "string" } }),
            &["paneId"]
        ),
        tool(
            "restore_checkpoint",
            "Puts an agent's files back as they were at a restore point (one is taken before each task). Default: the latest one, i.e. undo its last task. The current state is saved first.",
            json!({ "paneId": pane, "sha": { "type": "string", "description": "From get_diff's checkpoints" } }),
            &["paneId"]
        ),
        tool(
            "close_agent",
            "Stops an agent and closes its pane. Its worktree and files stay on disk.",
            json!({ "paneId": pane }),
            &["paneId"]
        ),
    ])
}

fn call(params: &Value, host: &impl Host) -> Result<Value, (i64, String)> {
    let name = params["name"].as_str().unwrap_or("");
    let known = definitions()
        .as_array()
        .is_some_and(|tools| tools.iter().any(|t| t["name"] == name));
    if !known {
        return Err((-32602, format!("Unknown tool: {name}")));
    }
    let empty = json!({});
    let args = params.get("arguments").unwrap_or(&empty);
    let outcome = run(name, args, host);
    host.activity(name, args, outcome.as_ref().err().map(String::as_str));
    // Tool failures are results with isError so the model can read and recover from them.
    Ok(match outcome {
        Ok(value) => {
            let text = serde_json::to_string_pretty(&value).unwrap_or_default();
            json!({ "content": [{ "type": "text", "text": text }] })
        }
        Err(message) => {
            json!({ "content": [{ "type": "text", "text": message }], "isError": true })
        }
    })
}

fn run(name: &str, args: &Value, host: &impl Host) -> Result<Value, String> {
    if MUTATING.contains(&name) && host.paused() {
        return Err(PAUSED_MSG.into());
    }
    match name {
        "list_agents" => host.frontend(name, &json!({})),
        "spawn_agent" => host.frontend(name, &spawn_args(args)?),
        "send_prompt" => {
            let pane_id = pane_arg(args)?;
            let text = args["text"].as_str().ok_or("missing text")?;
            if text.trim().is_empty() || text.len() > MAX_PROMPT {
                return Err(format!(
                    "text must be non-empty and at most {MAX_PROMPT} bytes"
                ));
            }
            let submit = bool_arg(args, "submit", true)?;
            let result = host.frontend(
                name,
                &json!({ "paneId": pane_id, "text": text, "submit": submit }),
            )?;
            // A wait_for right after must not see the previous turn's "done".
            if submit {
                host.statuses().set(pane_id, PaneStatus::Working);
            }
            Ok(result)
        }
        "get_status" | "get_result" | "interrupt" => {
            host.frontend(name, &json!({ "paneId": pane_arg(args)? }))
        }
        "read_output" => {
            let pane_id = pane_arg(args)?;
            let lines = uint_arg(args, "lines", 50, MAX_LINES)? as usize;
            let raw = host.frontend(name, &json!({ "paneId": pane_id }))?;
            let text = last_lines(raw["raw"].as_str().unwrap_or(""), lines);
            Ok(json!({ "paneId": pane_id, "output": text }))
        }
        "wait_for" => wait_for(args, host),
        "get_diff" => host.frontend(
            name,
            &json!({ "paneId": pane_arg(args)?, "path": opt_str(args, "path", 1024)? }),
        ),
        "merge_worktree" => host.frontend(
            name,
            &json!({ "paneId": pane_arg(args)?, "commitMessage": opt_str(args, "commitMessage", 4096)? }),
        ),
        "restore_checkpoint" => {
            let sha = opt_str(args, "sha", 64)?;
            if sha.is_some_and(|s| !s.bytes().all(|b| b.is_ascii_hexdigit())) {
                return Err("sha must be hexadecimal".into());
            }
            host.frontend(name, &json!({ "paneId": pane_arg(args)?, "sha": sha }))
        }
        "close_agent" => host.frontend(name, &json!({ "paneId": pane_arg(args)? })),
        _ => Err(format!("unknown tool: {name}")),
    }
}

fn wait_for(args: &Value, host: &impl Host) -> Result<Value, String> {
    let ids: Vec<String> = args["paneIds"]
        .as_array()
        .ok_or("paneIds must be a list")?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or("paneIds: strings expected")
        })
        .collect::<Result<_, _>>()?;
    if ids.is_empty() || ids.len() > 20 {
        return Err("paneIds: between 1 and 20 agents".into());
    }
    let until = Until::parse(args["until"].as_str().unwrap_or(""))
        .ok_or("until must be done, waiting or any")?;
    let timeout = uint_arg(args, "timeoutSec", 300, MAX_WAIT_S)?;
    // The UI checks the scope and reports its view of each pane (covers agents without hooks).
    let current = host.frontend("wait_for", &json!({ "paneIds": ids }))?;
    for id in &ids {
        if let Some(s) = current["statuses"][id].as_str().and_then(status::parse) {
            host.statuses().seed(id, s);
        }
    }
    let (met, statuses) = host
        .statuses()
        .wait(&ids, until, Duration::from_secs(timeout), |p| host.alive(p));
    Ok(json!({ "met": met, "timedOut": !met, "statuses": statuses }))
}

/// Validated `spawn_agent` arguments, normalized for the UI.
fn spawn_args(args: &Value) -> Result<Value, String> {
    let agent: AgentKind = serde_json::from_value(args["agent"].clone())
        .map_err(|_| "agent must be claude, codex, gemini, grok or shell")?;
    let title = args["title"].as_str().map(str::trim).unwrap_or("");
    if title.is_empty() || title.chars().count() > 80 {
        return Err("title: 1 to 80 characters".into());
    }
    let cwd = match &args["cwd"] {
        Value::Null => None,
        Value::String(s) if !s.trim().is_empty() => Some(s.trim()),
        _ => return Err("cwd must be a path".into()),
    };
    let extra: Vec<&str> = match &args["args"] {
        Value::Null => Vec::new(),
        Value::Array(a) => a
            .iter()
            .map(|v| v.as_str().ok_or("args: strings expected"))
            .collect::<Result<_, _>>()?,
        _ => return Err("args must be a list of strings".into()),
    };
    if extra.len() > 20
        || extra
            .iter()
            .any(|a| a.len() > 500 || a.contains(char::is_control))
    {
        return Err("args: at most 20 arguments, no control characters".into());
    }
    Ok(json!({
        "agent": agent,
        "title": title,
        "cwd": cwd,
        "worktree": bool_arg(args, "worktree", false)?,
        "args": extra,
    }))
}

fn pane_arg(args: &Value) -> Result<&str, String> {
    args["paneId"]
        .as_str()
        .filter(|p| !p.is_empty() && p.len() <= 64)
        .ok_or_else(|| "missing paneId".into())
}

/// Optional non-empty string of at most `max` bytes.
fn opt_str<'a>(args: &'a Value, key: &str, max: usize) -> Result<Option<&'a str>, String> {
    match &args[key] {
        Value::Null => Ok(None),
        Value::String(s) if s.trim().is_empty() => Ok(None),
        Value::String(s) if s.len() <= max => Ok(Some(s.trim())),
        _ => Err(format!("{key}: text of at most {max} bytes")),
    }
}

fn bool_arg(args: &Value, key: &str, default: bool) -> Result<bool, String> {
    match &args[key] {
        Value::Null => Ok(default),
        Value::Bool(b) => Ok(*b),
        _ => Err(format!("{key} must be a boolean")),
    }
}

fn uint_arg(args: &Value, key: &str, default: u64, max: u64) -> Result<u64, String> {
    match &args[key] {
        Value::Null => Ok(default),
        v => v
            .as_u64()
            .filter(|n| (1..=max).contains(n))
            .ok_or_else(|| format!("{key}: integer between 1 and {max}")),
    }
}

/// Last `n` non-blank-trailing lines of terminal output, ANSI stripped.
pub fn last_lines(raw: &str, n: usize) -> String {
    let mut text = String::new();
    crate::history::ansi::Stripper::default().push(raw, &mut text);
    let lines: Vec<&str> = text.trim_end().lines().map(str::trim_end).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        paused: bool,
        calls: RefCell<Vec<(String, Value)>>,
        log: RefCell<Vec<(String, bool)>>,
        statuses: Statuses,
    }

    impl Host for Fake {
        fn frontend(&self, tool: &str, args: &Value) -> Result<Value, String> {
            self.calls.borrow_mut().push((tool.into(), args.clone()));
            match tool {
                "read_output" => Ok(json!({ "raw": "\x1b[31ma\x1b[0m\r\nb\r\nc\r\n" })),
                "wait_for" => Ok(json!({ "statuses": { "p1": "done", "p2": "working" } })),
                "get_status" if args["paneId"] == "other-ws" => Err("outside workspace".into()),
                _ => Ok(json!({ "ok": true })),
            }
        }
        fn paused(&self) -> bool {
            self.paused
        }
        fn alive(&self, _: &str) -> bool {
            true
        }
        fn statuses(&self) -> &Statuses {
            &self.statuses
        }
        fn activity(&self, tool: &str, _: &Value, error: Option<&str>) {
            self.log.borrow_mut().push((tool.into(), error.is_none()));
        }
    }

    fn req(method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params })
    }

    fn tool(host: &Fake, name: &str, args: Value) -> Value {
        handle(
            &req("tools/call", json!({ "name": name, "arguments": args })),
            host,
        )
        .unwrap()["result"]
            .clone()
    }

    #[test]
    fn initialize_negotiates_version() {
        let host = Fake::default();
        let r = handle(
            &req("initialize", json!({ "protocolVersion": "2025-03-26" })),
            &host,
        )
        .unwrap();
        assert_eq!(r["id"], 7);
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        let r = handle(
            &req("initialize", json!({ "protocolVersion": "1999-01-01" })),
            &host,
        )
        .unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    }

    #[test]
    fn notifications_and_unknown_methods() {
        let host = Fake::default();
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle(&note, &host).is_none());
        assert!(handle(&json!({ "jsonrpc": "2.0", "id": 1, "result": {} }), &host).is_none());
        assert_eq!(
            handle(&req("ping", json!({})), &host).unwrap()["result"],
            json!({})
        );
        let r = handle(&req("resources/list", json!({})), &host).unwrap();
        assert_eq!(r["error"]["code"], -32601);
        let r = handle(&req("tools/call", json!({ "name": "kill_pane" })), &host).unwrap();
        assert_eq!(r["error"]["code"], -32602);
    }

    #[test]
    fn lists_twelve_tools_with_schemas() {
        let host = Fake::default();
        let r = handle(&req("tools/list", json!({})), &host).unwrap();
        let tools = r["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 12);
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
    }

    #[test]
    fn validates_arguments() {
        let host = Fake::default();
        let err = |r: Value| r["isError"] == true;
        assert!(err(tool(
            &host,
            "spawn_agent",
            json!({ "agent": "vim", "title": "x" })
        )));
        assert!(err(tool(
            &host,
            "spawn_agent",
            json!({ "agent": "codex", "title": " " })
        )));
        assert!(err(tool(
            &host,
            "spawn_agent",
            json!({ "agent": "codex", "title": "t", "args": [1] })
        )));
        assert!(err(tool(
            &host,
            "send_prompt",
            json!({ "paneId": "p1", "text": "  " })
        )));
        assert!(err(tool(&host, "send_prompt", json!({ "text": "hi" }))));
        assert!(err(tool(
            &host,
            "read_output",
            json!({ "paneId": "p1", "lines": 500 })
        )));
        assert!(err(tool(
            &host,
            "wait_for",
            json!({ "paneIds": [], "until": "done" })
        )));
        assert!(err(tool(
            &host,
            "wait_for",
            json!({ "paneIds": ["p1"], "until": "soon" })
        )));
        assert!(err(tool(
            &host,
            "get_status",
            json!({ "paneId": "other-ws" })
        )));
        // Nothing invalid reached the UI.
        assert_eq!(host.calls.borrow().len(), 1);

        let ok = tool(
            &host,
            "spawn_agent",
            json!({ "agent": "codex", "title": " Tests ", "args": ["--model", "o3"] }),
        );
        assert!(ok.get("isError").is_none());
        let (_, sent) = host.calls.borrow().last().cloned().unwrap();
        assert_eq!(
            sent,
            json!({ "agent": "codex", "title": "Tests", "cwd": null, "worktree": false, "args": ["--model", "o3"] })
        );
        assert_eq!(host.log.borrow().len(), 10);
    }

    #[test]
    fn git_tools_validate_and_forward() {
        let host = Fake::default();
        let err = |r: Value| r["isError"] == true;
        assert!(err(tool(
            &host,
            "restore_checkpoint",
            json!({ "paneId": "p1", "sha": "HEAD~1; rm" })
        )));
        assert!(err(tool(
            &host,
            "get_diff",
            json!({ "paneId": "p1", "path": 3 })
        )));
        assert!(err(tool(&host, "close_agent", json!({}))));
        assert!(host.calls.borrow().is_empty());
        tool(
            &host,
            "merge_worktree",
            json!({ "paneId": "p1", "commitMessage": " feat: x " }),
        );
        tool(&host, "restore_checkpoint", json!({ "paneId": "p1" }));
        let calls = host.calls.borrow();
        assert_eq!(
            calls[0].1,
            json!({ "paneId": "p1", "commitMessage": "feat: x" })
        );
        assert_eq!(calls[1].1, json!({ "paneId": "p1", "sha": null }));
        let paused = Fake {
            paused: true,
            ..Fake::default()
        };
        assert!(err(tool(&paused, "close_agent", json!({ "paneId": "p1" }))));
        assert!(!err(tool(&paused, "get_diff", json!({ "paneId": "p1" }))));
    }

    #[test]
    fn pause_blocks_mutating_tools_only() {
        let host = Fake {
            paused: true,
            ..Fake::default()
        };
        let r = tool(
            &host,
            "send_prompt",
            json!({ "paneId": "p1", "text": "go" }),
        );
        assert_eq!(r["isError"], true);
        assert_eq!(r["content"][0]["text"], PAUSED_MSG);
        assert!(
            tool(&host, "list_agents", json!({}))
                .get("isError")
                .is_none()
        );
        assert!(host.calls.borrow().iter().all(|(t, _)| t == "list_agents"));
    }

    #[test]
    fn send_prompt_marks_working_and_wait_for_seeds() {
        let host = Fake::default();
        host.statuses.set("p2", PaneStatus::Done);
        tool(
            &host,
            "send_prompt",
            json!({ "paneId": "p2", "text": "go" }),
        );
        let r = tool(
            &host,
            "wait_for",
            json!({ "paneIds": ["p1", "p2"], "until": "any", "timeoutSec": 1 }),
        );
        let body: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["met"], true);
        assert_eq!(body["statuses"], json!({ "p1": "done", "p2": "working" }));
    }

    #[test]
    fn read_output_strips_and_tails() {
        let host = Fake::default();
        let r = tool(&host, "read_output", json!({ "paneId": "p1", "lines": 2 }));
        let body: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["output"], "b\nc");
        assert_eq!(last_lines("x\ny", 200), "x\ny");
    }
}
