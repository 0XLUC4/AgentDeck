//! Shared MCP hub: AgentDeck runs ONE instance of each configured MCP server and every agent pane
//! reaches it through `/hub/<name>` on the hook server. Servers that tolerate a single client (Roblox
//! Studio: its plugin bridge is one fixed local port, newest process wins) are no longer fought over.
//!
//! Agents see the same server names as in their own config: Claude's `--mcp-config` replaces a
//! same-named global server and Codex's `-c mcp_servers.<name>` overrides it, so they start no process
//! of their own. Grok has no CLI override: its scan of `~/.claude.json` is turned off in its panes.
//!
//! The hub answers `initialize` from the upstream's cached handshake, forwards requests under its own
//! ids (one session shared by all panes) and runs `tools/call` one at a time.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tiny_http::{Request, Response};

use super::{json_reply, lock, read_json, rpc};
use crate::agents::AgentKind;
use crate::hooks_server::{PANE_HEADER, TOKEN_HEADER, header, token_ok};

const PROTOCOL: &str = "2025-06-18";
const INIT_TIMEOUT: Duration = Duration::from_secs(30);
/// Roblox playtests and asset generation are slow: be generous before giving up on a call.
const CALL_TIMEOUT: Duration = Duration::from_secs(600);
/// Last bytes of a server's stderr kept to explain a failed start.
const STDERR_TAIL: usize = 2048;

/// One shared server: `command` (stdio, local) or `url` (streamable HTTP), as in Claude's config.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HubServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub url: String,
    pub headers: HashMap<String, String>,
}

static SERVERS: LazyLock<Mutex<Vec<HubServer>>> = LazyLock::new(Default::default);
static RUNNING: LazyLock<Mutex<HashMap<String, Arc<Upstream>>>> = LazyLock::new(Default::default);
/// `<app data>/hub` and the hook server port, set once by `init`.
static DIR: OnceLock<(PathBuf, u16)> = OnceLock::new();

