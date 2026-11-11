//! User API: the `agentdeck` command line (loopback, token in the data folder) and the phone page
//! (LAN listener, off until turned on in the settings) drive the app through it. Pane actions run
//! in the UI (`user-request` event, answered with `mcp::mcp_respond`).
//!
//! The phone listener speaks plain HTTP: its token travels in clear on the local network, so it is
//! for a network you trust. The token stays out of the page's address sent to the server (it is in
//! the `#fragment`) and out of the page itself.

use std::net::{IpAddr, UdpSocket};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use serde_json::json;
use tiny_http::{Header, Method, Request, Response, ResponseBox, Server};

use crate::AppHandle;
use crate::hooks_server::permissions::PENDING;
use crate::hooks_server::{header, token_ok};
use crate::mcp::{ask_ui, json_reply, read_json};

/// Header carrying the token of both listeners.
pub const API_HEADER: &str = "X-AgentDeck-Api";
/// `{port, token}` of this run, in the data folder: how the command line finds the app.
pub const API_FILE: &str = "api.json";
/// What the phone may do: see the agents, type to one, answer a permission. Starting agents stays
/// on the computer.
const PHONE_OPS: [&str; 4] = ["list", "send", "permissions", "answer"];
const MIN_TOKEN: usize = 16;

static CLI_TOKEN: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());
static PHONE: Mutex<Option<Arc<Server>>> = Mutex::new(None);

/// Writes where the command line finds this run.
pub fn init(app: &AppHandle, port: u16) {
    let written = app.app_data_dir().and_then(|dir| {
        let text = json!({ "port": port, "token": *CLI_TOKEN }).to_string();
        std::fs::write(dir.join(API_FILE), text).map_err(|e| e.to_string())
    });
    if let Err(e) = written {
        eprintln!("command line disabled: {e}");
    }
}

/// One request on `/api` of the hook server (loopback): the command line.
pub fn serve(app: &AppHandle, mut req: Request) {
    // A browser always sends Origin, the command line never does: no web page may call this.
    let reply = if header(&req, "Origin").is_some() {
        Response::empty(403).boxed()
    } else if !token_ok(header(&req, API_HEADER).unwrap_or(""), &CLI_TOKEN) {
        Response::empty(401).boxed()
    } else {
        answer(app, &mut req, false)
    };
    let _ = req.respond(reply);
}

fn answer(app: &AppHandle, req: &mut Request, phone: bool) -> ResponseBox {
    let body = match read_json(req) {
        Ok(body) => body,
        Err(code) => return Response::empty(code).boxed(),
    };
    let op = body["op"].as_str().unwrap_or("");
    let result = match op {
        _ if phone && !PHONE_OPS.contains(&op) => Err("not available from the phone".to_owned()),
        "permissions" => Ok(json!(PENDING.list())),
        // Allow or deny only: "always allow" changes the rules, that stays on the computer.
        "answer" => match body["decision"].as_str() {
            Some(decision @ ("allow" | "deny")) => PENDING
                .answer(body["requestId"].as_str().unwrap_or(""), decision, None)
                .map(|()| json!({ "ok": true })),
            _ => Err("decision: allow or deny".to_owned()),
        },
        _ => ask_ui(app, "user-request", json!({ "op": op, "args": body })),
    };
    json_reply(
        200,
        &match result {
            Ok(value) => json!({ "result": value }),
            Err(e) => json!({ "error": e }),
        },
    )
}

