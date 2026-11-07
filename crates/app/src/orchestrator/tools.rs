//! MCP tools that act on panes (src/lib/orchestrator.ts `handleMcpRequest`). The read-only tools
//! (list_agents, get_status, get_result, wait_for) are answered by `state::Deck`; everything else
//! arriving as `mcp-request` is run here and answered with `mcp::mcp_respond`.

use agentdeck_core::agents::AgentKind;
use agentdeck_core::git;
use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::{Pane, PaneSpec, new_id};
use agentdeck_core::logic::orchestrator::{
    DIFF_CAP_CHARS, ORCHESTRATOR_TITLE, OUTPUT_TAIL_CHARS, assert_can_type, resolve_inside, scope_of, target,
};
use agentdeck_core::mcp;
use chrono::{SecondsFormat, TimeZone, Utc};
use gpui_kit::*;
use serde_json::{Value, json};

use super::runtime;
use crate::state::{Deck, DeckEvent};

/// Answered synchronously by `state::Deck`.
const READ_TOOLS: [&str; 4] = ["list_agents", "get_status", "get_result", "wait_for"];

/// Pauses or resumes the orchestrator: launch, send and interrupt are refused while paused.
pub fn set_paused(deck: &Entity<Deck>, paused: bool, cx: &mut App) {
    mcp::mcp_set_paused(paused);
    deck.update(cx, |d, cx| {
        d.orchestrator.paused = paused;
        cx.notify();
    });
}

/// Starts an orchestrator Claude pane in the active session.
pub fn create_orchestrator(deck: &Entity<Deck>, cx: &mut App) {
    let d = deck.read(cx);
    let path = &d.state.active_workspace().path;
    let cwd = if path.is_empty() { d.state.last_cwd.clone() } else { path.clone() };
    let Some(tab_id) = d.state.active_tab().map(|t| t.id.clone()).filter(|_| !cwd.is_empty()) else {
        deck.update(cx, |d, cx| d.toast("Link a workspace first: the orchestrator works in its folder.", cx));
        return;
    };
    let mut spec = PaneSpec::new(AgentKind::Claude, ORCHESTRATOR_TITLE, cwd);
    spec.font_size = d.settings.font_size;
    spec.orchestrator = true;
    // A failed start is already shown on the pane and as a toast.
    crate::panes::runtime::create_pane(spec, &tab_id, false, cx);
}

pub fn on_deck_event(deck: &Entity<Deck>, event: &DeckEvent, cx: &mut App) {
    let DeckEvent::Backend(event) = event;
    if event.name != "mcp-request" {
        return;
    }
    let req = &event.payload;
    let tool = req["tool"].as_str().unwrap_or("").to_owned();
    if READ_TOOLS.contains(&tool.as_str()) {
        return;
    }
    let request_id = req["requestId"].as_str().unwrap_or("").to_owned();
    let orchestrator_id = req["orchestratorId"].as_str().unwrap_or("").to_owned();
    let args = req["args"].clone();
    let deck = deck.clone();
    cx.spawn(async move |cx| {
        let (result, error) = match run(&deck, &orchestrator_id, &tool, &args, cx).await {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };
        if let Err(e) = mcp::mcp_respond(request_id, result, error) {
            eprintln!("mcp_respond failed: {e}");
        }
    })
    .detach();
}

fn arg(args: &Value, key: &str) -> String {
    args[key].as_str().unwrap_or("").to_owned()
}

/// The target pane of `args.paneId`, checked against the orchestrator's workspace.
fn target_pane(deck: &Entity<Deck>, orchestrator_id: &str, args: &Value, cx: &AsyncApp) -> Result<Pane, String> {
    deck.read_with(cx, |d, _| {
        let scope = scope_of(&d.state, orchestrator_id)?;
        target(&scope, args["paneId"].as_str().unwrap_or("")).cloned()
    })
}

/// Runs a blocking backend call off the UI thread.
async fn blocking<R: Send + 'static>(cx: &AsyncApp, job: impl FnOnce() -> R + Send + 'static) -> R {
    cx.background_executor().spawn(async move { job() }).await
}