/// Loads the saved servers and rewrites the Claude config for this run's port.
pub fn init(app: &AppHandle, port: u16) {
    let Ok(dir) = app.path().app_data_dir().map(|d| d.join("hub")) else {
        return;
    };
    let saved = std::fs::read_to_string(dir.join("servers.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<HubServer>>(&text).ok())
        .unwrap_or_default();
    let _ = DIR.set((dir, port));
    *lock(&SERVERS) = saved;
    if let Err(e) = write_claude_config() {
        eprintln!("mcp hub: {e}");
    }
}

/// Stops every server (app exit): they would otherwise outlive AgentDeck and keep their ports.
pub fn stop_all() {
    lock(&RUNNING).clear();
}

/// CLI args that point `kind` at the hub instead of starting its own servers.
pub fn agent_args(kind: AgentKind) -> Vec<String> {
    let servers = lock(&SERVERS);
    if servers.is_empty() {
        return Vec::new();
    }
    match kind {
        AgentKind::Claude => DIR
            .get()
            .map(|(dir, _)| {
                vec![
                    "--mcp-config".into(),
                    dir.join("claude.json").to_string_lossy().into_owned(),
                ]
            })
            .unwrap_or_default(),
        // TOML literal strings and no spaces survive cmd.exe + the npm .cmd shim (see `hook_args`).
        AgentKind::Codex => servers
            .iter()
            .flat_map(|s| {
                [
                    "-c".to_owned(),
                    format!(
                        "mcp_servers.{}={{url='{}',env_http_headers={{'{TOKEN_HEADER}'='AGENTDECK_HOOK_TOKEN','{PANE_HEADER}'='AGENTDECK_PANE_ID'}}}}",
                        s.name,
                        hub_url(&s.name)
                    ),
                ]
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Extra env for `kind` while the hub is on.
pub fn agent_env(kind: AgentKind) -> Option<(&'static str, &'static str)> {
    // shortcut: Grok also loses its other Claude-imported servers; upgrade once Grok has a CLI override.
    (kind == AgentKind::Grok && !lock(&SERVERS).is_empty())
        .then_some(("GROK_CLAUDE_MCPS_ENABLED", "false"))
}

fn hub_url(name: &str) -> String {
    let port = DIR.get().map_or(0, |(_, p)| *p);
    format!("http://127.0.0.1:{port}/hub/{name}")
}

/// Token and pane id are expanded by Claude Code from the pane's env: the file holds no secret.
fn claude_config(servers: &[HubServer]) -> Value {
    let entries: serde_json::Map<String, Value> = servers
        .iter()
        .map(|s| {
            let entry = json!({
                "type": "http",
                "url": hub_url(&s.name),
                "headers": {
                    TOKEN_HEADER: "${AGENTDECK_HOOK_TOKEN}",
                    PANE_HEADER: "${AGENTDECK_PANE_ID}",
                },
            });
            (s.name.clone(), entry)
        })
        .collect();
    json!({ "mcpServers": entries })
}

fn write_file(name: &str, text: &str) -> Result<(), String> {
    let (dir, _) = DIR.get().ok_or("hub not initialized")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(name);
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_claude_config() -> Result<(), String> {
    let config = claude_config(&lock(&SERVERS));
    write_file(
        "claude.json",
        &serde_json::to_string_pretty(&config).unwrap_or_default(),
    )
}

fn validate(servers: &[HubServer]) -> Result<(), String> {
    for (i, s) in servers.iter().enumerate() {
        let name_ok = !s.name.is_empty()
            && s.name.len() <= 64
            && s.name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !name_ok {
            return Err(format!(
                "invalid name \"{}\" (letters, digits, - and _ only)",
                s.name
            ));
        }
        if servers[..i].iter().any(|o| o.name == s.name) {
            return Err(format!("duplicate name \"{}\"", s.name));
        }
        match (s.command.trim().is_empty(), s.url.trim().is_empty()) {
            (false, true) => {}
            (true, false) if s.url.starts_with("http://") || s.url.starts_with("https://") => {}
            (true, false) => return Err(format!("{}: the URL must start with http(s)://", s.name)),
            _ => return Err(format!("{}: give either a command or a URL", s.name)),
        }
    }
    Ok(())
}

#[tauri::command]
pub fn hub_list() -> Vec<HubServer> {
    lock(&SERVERS).clone()
}

/// Saves the servers; a changed or removed server is stopped (restarted on next use).
#[tauri::command]
pub fn hub_save(servers: Vec<HubServer>) -> Result<(), String> {
    validate(&servers)?;
    write_file(
        "servers.json",
        &serde_json::to_string_pretty(&servers).unwrap_or_default(),
    )?;
    let stale: Vec<String> = lock(&SERVERS)
        .iter()
        .filter(|old| !servers.contains(old))
        .map(|old| old.name.clone())
        .collect();
    lock(&RUNNING).retain(|name, _| !stale.contains(name));
    *lock(&SERVERS) = servers;
    write_claude_config()
}

/// Starts `name` if needed and returns its tool count: "does it work?" from Settings.
#[tauri::command(async)]
pub fn hub_test(name: String) -> Result<usize, String> {
    let reply = upstream(&name)?.request("tools/list", None, INIT_TIMEOUT)?;
    if let Some(e) = reply.get("error") {
        return Err(e["message"]
            .as_str()
            .unwrap_or("tools/list failed")
            .to_owned());
    }
    Ok(reply["result"]["tools"].as_array().map_or(0, Vec::len))
}

/// User-scope servers from `~/.claude.json`, to import. claude.ai connectors are not in it: their
/// OAuth stays inside Claude, so they cannot be shared anyway.
#[tauri::command]
pub fn hub_claude_servers() -> Vec<HubServer> {
    let config = crate::hooks_server::home_dir()
        .and_then(|home| std::fs::read_to_string(home.join(".claude.json")).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or_default();
    let Some(servers) = config["mcpServers"].as_object() else {
        return Vec::new();
    };
    servers
        .iter()
        .filter(|(_, v)| v["type"] != "sse") // legacy SSE transport: not supported by the hub
        .filter_map(|(name, v)| {
            let mut server: HubServer = serde_json::from_value(v.clone()).ok()?;
            server.name = name.clone();
            Some(server)
        })
        .collect()
}

/// Answers one HTTP request on `/hub/<name>` from an agent pane.
pub fn serve(app: &AppHandle, mut req: Request, token: &str, name: &str) {
    let reply = match authorize(app, &mut req, token) {
        Err(code) => Response::empty(code).boxed(),
        Ok(msg) => match handle(name, &msg) {
            Some(body) => json_reply(200, &body),
            None => Response::empty(202).boxed(),
        },
    };
    let _ = req.respond(reply);
}

/// Hook token + a live pane id: what every AgentDeck pane (and only them) has.
fn authorize(app: &AppHandle, req: &mut Request, token: &str) -> Result<Value, u16> {
    if *req.method() != tiny_http::Method::Post {
        return Err(405); // no server stream (GET) nor session to delete (DELETE)
    }
    if header(req, "Origin").is_some() {
        return Err(403); // DNS rebinding: browsers send Origin, CLIs don't
    }
    if !token_ok(header(req, TOKEN_HEADER).unwrap_or(""), token) {
        return Err(401);
    }
    let pane = header(req, PANE_HEADER).unwrap_or("");
    let alive = app
        .try_state::<crate::AppState>()
        .is_some_and(|s| s.panes.0.lock().is_ok_and(|p| p.contains_key(pane)));
    if !alive {
        return Err(403);
    }
    let msg = read_json(req)?;
    if !msg.is_object() {
        return Err(400); // batches were removed from the 2025-06-18 spec
    }
    Ok(msg)
}

/// `None`: nothing to answer (notification, or a response from the agent).
fn handle(name: &str, msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let method = msg["method"].as_str()?;
    let reply = match method {
        "ping" => Ok(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
        _ => upstream(name).and_then(|up| {
            if method == "initialize" {
                return Ok(json!({ "jsonrpc": "2.0", "id": id, "result": up.init }));
            }
            // Two agents editing the same place at once is how Studio state gets mixed up.
            let _turn = (method == "tools/call").then(|| lock(&up.calls));
            up.request(method, msg.get("params"), CALL_TIMEOUT)
                .map(|mut reply| {
                    reply["id"] = id.clone();
                    reply
                })
        }),
    };
    Some(reply.unwrap_or_else(|e| rpc::error(id, -32603, &e)))
}

fn upstream(name: &str) -> Result<Arc<Upstream>, String> {
    // shortcut: a slow start blocks the other servers' lookups (30 s max); upgrade to per-name locks if it bites.
    let mut running = lock(&RUNNING);
    if let Some(up) = running
        .get(name)
        .filter(|up| up.alive.load(Ordering::Relaxed))
    {
        return Ok(up.clone());
    }
    let server = lock(&SERVERS)
        .iter()
        .find(|s| s.name == name)
        .cloned()
        .ok_or_else(|| format!("unknown hub server \"{name}\""))?;
    let up = Arc::new(Upstream::start(&server)?);
    running.insert(name.to_owned(), up.clone());
    Ok(up)
}

struct Upstream {
    link: Link,
    alive: Arc<AtomicBool>,
    next: AtomicU64,
    /// The upstream's `initialize` result, replayed to every agent.
    init: Value,
    calls: Mutex<()>,
}

enum Link {
    Pipe(Pipe),
    Remote(Remote),
}

impl Upstream {
    fn start(server: &HubServer) -> Result<Self, String> {
        let alive = Arc::new(AtomicBool::new(true));
        let link = if server.url.is_empty() {
            Link::Pipe(Pipe::spawn(server, alive.clone())?)
        } else {
            Link::Remote(Remote::new(server)?)
        };
        let mut up = Self {
            link,
            alive,
            next: AtomicU64::new(1),
            init: Value::Null,
            calls: Mutex::new(()),
        };
        let params = json!({
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": { "name": "agentdeck-hub", "version": env!("CARGO_PKG_VERSION") },
        });
        let reply = up
            .request("initialize", Some(&params), INIT_TIMEOUT)
            .map_err(|e| up.explain(e))?;
        if let Some(e) = reply.get("error") {
            return Err(format!("initialize refused: {e}"));
        }
        up.init = reply["result"].clone();
        if let Link::Remote(remote) = &up.link {
            *lock(&remote.version) = up.init["protocolVersion"].as_str().map(str::to_owned);
        }
        up.notify("notifications/initialized")?;
        Ok(up)
    }

    /// Sends a request under a fresh hub id; returns the whole response (`result` or `error`).
    fn request(
        &self,
        method: &str,
        params: Option<&Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        if !self.alive.load(Ordering::Relaxed) {
            return Err("the MCP server stopped; retry to restart it".into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut msg = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if let Some(params) = params {
            msg["params"] = params.clone();
        }
        let reply = match &self.link {
            Link::Pipe(pipe) => pipe.request(id, &msg, timeout),
            Link::Remote(remote) => remote
                .post(&msg)
                .and_then(|r| r.ok_or_else(|| "no response".into())),
        };
        if reply.is_err() && matches!(self.link, Link::Remote(_)) {
            self.alive.store(false, Ordering::Relaxed); // new session on next use
        }
        reply
    }

    fn notify(&self, method: &str) -> Result<(), String> {
        let msg = json!({ "jsonrpc": "2.0", "method": method });
        match &self.link {
            Link::Pipe(pipe) => write_line(&pipe.stdin, &msg),
            Link::Remote(remote) => remote.post(&msg).map(drop),
        }
    }

    fn explain(&self, error: String) -> String {
        let Link::Pipe(pipe) = &self.link else {
            return error;
        };
        match lock(&pipe.stderr).trim() {
            "" => error,
            tail => format!("{error}\n{tail}"),
        }
    }
}

/// A local server on stdin/stdout, one JSON-RPC message per line.
struct Pipe {
    child: Mutex<Child>,
    stdin: Arc<Mutex<ChildStdin>>,
    waiting: Arc<Mutex<HashMap<u64, Sender<Value>>>>,
    stderr: Arc<Mutex<String>>,
}

impl Pipe {
    fn spawn(server: &HubServer, alive: Arc<AtomicBool>) -> Result<Self, String> {
        let mut child = command(server)
            .envs(&server.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", server.command))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err("no stdio pipes".into());
        };
        let pipe = Self {
            child: Mutex::new(child),
            stdin: Arc::new(Mutex::new(stdin)),
            waiting: Arc::default(),
            stderr: Arc::default(),
        };
        let tail = pipe.stderr.clone();
        std::thread::spawn(move || keep_tail(stderr, &tail));
        let (stdin, waiting) = (pipe.stdin.clone(), pipe.waiting.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                // Some servers log on stdout: anything that is not JSON-RPC is skipped.
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                match (msg.get("id"), msg.get("method")) {
                    (Some(id), None) => {
                        if let Some(tx) = id.as_u64().and_then(|id| lock(&waiting).remove(&id)) {
                            let _ = tx.send(msg);
                        }
                    }
                    // Server-to-client requests (sampling, roots, elicitation) have no agent to go to.
                    (Some(id), Some(_)) => {
                        let reply =
                            rpc::error(id.clone(), -32601, "not supported by the AgentDeck hub");
                        let _ = write_line(&stdin, &reply);
                    }
                    _ => {} // notifications (progress, logs, list_changed) are dropped
                }
            }
            alive.store(false, Ordering::Relaxed);
            lock(&waiting).clear(); // wakes pending requests with an error
        });
        Ok(pipe)
    }

    fn request(&self, id: u64, msg: &Value, timeout: Duration) -> Result<Value, String> {
        let (tx, rx) = channel();
        lock(&self.waiting).insert(id, tx);
        let reply = write_line(&self.stdin, msg).and_then(|()| {
            rx.recv_timeout(timeout).map_err(|e| match e {
                RecvTimeoutError::Timeout => "the MCP server did not answer in time".to_owned(),
                RecvTimeoutError::Disconnected => "the MCP server stopped".to_owned(),
            })
        });
        lock(&self.waiting).remove(&id);
        reply
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        let child = self
            .child
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The server runs under cmd.exe on Windows: kill the whole tree, not just the shell.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let _ = Command::new("taskkill")
                .args(["/T", "/F", "/PID", &child.id().to_string()])
                .creation_flags(CREATE_NO_WINDOW)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Windows: through `cmd /c` so `npx`/`.bat` resolve and `%VAR%` / `&&` work as in Claude's config.
fn command(server: &HubServer) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let join = |args: &mut dyn Iterator<Item = &String>| {
            args.map(|a| quote(a)).collect::<Vec<_>>().join(" ")
        };
        let mut cmd = Command::new("cmd.exe");
        // Already a cmd line (Roblox Studio: `cmd /c cd /d ... && mcp.bat`): a second shell would split its `&&`.
        if matches!(
            server.command.to_ascii_lowercase().as_str(),
            "cmd" | "cmd.exe"
        ) {
            cmd.raw_arg(join(&mut server.args.iter()));
        } else {
            let line = join(&mut std::iter::once(&server.command).chain(&server.args));
            cmd.raw_arg(format!("/d /s /c \"{line}\""));
        }
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new(&server.command);
        cmd.args(&server.args);
        cmd
    }
}

#[cfg(any(windows, test))]
fn quote(arg: &str) -> String {
    if arg.is_empty() || arg.contains([' ', '\t', '"']) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_owned()
    }
}

fn keep_tail(mut stderr: impl Read, tail: &Mutex<String>) {
    let mut buf = [0u8; 1024];
    while let Ok(n @ 1..) = stderr.read(&mut buf) {
        let mut text = lock(tail);
        text.push_str(&String::from_utf8_lossy(&buf[..n]));
        if text.len() > STDERR_TAIL {
            let cut = text.len() - STDERR_TAIL;
            let cut = (cut..text.len())
                .find(|&i| text.is_char_boundary(i))
                .unwrap_or(0);
            text.drain(..cut);
        }
    }
}

fn write_line(stdin: &Mutex<ChildStdin>, msg: &Value) -> Result<(), String> {
    let mut stdin = lock(stdin);
    writeln!(stdin, "{msg}")
        .and_then(|()| stdin.flush())
        .map_err(|_| "the MCP server stopped".into())
}

/// A remote server over streamable HTTP; the hub holds the one session.
struct Remote {
    client: reqwest::blocking::Client,
    url: String,
    headers: HashMap<String, String>,
    session: Mutex<Option<String>>,
    version: Mutex<Option<String>>,
}

impl Remote {
    fn new(server: &HubServer) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(CALL_TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            url: server.url.clone(),
            headers: server.headers.clone(),
            session: Mutex::default(),
            version: Mutex::default(),
        })
    }

    /// `Ok(None)`: accepted without a body (notification).
    fn post(&self, msg: &Value) -> Result<Option<Value>, String> {
        let mut req = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .json(msg);
        for (key, value) in &self.headers {
            req = req.header(key, value);
        }
        if let Some(session) = lock(&self.session).clone() {
            req = req.header("Mcp-Session-Id", session);
        }
        if let Some(version) = lock(&self.version).clone() {
            req = req.header("MCP-Protocol-Version", version);
        }
        let resp = req.send().map_err(|e| e.to_string())?;
        if let Some(session) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            *lock(&self.session) = Some(session.to_owned());
        }
        let status = resp.status();
        if status.as_u16() == 202 {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(format!("{} answered HTTP {status}", self.url));
        }
        let sse = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        let text = resp.text().map_err(|e| e.to_string())?;
        if sse {
            return Ok(sse_reply(&text, &msg["id"]));
        }
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| e.to_string())
    }
}

/// The response to `id` among the events of an SSE body.
fn sse_reply(body: &str, id: &Value) -> Option<Value> {
    body.replace("\r\n", "\n").split("\n\n").find_map(|event| {
        let data: Vec<&str> = event
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .map(str::trim_start)
            .collect();
        let msg: Value = serde_json::from_str(&data.join("\n")).ok()?;
        (msg.get("id") == Some(id) && msg.get("method").is_none()).then_some(msg)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio(name: &str) -> HubServer {
        HubServer {
            name: name.into(),
            command: "node".into(),
            ..Default::default()
        }
    }

    #[test]
    fn validates_names_and_transport() {
        assert!(validate(&[stdio("roblox-studio-plus"), stdio("Roblox_Studio")]).is_ok());
        assert!(validate(&[stdio("a b")]).is_err());
        assert!(validate(&[stdio("x"), stdio("x")]).is_err());
        let both = HubServer {
            url: "http://x".into(),
            ..stdio("x")
        };
        assert!(validate(&[both]).is_err());
        let ftp = HubServer {
            name: "x".into(),
            url: "ftp://x".into(),
            ..Default::default()
        };
        assert!(validate(&[ftp]).is_err());
        let empty = HubServer {
            name: "x".into(),
            ..Default::default()
        };
        assert!(validate(&[empty]).is_err());
    }

    #[test]
    fn claude_config_has_no_secret() {
        let c = claude_config(&[stdio("roblox-studio-plus")]);
        let entry = &c["mcpServers"]["roblox-studio-plus"];
        assert_eq!(entry["type"], "http");
        assert!(
            entry["url"]
                .as_str()
                .unwrap()
                .ends_with("/hub/roblox-studio-plus")
        );
        assert_eq!(entry["headers"][TOKEN_HEADER], "${AGENTDECK_HOOK_TOKEN}");
        assert_eq!(entry["headers"][PANE_HEADER], "${AGENTDECK_PANE_ID}");
    }

    #[test]
    fn finds_sse_response() {
        let body = "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\r\n\r\n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n";
        assert_eq!(sse_reply(body, &json!(7)).unwrap()["result"]["ok"], true);
        assert!(sse_reply(body, &json!(8)).is_none());
    }

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(quote("&&"), "&&");
        assert_eq!(quote("%LOCALAPPDATA%\\Roblox"), "%LOCALAPPDATA%\\Roblox");
        assert_eq!(quote("C:\\Program Files\\x"), "\"C:\\Program Files\\x\"");
        assert_eq!(quote(""), "\"\"");
    }

    #[test]
    fn handle_skips_notifications_and_answers_ping() {
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle("x", &note).is_none());
        assert!(handle("x", &json!({ "jsonrpc": "2.0", "id": 1, "result": {} })).is_none());
        let ping = json!({ "jsonrpc": "2.0", "id": 3, "method": "ping" });
        assert_eq!(handle("x", &ping).unwrap()["id"], 3);
        let unknown = json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/list" });
        assert_eq!(handle("nope", &unknown).unwrap()["error"]["code"], -32603);
    }

    fn short_path() -> String {
        std::env::var("PATH")
            .unwrap_or_default()
            .split(';')
            .filter(|dir| !dir.contains("target"))
            .collect::<Vec<_>>()
            .join(";")
    }

    /// End to end against a tiny stdio echo server (node is a dev requirement of the app).
    #[test]
    fn stdio_roundtrip() {
        let script = r#"require('readline').createInterface({input:process.stdin}).on('line',l=>{const m=JSON.parse(l);if(m.id===undefined)return;const result=m.method==='initialize'?{protocolVersion:'2025-06-18',capabilities:{tools:{}},serverInfo:{name:'t',version:'1'}}:{tools:[{name:'a'},{name:'b'}]};console.log('log line');console.log(JSON.stringify({jsonrpc:'2.0',id:m.id,result}))})"#;
        let server = HubServer {
            name: "echo".into(),
            command: "node".into(),
            args: vec!["-e".into(), script.into()],
            // cargo test's PATH (target dirs) exceeds cmd.exe's 8191-char limit: keep the rest.
            env: HashMap::from([("PATH".into(), short_path())]),
            ..Default::default()
        };
        let up = Upstream::start(&server).unwrap();
        assert_eq!(up.init["serverInfo"]["name"], "t");
        let reply = up.request("tools/list", None, INIT_TIMEOUT).unwrap();
        assert_eq!(reply["result"]["tools"].as_array().unwrap().len(), 2);
    }
}