/// Starts, moves or stops the phone listener; `None` stops it.
pub fn set_phone(app: &AppHandle, config: Option<(u16, String)>) -> Result<(), String> {
    let mut slot = PHONE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(server) = slot.take() {
        server.unblock(); // its thread ends and frees the port
    }
    let Some((port, token)) = config else { return Ok(()) };
    if token.len() < MIN_TOKEN {
        return Err("phone access: token too short".into());
    }
    let server = Server::http(("0.0.0.0", port)).map_err(|e| format!("phone access: port {port}: {e}"))?;
    let server = Arc::new(server);
    *slot = Some(server.clone());
    let app = app.clone();
    std::thread::spawn(move || {
        for mut req in server.incoming_requests() {
            // One thread per request, like the hook server: a slow phone must not stall the others.
            let (app, token) = (app.clone(), token.clone());
            std::thread::spawn(move || {
                let reply = if *req.method() == Method::Get {
                    page()
                } else if !token_ok(header(&req, API_HEADER).unwrap_or(""), &token) {
                    Response::empty(401).boxed()
                } else {
                    answer(&app, &mut req, true)
                };
                let _ = req.respond(reply);
            });
        }
    });
    Ok(())
}

fn page() -> ResponseBox {
    let html = Header::from_bytes("Content-Type", "text/html; charset=utf-8").expect("static header");
    let no_store = Header::from_bytes("Cache-Control", "no-store").expect("static header");
    Response::from_string(PAGE).with_header(html).with_header(no_store).boxed()
}

/// This computer's address on the local network (no packet is sent: the socket is only routed).
pub fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    socket.local_addr().ok().map(|addr| addr.ip()).filter(|ip| !ip.is_unspecified())
}

