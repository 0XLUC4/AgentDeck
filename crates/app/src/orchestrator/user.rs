//! Requests of the `agentdeck` command line and the phone page (`user-request`, see
//! `agentdeck_core::remote`). Unlike an orchestrator, the user sees every workspace.

use agentdeck_core::agents::AgentKind;
use agentdeck_core::logic::deck::{Pane, PaneSpec, new_id};
use agentdeck_core::logic::orchestrator::assert_can_type;
use agentdeck_core::logic::text::agent_name;
use agentdeck_core::mcp;
use gpui_kit::*;
use serde_json::{Value, json};

use super::runtime;
use crate::state::Deck;

/// Longest prompt taken from outside the window.
const MAX_PROMPT: usize = 64 * 1024;

pub(super) fn on_request(deck: &Entity<Deck>, req: &Value, cx: &mut App) {
    let request_id = req["requestId"].as_str().unwrap_or("").to_owned();
    let op = req["op"].as_str().unwrap_or("").to_owned();
    let args = req["args"].clone();
    let deck = deck.clone();
    cx.spawn(async move |cx| {
        let (result, error) = match run(&deck, &op, &args, cx).await {
            Ok(value) => (Some(value), None),
            Err(e) => (None, Some(e)),
        };
        if let Err(e) = mcp::mcp_respond(request_id, result, error) {
            eprintln!("user request: {e}");
        }
    })
    .detach();
}

/// Name shown for a pane: its title, else its agent.
fn name(pane: &Pane) -> String {
    if pane.title.is_empty() { agent_name(pane.agent).to_owned() } else { pane.title.clone() }
}

/// The pane `wanted` names: its id, else the only pane with that title (case ignored).
fn find<'a>(panes: impl Iterator<Item = &'a Pane> + Clone, wanted: &str) -> Result<&'a Pane, String> {
    if let Some(pane) = panes.clone().find(|p| p.id == wanted) {
        return Ok(pane);
    }
    let mut named = panes.filter(|p| name(p).eq_ignore_ascii_case(wanted));
    match (named.next(), named.next()) {
        (Some(pane), None) => Ok(pane),
        (Some(_), Some(_)) => Err(format!("several agents are named \"{wanted}\": use the id")),
        _ => Err(format!("no agent \"{wanted}\"")),
    }
}

fn prompt_of(args: &Value, key: &str) -> Result<String, String> {
    let text = args[key].as_str().unwrap_or("").trim();
    match text.len() {
        0 => Err("empty prompt".into()),
        n if n > MAX_PROMPT => Err("prompt too long".into()),
        _ => Ok(text.to_owned()),
    }
}

async fn run(deck: &Entity<Deck>, op: &str, args: &Value, cx: &mut AsyncApp) -> Result<Value, String> {
    match op {
        "list" => Ok(deck.read_with(cx, |d, _| {
            let rows: Vec<Value> = d
                .state
                .tabs
                .iter()
                .flat_map(|tab| {
                    let workspace = d.state.workspaces.iter().find(|w| w.id == tab.workspace_id);
                    let workspace = workspace.map_or("", |w| w.name.as_str());
                    tab.pane_ids.iter().filter_map(|id| d.state.panes.get(id)).map(move |p| {
                        json!({
                            "paneId": p.id,
                            "agent": p.agent,
                            "title": name(p),
                            "topic": p.topic,
                            "status": if p.exited { json!("exited") } else { json!(p.status) },
                            "lastAction": p.last_action,
                            "workspace": workspace,
                            "session": tab.name,
                        })
                    })
                })
                .collect();
            json!(rows)
        })),
        "send" => {
            let text = prompt_of(args, "text")?;
            let wanted = args["pane"].as_str().unwrap_or("");
            let pane = deck.read_with(cx, |d, _| find(d.state.panes.values(), wanted).cloned())?;
            // Same rule as the orchestrator: not into a shell, not over a pending permission.
            assert_can_type(&pane)?;
            cx.update(|cx| runtime::send_prompt(&pane.id, &text, true, cx));
            Ok(json!({ "ok": true, "paneId": pane.id }))
        }
        "spawn" => {
            let agent: AgentKind = serde_json::from_value(args["agent"].clone())
                .map_err(|_| "agent: claude, codex, gemini, grok or opencode".to_owned())?;
            if agent == AgentKind::Shell {
                return Err("a shell cannot be started from outside".into());
            }
            let prompt = args["text"].as_str().filter(|t| !t.trim().is_empty()).map(|_| prompt_of(args, "text")).transpose()?;
            let (mut spec, tab_id) = deck.read_with(cx, |d, _| -> Result<_, String> {
                let workspace = &d.state.active_workspace().path;
                let cwd = match args["cwd"].as_str().filter(|c| !c.is_empty()) {
                    Some(cwd) => cwd.to_owned(),
                    None if workspace.is_empty() => d.state.last_cwd.clone(),
                    None => workspace.clone(),
                };
                if !std::path::Path::new(&cwd).is_dir() {
                    return Err(format!("not a folder: {cwd}"));
                }
                let tab = d.state.active_tab().ok_or("no session open")?;
                let mut spec = PaneSpec::new(agent, agent_name(agent), cwd);
                spec.font_size = d.settings.font_size;
                Ok((spec, tab.id.clone()))
            })?;
            spec.id = new_id();
            let id = spec.id.clone();
            match prompt {
                Some(text) => {
                    // Waits for the agent's start screen: run on its own so the caller is not held.
                    let deck = deck.clone();
                    cx.spawn(async move |cx| {
                        crate::palette::race_one(deck, spec, tab_id, text, cx).await;
                    })
                    .detach();
                }
                None => {
                    runtime::create_pane(deck, spec, tab_id, cx).await;
                }
            }
            Ok(json!({ "paneId": id }))
        }
        "toggle" => {
            let view = args["view"].as_str().unwrap_or("");
            cx.update(|cx| match view {
                "wall" => Ok(super::toggle_wall(cx)),
                "activity" => Ok(cx.dispatch_action(&crate::layout::ToggleActivity)),
                "projects" => Ok(cx.dispatch_action(&crate::layout::ToggleSidebar)),
                other => Err(format!("unknown view \"{other}\": wall, activity or projects")),
            })?;
            Ok(json!({ "ok": true }))
        }
        other => Err(format!("unknown command: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::find;
    use agentdeck_core::agents::AgentKind;
    use agentdeck_core::logic::deck::{Pane, PaneSpec, new_pane};

    fn pane(id: &str, title: &str) -> Pane {
        let mut spec = PaneSpec::new(AgentKind::Claude, title, "C:/x");
        spec.id = id.into();
        new_pane(spec, 0)
    }

    #[test]
    fn finds_by_id_then_unique_title() {
        let panes = [pane("a", "Fix login"), pane("b", "Tests"), pane("c", "tests")];
        assert_eq!(find(panes.iter(), "a").unwrap().id, "a");
        assert_eq!(find(panes.iter(), "fix LOGIN").unwrap().id, "a");
        assert!(find(panes.iter(), "tests").unwrap_err().contains("several"));
        assert!(find(panes.iter(), "nope").is_err());
    }
}
