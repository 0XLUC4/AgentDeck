//! `agentdeck list | send | spawn`: drives the running app from a terminal or a script, through
//! the local API (`agentdeck_core::remote`). Runs before the window: the process answers and exits.

use std::time::Duration;

use agentdeck_core::remote::{API_FILE, API_HEADER};
use serde_json::{Value, json};

const USAGE: &str = "\
Usage:
  agentdeck list                               the agents and their status
  agentdeck send <agent id or title> <prompt>  types a prompt to an agent
  agentdeck spawn <agent> [--cwd <folder>] [prompt]
                                               starts claude, codex, gemini, grok or opencode
AgentDeck must be running.";

/// The exit code when the arguments are a command; None: start the app as usual.
pub fn run() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let request = match args.first().map(String::as_str)? {
        "list" => Ok(json!({ "op": "list" })),
        "send" if args.len() >= 3 => Ok(json!({ "op": "send", "pane": args[1], "text": args[2..].join(" ") })),
        "spawn" if args.len() >= 2 => Ok(spawn_request(&args[1..])),
        "help" | "--help" | "-h" | "send" | "spawn" => Err(USAGE.to_owned()),
        _ => return None, // --after-update and the like
    };
    let outcome = request.and_then(|body| call(&body));
    let (text, code) = match outcome {
        Ok(result) => (render(&result), 0),
        Err(e) => (e, 1),
    };
    print(&text);
    Some(code)
}

fn spawn_request(args: &[String]) -> Value {
    let (cwd, rest) = match args.get(1).map(String::as_str) {
        Some("--cwd") => (args.get(2).cloned(), args.get(3..).unwrap_or_default()),
        _ => (None, &args[1..]),
    };
    json!({ "op": "spawn", "agent": args[0], "cwd": cwd, "text": rest.join(" ") })
}

fn call(body: &Value) -> Result<Value, String> {
    let dir = match std::env::var_os("AGENTDECK_DATA_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => agentdeck_core::AppHandle::default_data_dir()?,
    };
    let not_running = "AgentDeck is not running";
    let api: Value = std::fs::read_to_string(dir.join(API_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or(not_running)?;
    let (port, token) = (api["port"].as_u64().unwrap_or(0), api["token"].as_str().unwrap_or(""));
    let answer: Value = reqwest::blocking::Client::new()
        .post(format!("http://127.0.0.1:{port}/api"))
        .header(API_HEADER, token)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .timeout(Duration::from_secs(40))
        .send()
        .map_err(|_| not_running.to_owned())?
        .error_for_status()
        .map_err(|e| format!("AgentDeck refused the command: {e}"))?
        .text()
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or("unreadable answer from AgentDeck")?;
    match answer["error"].as_str() {
        Some(e) => Err(e.to_owned()),
        None => Ok(answer["result"].clone()),
    }
}

/// The agent list as one line each; anything else as JSON.
fn render(result: &Value) -> String {
    let Some(rows) = result.as_array() else { return result.to_string() };
    let text = |row: &Value, key: &str| row[key].as_str().unwrap_or("").to_owned();
    rows.iter()
        .map(|r| {
            format!(
                "{}  {:<8} {:<8} {}  [{} / {}]",
                text(r, "paneId"),
                text(r, "agent"),
                text(r, "status"),
                text(r, "title"),
                text(r, "workspace"),
                text(r, "session")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The release build has no console of its own: write to the terminal that started it.
// shortcut: output goes to that console even when redirected (`agentdeck list > file` stays
// empty); switch to a console-subsystem launcher if scripts need pipes.
fn print(text: &str) {
    #[cfg(windows)]
    {
        use std::io::Write;
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        // SAFETY: plain Win32 call, no pointers.
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
        if let Ok(mut console) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
            let _ = writeln!(console, "\n{text}");
            return;
        }
    }
    println!("{text}");
}

#[cfg(test)]
mod tests {
    use super::{render, spawn_request};
    use serde_json::json;

    #[test]
    fn spawn_arguments_and_list_rows() {
        let args = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let with_cwd = spawn_request(&args(&["claude", "--cwd", "D:/proj", "fix", "the", "bug"]));
        assert_eq!(with_cwd, json!({ "op": "spawn", "agent": "claude", "cwd": "D:/proj", "text": "fix the bug" }));
        let bare = spawn_request(&args(&["codex"]));
        assert_eq!(bare, json!({ "op": "spawn", "agent": "codex", "cwd": null, "text": "" }));
        let row = json!([{ "paneId": "p1", "agent": "claude", "status": "working", "title": "Fix", "workspace": "W", "session": "S" }]);
        assert_eq!(render(&row), "p1  claude   working  Fix  [W / S]");
    }
}