/// The phone page: agents and their status, a prompt box per agent, allow/deny for permissions.
/// Text from the app is written with `textContent`, never as HTML.
const PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>AgentDeck</title>
<style>
  :root { color-scheme: dark; --bg: #0e0f11; --panel: #17191c; --line: #2a2d32; --fg: #e8e9eb; --muted: #9a9ea6; }
  * { box-sizing: border-box; }
  body { margin: 0; padding: 16px; background: var(--bg); color: var(--fg); font: 15px/1.4 system-ui, sans-serif; }
  h1 { font-size: 17px; margin: 0 0 12px; }
  h2 { font-size: 12px; margin: 20px 0 8px; color: var(--muted); text-transform: uppercase; letter-spacing: .05em; }
  .card { background: var(--panel); border: 1px solid var(--line); border-radius: 12px; padding: 12px; margin-bottom: 8px; }
  .row { display: flex; align-items: center; gap: 8px; }
  .title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .pill { font-size: 12px; padding: 1px 8px; border-radius: 99px; }
  .working { background: #1d3a5f; color: #7db7ff; } .waiting { background: #4a3a12; color: #f5c451; }
  .done { background: #173d27; color: #5fd08a; } .error { background: #4a1c1c; color: #ff8080; } .idle { color: var(--muted); }
  .sub { color: var(--muted); font-size: 13px; margin-top: 4px; overflow-wrap: anywhere; }
  textarea { width: 100%; margin-top: 8px; min-height: 64px; background: var(--bg); color: var(--fg); border: 1px solid var(--line); border-radius: 8px; padding: 8px; font: inherit; }
  button { font: inherit; padding: 8px 14px; border-radius: 8px; border: 1px solid var(--line); background: var(--panel); color: var(--fg); }
  button.ok { background: #1f6f43; border-color: #1f6f43; } button.no { background: #7a2a2a; border-color: #7a2a2a; }
  #error { color: #ff8080; }
</style>
</head>
<body>
<h1>AgentDeck</h1>
<div id="error"></div>
<div id="permissions"></div>
<h2>Agents</h2>
<div id="agents"></div>
<script>
const token = location.hash.slice(1);
let open = null, draft = "";
const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };
async function api(body) {
  const res = await fetch("/api", { method: "POST", headers: { "X-AgentDeck-Api": token, "Content-Type": "application/json" }, body: JSON.stringify(body) });
  if (!res.ok) throw new Error(res.status === 401 ? "Wrong or missing link token" : "HTTP " + res.status);
  const json = await res.json();
  if (json.error) throw new Error(json.error);
  return json.result;
}
function fail(e) { document.getElementById("error").textContent = e.message; }
async function refresh() {
  try {
    const [agents, permissions] = await Promise.all([api({ op: "list" }), api({ op: "permissions" })]);
    document.getElementById("error").textContent = "";
    const names = Object.fromEntries(agents.map(a => [a.paneId, a.title]));
    const box = document.getElementById("permissions");
    box.replaceChildren();
    if (permissions.length) box.append(el("h2", "", "Waiting for you"));
    for (const p of permissions) {
      const card = el("div", "card");
      card.append(el("div", "title", names[p.paneId] || "Agent"), el("div", "sub", p.summary));
      const row = el("div", "row");
      row.style.marginTop = "8px";
      if (p.question) row.append(el("div", "sub", "A question: answer it on the computer."));
      else for (const [label, decision, cls] of [["Allow", "allow", "ok"], ["Deny", "deny", "no"]]) {
        const b = el("button", cls, label);
        b.onclick = () => api({ op: "answer", requestId: p.requestId, decision }).then(refresh, fail);
        row.append(b);
      }
      card.append(row);
      box.append(card);
    }
    if (document.activeElement && document.activeElement.tagName === "TEXTAREA") return; // do not wipe what is being typed
    const list = document.getElementById("agents");
    list.replaceChildren();
    for (const a of agents) {
      const card = el("div", "card");
      const row = el("div", "row");
      row.append(el("div", "title", a.title), el("span", "pill " + a.status, a.status));
      card.append(row, el("div", "sub", [a.workspace, a.lastAction].filter(Boolean).join(" · ")));
      row.onclick = () => { open = open === a.paneId ? null : a.paneId; draft = ""; refresh(); };
      if (open === a.paneId) {
        const text = el("textarea");
        text.placeholder = "Prompt for " + a.title;
        text.value = draft;
        text.oninput = () => { draft = text.value; };
        const send = el("button", "ok", "Send");
        send.style.marginTop = "8px";
        send.onclick = () => api({ op: "send", pane: a.paneId, text: text.value }).then(() => { open = null; draft = ""; text.blur(); refresh(); }, fail);
        card.append(text, send);
      }
      list.append(card);
    }
  } catch (e) { fail(e); }
}
refresh();
setInterval(refresh, 3000);
</script>
</body>
</html>
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_cannot_start_agents_and_page_holds_no_token() {
        assert!(!PHONE_OPS.contains(&"spawn"));
        assert!(!PAGE.contains(CLI_TOKEN.as_str()));
        // The page never writes app text as HTML.
        assert!(!PAGE.contains("innerHTML"));
    }

    #[test]
    fn phone_listener_requires_the_token() {
        let app = AppHandle::new(std::env::temp_dir());
        let (port, token) = (47931, "0123456789abcdef-test".to_owned());
        set_phone(&app, Some((port, token.clone()))).unwrap();
        let client = reqwest::blocking::Client::new();
        let post = |token: &str, body: &str| {
            let sent = client.post(format!("http://127.0.0.1:{port}/api")).header(API_HEADER, token).body(body.to_owned());
            sent.send().unwrap()
        };
        let json = |token: &str, body: &str| -> serde_json::Value { serde_json::from_str(&post(token, body).text().unwrap()).unwrap() };
        let page = client.get(format!("http://127.0.0.1:{port}/")).send().unwrap().text().unwrap();
        assert!(page.contains("<title>AgentDeck</title>") && !page.contains(&token));
        assert_eq!(post("wrong", "{}").status(), 401);
        assert!(json(&token, r#"{"op":"permissions"}"#)["result"].is_array());
        assert_eq!(json(&token, r#"{"op":"spawn"}"#)["error"], "not available from the phone");
        assert_eq!(json(&token, r#"{"op":"answer","requestId":"x","decision":"allow_always"}"#)["error"], "decision: allow or deny");
        set_phone(&app, None).unwrap();
        assert!(set_phone(&app, Some((port, "short".into()))).is_err());
    }
}
