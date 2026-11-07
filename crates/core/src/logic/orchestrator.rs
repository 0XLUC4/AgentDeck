//! Orchestrator logic (`src/lib/orchestrator.ts`): a Claude pane driving the other agents of its
//! workspace through AgentDeck's MCP server ([`crate::mcp`]). The backend validates arguments and
//! emits `mcp-request`; this side checks scope and answers the read-only tools. Tools that act on panes
//! (spawn_agent, send_prompt, interrupt, get_diff, merge_worktree, restore_checkpoint, close_agent,
//! read_output) are run by the app with [`Scope`] and [`target`], then answered via `mcp::mcp_respond`.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::deck::{DeckState, Pane, Tab, activity_key};
use super::tasks::TaskLog;
use crate::agents::AgentKind;
use crate::hooks_server::PaneStatus;

pub const ORCHESTRATOR_TITLE: &str = "Orchestrator";
pub const MAX_LOG: usize = 100;
/// Raw terminal text handed to the backend for read_output.
pub const OUTPUT_TAIL_CHARS: usize = 64 * 1024;
pub const RESULT_ACTIONS: usize = 30;
/// get_diff without a path: unified diffs of the changed files until this many characters.
pub const DIFF_CAP_CHARS: usize = 48 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct ActivityEntry {
    pub at: i64,
    pub tool: String,
    /// "Tests (codex)" or the args summary.
    pub summary: String,
    pub error: Option<String>,
}

/// Activity log per orchestrator pane, newest first.
#[derive(Debug, Clone, Default)]
pub struct OrchestratorState {
    pub paused: bool,
    pub log: HashMap<String, Vec<ActivityEntry>>,
}

impl OrchestratorState {
    /// Records an `mcp-activity` event payload.
    pub fn record(&mut self, event: &Value, panes: &HashMap<String, Pane>, now: i64) {
        let id = event["orchestratorId"].as_str().unwrap_or("").to_owned();
        let tool = event["tool"].as_str().unwrap_or("").to_owned();
        let entry = ActivityEntry {
            at: now,
            summary: summarize_call(&tool, &event["args"], panes),
            tool,
            error: event["error"].as_str().map(str::to_owned),
        };
        let list = self.log.entry(id).or_default();
        list.insert(0, entry);
        list.truncate(MAX_LOG);
    }
}

fn pane_name(pane: Option<&Pane>, id: &str) -> String {
    match pane {
        Some(p) => format!(
            "{} ({})",
            p.title,
            serde_json::to_value(p.agent)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
        ),
        None => id.chars().take(8).collect(),
    }
}