async fn run(deck: &Entity<Deck>, orchestrator_id: &str, tool: &str, args: &Value, cx: &mut AsyncApp) -> Result<Value, String> {
    match tool {
        "spawn_agent" => {
            let (spec, tab_id) = deck.read_with(cx, |d, _| -> Result<_, String> {
                let scope = scope_of(&d.state, orchestrator_id)?;
                let agent: AgentKind = serde_json::from_value(args["agent"].clone())
                    .map_err(|_| format!("Unknown agent: {}", args["agent"]))?;
                let mut spec = PaneSpec::new(agent, arg(args, "title"), resolve_inside(&scope.root, Some(&arg(args, "cwd")))?);
                spec.id = new_id();
                spec.args = args["args"]
                    .as_array()
                    .map(|a| a.iter().map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned)).collect())
                    .unwrap_or_default();
                spec.worktree = args["worktree"] == true;
                spec.font_size = d.settings.font_size;
                Ok((spec, scope.tab.id.clone()))
            })?;
            let id = spec.id.clone();
            runtime::create_pane(deck, spec, tab_id, cx).await;
            let pane = deck.read_with(cx, |d, _| d.state.panes.get(&id).cloned());
            let pane = pane.ok_or("The agent was closed while starting")?;
            if pane.status == PaneStatus::Error {
                return Err(format!(
                    "Failed to start: {}",
                    pane.last_action.as_deref().unwrap_or("unknown error")
                ));
            }
            Ok(json!({ "paneId": id }))
        }
        "send_prompt" => {
            let pane = target_pane(deck, orchestrator_id, args, cx)?;
            assert_can_type(&pane)?;
            let (text, submit) = (arg(args, "text"), args["submit"] != false);
            cx.update(|cx| runtime::send_prompt(&pane.id, &text, submit, cx));
            Ok(json!({ "ok": true }))
        }
        "read_output" => {
            let pane = target_pane(deck, orchestrator_id, args, cx)?;
            let raw = cx.update(|cx| runtime::pane_output_tail(&pane.id, OUTPUT_TAIL_CHARS, cx));
            Ok(json!({ "raw": raw }))
        }
        "interrupt" => {
            let pane = target_pane(deck, orchestrator_id, args, cx)?;
            assert_can_type(&pane)?;
            cx.update(|cx| runtime::interrupt_pane(&pane.id, cx));
            Ok(json!({ "ok": true }))
        }
        "get_diff" => {
            let pane = target_pane(deck, orchestrator_id, args, cx)?;
            let path = arg(args, "path");
            blocking(cx, move || get_diff(&pane, &path)).await
        }
        "merge_worktree" => {
            let pane = target_pane(deck, orchestrator_id, args, cx)?;
            if pane.status == PaneStatus::Working {
                return Err(format!("{} is still working: wait for it first", pane.title));
            }
            let message = arg(args, "commitMessage");
            blocking(cx, move || {
                let dir = pane_dir(&pane);
                if !message.is_empty() && !git::git_status(dir.clone())?.files.is_empty() {
                    git::git_commit(dir.clone(), message)?;
                }
                Ok(json!({ "result": git::git_merge_worktree(dir)? }))
            })
            .await
        }
        "restore_checkpoint" => {
            let pane = target_pane(deck, orchestrator_id, args, cx)?;
            if pane.status == PaneStatus::Working {
                return Err(format!("{} is working: interrupt it first", pane.title));
            }
            let sha = arg(args, "sha");
            blocking(cx, move || {
                let dir = pane_dir(&pane);
                let sha = if sha.is_empty() {
                    git::git_list_checkpoints(dir.clone(), pane.id.clone())?
                        .into_iter()
                        .next()
                        .map(|c| c.sha)
                        .ok_or_else(|| format!("{} has no restore point", pane.title))?
                } else {
                    sha
                };
                let backup = git::git_restore_checkpoint(dir, pane.id.clone(), sha.clone())?;
                Ok(json!({ "restored": sha, "backup": backup }))
            })
            .await
        }
        "close_agent" => {
            let pane = target_pane(deck, orchestrator_id, args, cx)?;
            cx.update(|cx| runtime::close_pane(&pane.id, cx));
            Ok(json!({ "ok": true }))
        }
        _ => Err(format!("Unknown tool: {tool}")),
    }
}

/// The pane's git folder: its worktree when it has one (lib/checkpoints.ts `paneGitDir`).
fn pane_dir(pane: &Pane) -> String {
    git::git_pane_dir(pane.cwd.clone(), pane.id.clone(), pane.worktree)
}

/// Blocking: status, last checkpoints and unified diffs (capped) of the pane's folder.
fn get_diff(pane: &Pane, path: &str) -> Result<Value, String> {
    let dir = pane_dir(pane);
    let status = git::git_status(dir.clone())?;
    let checkpoints = git::git_list_checkpoints(dir.clone(), pane.id.clone())?;
    let paths: Vec<String> = if path.is_empty() {
        status.files.iter().map(|f| f.path.clone()).collect()
    } else {
        vec![path.to_owned()]
    };
    let mut diff = String::new();
    for file in paths {
        if diff.len() >= DIFF_CAP_CHARS {
            diff.push_str("\n... (diff truncated: ask for one file with `path`)");
            break;
        }
        diff.push_str(&git::git_diff(dir.clone(), file)?);
    }
    let checkpoints: Vec<Value> = checkpoints
        .iter()
        .take(10)
        .map(|c| json!({ "sha": c.sha, "at": iso_date(c.ts) }))
        .collect();
    Ok(json!({
        "paneId": pane.id,
        "dir": dir,
        "branch": status.branch,
        "files": status.files,
        "checkpoints": checkpoints,
        "diff": truncate_chars(&diff, DIFF_CAP_CHARS + 100),
    }))
}

/// `new Date(ms).toISOString()`.
fn iso_date(ms: u64) -> String {
    i64::try_from(ms)
        .ok()
        .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
        .map(|d| d.to_rfc3339_opts(SecondsFormat::Millis, true))
        .unwrap_or_default()
}

fn truncate_chars(text: &str, max: usize) -> &str {
    text.char_indices().nth(max).map_or(text, |(i, _)| &text[..i])
}

#[cfg(test)]
mod tests {
    use super::{iso_date, truncate_chars};

    #[test]
    fn iso_and_truncate() {
        assert_eq!(iso_date(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(truncate_chars("h\u{e9}llo", 2), "h\u{e9}");
        assert_eq!(truncate_chars("hi", 5), "hi");
    }
}
