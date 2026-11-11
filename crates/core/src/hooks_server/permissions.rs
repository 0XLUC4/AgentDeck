//! Claude Code `PermissionRequest` hooks answered from the UI.
//!
//! The hook's HTTP request is held until the user answers (`answer_permission`), the hold expires,
//! the pane closes, or a later hook shows the prompt was answered in the terminal. A 2xx empty body
//! means "no decision": Claude Code falls back to its own terminal prompt.
//! Ref: https://code.claude.com/docs/en/hooks#permissionrequest-decision-control

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use super::{describe_tool, truncate};

/// Our answer must reach Claude before its hook timeout (600 s, set in agents/mod.rs) cancels the request.
pub const PERMISSION_HOLD: Duration = Duration::from_secs(590);
/// Cap on the tool input shown to the user (bytes).
const MAX_DETAIL: usize = 4 * 1024;

pub static PENDING: LazyLock<Permissions> = LazyLock::new(Permissions::default);

/// Payload of the `agent-permission` event.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub request_id: String,
    pub pane_id: String,
    pub agent: String,
    pub tool_name: String,
    /// "Bash: rm -rf build", "Edit src/x.ts".
    pub summary: String,
    /// Full Bash command, or the tool input as pretty JSON; capped.
    pub detail: String,
    /// Rules "Toujours autoriser" would add, e.g. "Bash(npm test)". Empty: no such option.
    pub suggestions: Vec<String>,
    /// `AskUserQuestion` only: the questions, answered with [`answer_questions`] instead of allow/deny.
    #[serde(default)]
    pub questions: Vec<Question>,
}

/// One `AskUserQuestion` question with the choices the agent offers.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub question: String,
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub multi_select: bool,
    pub options: Vec<QuestionOption>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

struct Pending {
    pane_id: String,
    tool_name: Value,
    tool_input: Value,
    /// `addRules`/allow entries from `permission_suggestions`, echoed back as `updatedPermissions`.
    always: Vec<Value>,
    /// "Bash: rm -rf build": shown on the phone page.
    summary: String,
    tx: Sender<Option<Value>>,
}

#[derive(Default)]
pub struct Permissions(Mutex<HashMap<String, Pending>>);