/// One-line, human summary of a tool call for the activity log.
pub fn summarize_call(tool: &str, args: &Value, panes: &HashMap<String, Pane>) -> String {
    let target = args["paneId"]
        .as_str()
        .map(|id| pane_name(panes.get(id), id))
        .unwrap_or_default();
    let s = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    match tool {
        "spawn_agent" => format!(
            "{} \"{}\"{}",
            s(&args["agent"]),
            s(&args["title"]),
            if args["worktree"] == true { " \u{b7} worktree" } else { "" }
        ),
        "send_prompt" => {
            let text = args["text"]
                .as_str()
                .unwrap_or("")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let short = if text.chars().count() > 60 {
                format!("{}\u{2026}", text.chars().take(60).collect::<String>())
            } else {
                text
            };
            format!("{target}: {short}")
        }
        "wait_for" => {
            let ids: Vec<String> = args["paneIds"]
                .as_array()
                .map(|a| a.iter().map(|v| pane_name(panes.get(&s(v)), &s(v))).collect())
                .unwrap_or_default();
            format!("{} \u{2192} {}", ids.join(", "), s(&args["until"]))
        }
        _ => target,
    }
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// Absolute folder for `cwd` (relative to `root` when not absolute); errors when it leaves `root`.
pub fn resolve_inside(root: &str, cwd: Option<&str>) -> Result<String, String> {
    let Some(cwd) = cwd.filter(|c| !c.is_empty()) else {
        return Ok(root.to_owned());
    };
    if cwd.split(['\\', '/']).any(|seg| seg == "..") {
        return Err("cwd cannot contain ..".into());
    }
    let bytes = cwd.as_bytes();
    let absolute = cwd.starts_with(['\\', '/'])
        || (bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/'));
    let sep = if root.contains('\\') { "\\" } else { "/" };
    let full = if absolute {
        cwd.to_owned()
    } else {
        format!("{}{sep}{cwd}", root.trim_end_matches(['\\', '/']))
    };
    let (base, path) = (normalize(root), normalize(&full));
    if path != base && !path.starts_with(&format!("{base}/")) {
        return Err(format!("cwd must stay inside {root}"));
    }
    Ok(full)
}

/// The orchestrator's workspace: every pane of every session of it, except the orchestrator itself.
pub struct Scope<'a> {
    pub orchestrator: &'a Pane,
    pub tab: &'a Tab,
    /// Folder new agents must stay in.
    pub root: String,
    pub panes: Vec<(&'a Pane, &'a Tab)>,
}

pub fn scope_of<'a>(deck: &'a DeckState, orchestrator_id: &str) -> Result<Scope<'a>, String> {
    let orchestrator = deck
        .panes
        .get(orchestrator_id)
        .filter(|p| p.orchestrator)
        .ok_or("Orchestrator not found")?;
    let tab = deck
        .tabs
        .iter()
        .find(|t| t.pane_ids.iter().any(|id| id == orchestrator_id))
        .ok_or("Orchestrator not found")?;
    let root = deck
        .workspaces
        .iter()
        .find(|w| w.id == tab.workspace_id)
        .map(|w| w.path.clone())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| orchestrator.cwd.clone());
    let panes = deck
        .tabs
        .iter()
        .filter(|t| t.workspace_id == tab.workspace_id)
        .flat_map(|t| {
            t.pane_ids
                .iter()
                .filter(|id| *id != orchestrator_id)
                .filter_map(|id| deck.panes.get(id))
                .map(move |p| (p, t))
        })
        .collect();
    Ok(Scope {
        orchestrator,
        tab,
        root,
        panes,
    })
}

pub fn target<'a>(scope: &Scope<'a>, pane_id: &str) -> Result<&'a Pane, String> {
    if pane_id == scope.orchestrator.id {
        return Err("You cannot control yourself".into());
    }
    let pane = scope
        .panes
        .iter()
        .find(|(p, _)| p.id == pane_id)
        .map(|(p, _)| *p)
        .ok_or_else(|| format!("Agent {pane_id} not found in your workspace (see list_agents)"))?;
    // Two orchestrators driving each other would loop with no human in between.
    if pane.orchestrator {
        return Err(format!("{} is another orchestrator: it cannot be controlled", pane.title));
    }
    Ok(pane)
}

/// Permission prompts are the user's to answer: no keystrokes go to a waiting agent.
pub fn assert_can_type(pane: &Pane) -> Result<(), String> {
    // Typing into a shell runs commands with no permission prompt: shells stay the user's.
    if pane.agent == AgentKind::Shell {
        return Err(format!("{} is a shell: only the user types there", pane.title));
    }
    if pane.exited {
        return Err(format!("{} is stopped", pane.title));
    }
    if pane.status == PaneStatus::Waiting {
        return Err(format!("{} is waiting for the user's answer: try again later", pane.title));
    }
    Ok(())
}