impl Permissions {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pending>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn register(
        &self,
        pane_id: &str,
        body: &Value,
    ) -> (PermissionRequest, Receiver<Option<Value>>) {
        let (tx, rx) = channel();
        let request_id = uuid::Uuid::new_v4().to_string();
        // Only rule additions that allow: never forward a suggestion that changes the permission mode.
        let always: Vec<Value> = body["permission_suggestions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["type"] == "addRules" && s["behavior"] == "allow")
            .cloned()
            .collect();
        let suggestions = always
            .iter()
            .flat_map(|s| s["rules"].as_array().cloned().unwrap_or_default())
            .filter_map(|r| {
                let tool = r["toolName"].as_str()?;
                Some(match r["ruleContent"].as_str() {
                    Some(content) => format!("{tool}({content})"),
                    None => tool.to_owned(),
                })
            })
            .collect();
        let input = &body["tool_input"];
        let detail = match input["command"].as_str() {
            Some(cmd) => cmd.to_owned(),
            None => serde_json::to_string_pretty(input).unwrap_or_default(),
        };
        let truncated = truncate(&detail, MAX_DETAIL);
        let detail = if truncated.len() < detail.len() {
            format!("{truncated}\n... (truncated, see the terminal)")
        } else {
            truncated
        };
        let tool_name = body["tool_name"].as_str().unwrap_or("?").to_owned();
        let questions = if tool_name == "AskUserQuestion" {
            serde_json::from_value(input["questions"].clone()).unwrap_or_default()
        } else {
            Vec::new()
        };
        let request = PermissionRequest {
            request_id: request_id.clone(),
            pane_id: pane_id.to_owned(),
            agent: "claude".into(),
            summary: describe_tool(body).unwrap_or_else(|| tool_name.clone()),
            tool_name,
            detail,
            suggestions,
            questions,
        };
        self.lock().insert(
            request_id,
            Pending {
                pane_id: pane_id.to_owned(),
                tool_name: body["tool_name"].clone(),
                tool_input: input.clone(),
                always,
                summary: request.summary.clone(),
                tx,
            },
        );
        (request, rx)
    }

    /// Pending requests (phone page): id, pane, what is asked. A question is answered on the desktop.
    pub fn list(&self) -> Vec<Value> {
        self.lock()
            .iter()
            .map(|(id, p)| {
                json!({ "requestId": id, "paneId": p.pane_id, "summary": p.summary, "question": p.tool_name == "AskUserQuestion" })
            })
            .collect()
    }

    /// Unblocks the held hook. `decision`: "allow" | "allow_always" | "deny" | "ask" (back to the terminal).
    pub fn answer(&self, id: &str, decision: &str, message: Option<&str>) -> Result<(), String> {
        if !matches!(decision, "allow" | "allow_always" | "deny" | "ask") {
            return Err(format!("unknown decision: {decision}"));
        }
        let pending = self
            .lock()
            .remove(id)
            .ok_or("Request expired or already handled")?;
        let body = decision_body(decision, message, &pending.always);
        // The holding thread may have just timed out: nothing left to answer.
        let _ = pending.tx.send(body);
        Ok(())
    }

    /// Allows an `AskUserQuestion` call with the user's picks (question text -> label(s), comma-joined),
    /// the way Claude Code's own prompt fills `answers`.
    pub fn answer_questions(&self, id: &str, answers: &HashMap<String, String>) -> Result<(), String> {
        let pending = self
            .lock()
            .remove(id)
            .ok_or("Request expired or already handled")?;
        let mut input = pending.tool_input;
        input["answers"] = json!(answers);
        let body = json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": { "behavior": "allow", "updatedInput": input }
            }
        });
        let _ = pending.tx.send(Some(body));
        Ok(())
    }

    /// A later hook from the same pane means the prompt was answered elsewhere (terminal): release it.
    pub fn cancel_stale(&self, pane_id: &str, body: &Value) {
        let event = body["hook_event_name"].as_str().unwrap_or("");
        let whole_turn = matches!(
            event,
            "UserPromptSubmit" | "Stop" | "StopFailure" | "SessionStart"
        );
        let tool_done = matches!(event, "PostToolUse" | "PostToolUseFailure");
        if !whole_turn && !tool_done {
            return;
        }
        // An `AskUserQuestion` answered in the terminal comes back with `answers` added to its input.
        let mut done_input = body["tool_input"].clone();
        if let Some(input) = done_input.as_object_mut() {
            input.remove("answers");
        }
        self.lock().retain(|_, p| {
            let stale = p.pane_id == pane_id
                && (whole_turn
                    || (p.tool_name == body["tool_name"] && p.tool_input == done_input));
            if stale {
                let _ = p.tx.send(None);
            }
            !stale
        });
    }

    pub fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    /// True while `pane_id` still has a request the user has not answered.
    pub fn has_pane(&self, pane_id: &str) -> bool {
        self.lock().values().any(|p| p.pane_id == pane_id)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

/// Response body for Claude Code; `None` = no decision (2xx empty body, terminal prompt shows).
pub fn decision_body(decision: &str, message: Option<&str>, always: &[Value]) -> Option<Value> {
    let decision = match decision {
        "allow" => json!({ "behavior": "allow" }),
        "allow_always" if !always.is_empty() => {
            json!({ "behavior": "allow", "updatedPermissions": always })
        }
        "allow_always" => json!({ "behavior": "allow" }),
        "deny" => match message.map(str::trim).filter(|m| !m.is_empty()) {
            Some(m) => json!({ "behavior": "deny", "message": m }),
            None => json!({ "behavior": "deny" }),
        },
        _ => return None,
    };
    Some(json!({
        "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision }
    }))
}

/// Blocks until answered, `hold` elapses, or `alive()` turns false (pane closed). Checks every second.
pub fn wait_decision(
    rx: &Receiver<Option<Value>>,
    hold: Duration,
    alive: impl Fn() -> bool,
) -> Option<Value> {
    let deadline = Instant::now() + hold;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left.min(Duration::from_secs(1))) {
            Ok(decision) => return decision,
            Err(RecvTimeoutError::Disconnected) => return None,
            Err(RecvTimeoutError::Timeout) if left.is_zero() || !alive() => return None,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Answers a pending permission prompt from the UI.
pub fn answer_permission(
    request_id: String,
    decision: String,
    message: Option<String>,
) -> Result<(), String> {
    PENDING.answer(&request_id, &decision, message.as_deref())
}

/// Answers a pending `AskUserQuestion` from the UI.
pub fn answer_questions(request_id: &str, answers: &HashMap<String, String>) -> Result<(), String> {
    PENDING.answer_questions(request_id, answers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(cmd: &str) -> Value {
        json!({
            "hook_event_name": "PermissionRequest",
            "cwd": "/repo",
            "tool_name": "Bash",
            "tool_input": { "command": cmd, "description": "x" },
            "permission_suggestions": [
                { "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "npm test" }],
                  "behavior": "allow", "destination": "localSettings" },
                { "type": "setMode", "mode": "acceptEdits", "destination": "session" }
            ]
        })
    }

    #[test]
    fn register_describes_request() {
        let p = Permissions::default();
        let (req, _rx) = p.register("p1", &request("npm test"));
        assert_eq!(req.summary, "Bash: npm test");
        assert_eq!(req.detail, "npm test");
        assert_eq!(req.suggestions, vec!["Bash(npm test)".to_string()]);
        assert_eq!(req.agent, "claude");
        let long = "x".repeat(MAX_DETAIL + 10);
        let (req, _rx) = p.register("p1", &request(&long));
        assert!(req.detail.ends_with("(truncated, see the terminal)"));
        let (req, _rx) = p.register("p1", &json!({"tool_name": "Edit", "tool_input": {"a": 1}}));
        assert!(req.detail.contains("\"a\": 1"));
        assert!(req.suggestions.is_empty());
    }

    #[test]
    fn answer_unblocks_with_decision() {
        let p = Permissions::default();
        let (req, rx) = p.register("p1", &request("npm test"));
        assert!(p.answer(&req.request_id, "maybe", None).is_err());
        p.answer(&req.request_id, "allow_always", None).unwrap();
        let body = wait_decision(&rx, Duration::from_secs(5), || true).unwrap();
        let d = &body["hookSpecificOutput"]["decision"];
        assert_eq!(
            body["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest"
        );
        assert_eq!(d["behavior"], "allow");
        // Only the addRules suggestion is echoed, never the setMode one.
        assert_eq!(d["updatedPermissions"].as_array().unwrap().len(), 1);
        assert_eq!(
            d["updatedPermissions"][0]["rules"][0]["ruleContent"],
            "npm test"
        );
        assert_eq!(p.len(), 0);
        assert!(
            p.answer(&req.request_id, "allow", None).is_err(),
            "answered twice"
        );
    }

    #[test]
    fn questions_are_parsed_and_answered() {
        let p = Permissions::default();
        let body = json!({"tool_name": "AskUserQuestion", "tool_input": {"questions": [
            {"question": "Which DB?", "header": "DB", "multiSelect": false,
             "options": [{"label": "SQLite", "description": "local"}, {"label": "Postgres", "description": "server"}]}
        ]}});
        let (req, rx) = p.register("p1", &body);
        assert_eq!(req.questions.len(), 1);
        assert_eq!(req.questions[0].options[1].label, "Postgres");
        let answers = HashMap::from([("Which DB?".to_string(), "SQLite".to_string())]);
        p.answer_questions(&req.request_id, &answers).unwrap();
        let out = wait_decision(&rx, Duration::from_secs(5), || true).unwrap();
        let d = &out["hookSpecificOutput"]["decision"];
        assert_eq!(d["behavior"], "allow");
        assert_eq!(d["updatedInput"]["answers"]["Which DB?"], "SQLite");
        assert_eq!(d["updatedInput"]["questions"][0]["header"], "DB");
        // Other tools never get questions.
        let (req, _rx) = p.register("p1", &request("ls"));
        assert!(req.questions.is_empty());
    }

    #[test]
    fn ask_releases_without_decision() {
        let p = Permissions::default();
        let (req, rx) = p.register("p1", &request("ls"));
        p.answer(&req.request_id, "ask", None).unwrap();
        assert_eq!(wait_decision(&rx, Duration::from_secs(5), || true), None);
    }

    #[test]
    fn times_out_or_stops_when_pane_closes() {
        let p = Permissions::default();
        let (_req, rx) = p.register("p1", &request("ls"));
        let start = Instant::now();
        assert_eq!(wait_decision(&rx, Duration::from_millis(50), || true), None);
        assert!(start.elapsed() < Duration::from_secs(1));
        let start = Instant::now();
        assert_eq!(wait_decision(&rx, Duration::from_secs(60), || false), None);
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn later_event_cancels_stale_request() {
        let p = Permissions::default();
        let (_a, rx_a) = p.register("p1", &request("rm -rf build"));
        let (_b, _rx_b) = p.register("p2", &request("rm -rf build"));
        // Another tool finishing in parallel does not cancel it.
        p.cancel_stale(
            "p1",
            &json!({"hook_event_name": "PostToolUse", "tool_name": "Read", "tool_input": {"file_path": "x"}}),
        );
        p.cancel_stale("p1", &json!({"hook_event_name": "Notification"}));
        assert_eq!(p.len(), 2);
        // The same call completing means it was allowed in the terminal.
        p.cancel_stale(
            "p1",
            &json!({"hook_event_name": "PostToolUse", "tool_name": "Bash",
                "tool_input": {"command": "rm -rf build", "description": "x"}}),
        );
        assert_eq!(p.len(), 1);
        assert_eq!(rx_a.recv_timeout(Duration::from_secs(1)), Ok(None));
        p.cancel_stale("p2", &json!({"hook_event_name": "Stop"}));
        assert_eq!(p.len(), 0);
        // A question answered in the terminal: same call, now with `answers`.
        let questions = json!([{"question": "Which DB?", "options": [{"label": "SQLite"}]}]);
        let (_q, rx_q) = p.register(
            "p1",
            &json!({"tool_name": "AskUserQuestion", "tool_input": {"questions": questions}}),
        );
        p.cancel_stale(
            "p1",
            &json!({"hook_event_name": "PostToolUse", "tool_name": "AskUserQuestion",
                "tool_input": {"questions": questions, "answers": {"Which DB?": "SQLite"}}}),
        );
        assert_eq!(p.len(), 0);
        assert_eq!(rx_q.recv_timeout(Duration::from_secs(1)), Ok(None));
    }

    #[test]
    fn decision_json_matches_docs() {
        assert_eq!(
            decision_body("deny", Some("  not that "), &[]),
            Some(
                json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest",
                "decision": {"behavior": "deny", "message": "not that"}}})
            )
        );
        assert_eq!(
            decision_body("deny", Some(" "), &[]).unwrap()["hookSpecificOutput"]["decision"],
            json!({"behavior": "deny"})
        );
        assert_eq!(
            decision_body("allow", Some("ignored"), &[]).unwrap()["hookSpecificOutput"]["decision"],
            json!({"behavior": "allow"})
        );
        assert_eq!(decision_body("ask", None, &[]), None);
    }
}