/// Answers the tools that only read state. `None`: the tool acts on panes, the app runs it.
pub fn handle_read_tool(
    deck: &DeckState,
    tasks: &HashMap<String, TaskLog>,
    orchestrator_id: &str,
    tool: &str,
    args: &Value,
    now: i64,
) -> Option<Result<Value, String>> {
    let scope = match scope_of(deck, orchestrator_id) {
        Ok(s) => s,
        Err(e) => return Some(Err(e)),
    };
    let pane_arg = || target(&scope, args["paneId"].as_str().unwrap_or(""));
    let result = match tool {
        "list_agents" => Ok(Value::Array(
            scope
                .panes
                .iter()
                .map(|(p, t)| {
                    json!({
                        "paneId": p.id, "agent": p.agent, "title": p.title,
                        "status": activity_key(p).as_str(), "lastAction": p.last_action,
                        "session": t.name, "cwd": p.cwd,
                    })
                })
                .collect(),
        )),
        "get_status" => pane_arg().map(|p| {
            json!({
                "paneId": p.id, "status": activity_key(p).as_str(), "lastAction": p.last_action,
                "timeInStatusSec": ((now - p.status_since) as f64 / 1000.0).round() as i64,
            })
        }),
        "get_result" => pane_arg().map(|p| {
            let log = tasks.get(&p.id);
            let task = log.and_then(|l| l.history.first());
            json!({
                "paneId": p.id,
                "running": log.is_some_and(|l| l.current.is_some()),
                "result": task.map(|t| json!({
                    "prompt": t.prompt, "summary": t.summary, "status": t.status,
                    "actions": t.actions.iter().rev().take(RESULT_ACTIONS).rev().map(|a| a.text.clone()).collect::<Vec<_>>(),
                    "durationSec": t.ended_at.map(|e| ((e - t.started_at) as f64 / 1000.0).round() as i64),
                })),
            })
        }),
        "wait_for" => args["paneIds"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|id| {
                let id = id.as_str().unwrap_or("");
                target(&scope, id).map(|p| (id.to_owned(), json!(p.status)))
            })
            .collect::<Result<serde_json::Map<_, _>, _>>()
            .map(|statuses| json!({ "statuses": statuses })),
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentKind;
    use crate::logic::deck::{PaneSpec, new_pane};

    fn deck() -> DeckState {
        let mut d = DeckState::default();
        let tab = d.tabs[0].id.clone();
        for (id, agent, orch) in [("o", AgentKind::Claude, true), ("a", AgentKind::Claude, false), ("b", AgentKind::Codex, false)] {
            let mut s = PaneSpec::new(agent, id, "C:\\proj");
            s.id = id.into();
            s.orchestrator = orch;
            d.add_pane(new_pane(s, 0), &tab);
        }
        d
    }

    #[test]
    fn resolve_inside_cases() {
        assert_eq!(resolve_inside("C:\\proj", Some("")).unwrap(), "C:\\proj");
        assert_eq!(resolve_inside("C:\\proj", Some("C:/proj/a")).unwrap(), "C:/proj/a");
        assert!(resolve_inside("C:\\proj", Some("C:\\project2")).is_err());
        assert!(resolve_inside("C:\\proj", Some("a\\..\\..\\x")).unwrap_err().contains(".."));
        assert_eq!(resolve_inside("/home/u/p", Some("src")).unwrap(), "/home/u/p/src");
    }

    #[test]
    fn summaries() {
        let d = deck();
        assert_eq!(
            summarize_call("send_prompt", &json!({ "paneId": "b", "text": "go  now" }), &d.panes),
            "b (codex): go now"
        );
        assert_eq!(
            summarize_call("wait_for", &json!({ "paneIds": ["a", "zzzzzzzzzz"], "until": "done" }), &d.panes),
            "a (claude), zzzzzzzz \u{2192} done"
        );
    }

    #[test]
    fn scope_and_read_tools() {
        let d = deck();
        let tasks = HashMap::new();
        let list = handle_read_tool(&d, &tasks, "o", "list_agents", &json!({}), 0).unwrap().unwrap();
        assert_eq!(list.as_array().unwrap().len(), 2);
        assert!(handle_read_tool(&d, &tasks, "a", "list_agents", &json!({}), 0).unwrap().is_err());
        let me = handle_read_tool(&d, &tasks, "o", "get_status", &json!({ "paneId": "o" }), 0).unwrap();
        assert!(me.unwrap_err().contains("yourself"));
        assert!(handle_read_tool(&d, &tasks, "o", "spawn_agent", &json!({}), 0).is_none());
        let mut waiting = d.panes["a"].clone();
        waiting.status = PaneStatus::Waiting;
        assert!(assert_can_type(&waiting).is_err());
    }
}
