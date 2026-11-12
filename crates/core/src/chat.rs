//! Chat mode: the JSONL event streams of `claude -p --output-format stream-json` and
//! `codex exec --json`, parsed into one small typed model the chat view renders.
//! Pure: no process, no UI. Unknown lines and fields are ignored (both formats grow over time).

use serde_json::{Value, json};

/// Longest tool output kept in the transcript (bytes); the rest is cut.
const MAX_OUTPUT: usize = 16 * 1024;
/// Longest one-line summary of a tool call's input.
const MAX_SUMMARY: usize = 120;

/// One thing an agent reported.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    /// Conversation id, to resume it (Claude `session_id`, Codex `thread_id`).
    Session(String),
    /// Streamed piece of the assistant's current text block.
    TextDelta(String),
    /// A whole assistant text block (replaces the streamed pieces of that block).
    Text(String),
    ToolStart { id: String, name: String, input: String },
    ToolResult { id: String, output: String, error: bool },
    /// Non-fatal message from the CLI (config warnings...).
    Notice(String),
    /// The turn is over; `Some` when it failed.
    TurnEnd { error: Option<String> },
}

/// A row of the chat.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    User(String),
    Assistant(String),
    Tool(Tool),
    Notice(String),
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub id: String,
    pub name: String,
    /// Short one-line summary of the input (command, path...).
    pub input: String,
    /// None while running.
    pub output: Option<String>,
    pub error: bool,
}

/// What the chat view shows, built by applying events in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    pub items: Vec<Item>,
    /// A turn is running.
    pub busy: bool,
    pub session: Option<String>,
    /// The last item is an assistant block still receiving deltas.
    streaming: bool,
}

impl Transcript {
    /// The user sent a message: a turn starts.
    pub fn push_user(&mut self, text: &str) {
        self.items.push(Item::User(text.to_owned()));
        self.busy = true;
        self.streaming = false;
    }

    pub fn apply(&mut self, event: ChatEvent) {
        match event {
            ChatEvent::Session(id) => self.session = Some(id),
            ChatEvent::TextDelta(text) => match self.items.last_mut() {
                Some(Item::Assistant(current)) if self.streaming => current.push_str(&text),
                _ => {
                    self.items.push(Item::Assistant(text));
                    self.streaming = true;
                }
            },
            ChatEvent::Text(text) => {
                match self.items.last_mut() {
                    Some(Item::Assistant(current)) if self.streaming => *current = text,
                    _ if text.trim().is_empty() => {}
                    _ => self.items.push(Item::Assistant(text)),
                }
                self.streaming = false;
            }
            ChatEvent::ToolStart { id, name, input } => {
                self.streaming = false;
                // Codex reports a command when it starts and again when it completes.
                if !self.items.iter().any(|item| matches!(item, Item::Tool(t) if t.id == id)) {
                    self.items.push(Item::Tool(Tool { id, name, input, output: None, error: false }));
                }
            }
            ChatEvent::ToolResult { id, output, error } => {
                let tool = self.items.iter_mut().rev().find_map(|item| match item {
                    Item::Tool(t) if t.id == id => Some(t),
                    _ => None,
                });
                if let Some(tool) = tool {
                    tool.output = Some(output);
                    tool.error = error;
                }
            }
            ChatEvent::Notice(text) => self.items.push(Item::Notice(text)),
            ChatEvent::TurnEnd { error } => {
                self.busy = false;
                self.streaming = false;
                if let Some(error) = error {
                    self.items.push(Item::Error(error));
                }
            }
        }
    }

    /// Last assistant text (the turn's answer, for notifications).
    pub fn last_answer(&self) -> Option<&str> {
        self.items.iter().rev().find_map(|item| match item {
            Item::Assistant(text) => Some(text.as_str()),
            _ => None,
        })
    }
}

/// Claude stream-json input line carrying a user message.
pub fn claude_user_line(text: &str) -> String {
    json!({"type": "user", "message": {"role": "user", "content": text}}).to_string()
}

/// Claude stream-json control request that interrupts the running turn (the process stays).
pub fn claude_interrupt_line(request_id: &str) -> String {
    json!({"type": "control_request", "request_id": request_id, "request": {"subtype": "interrupt"}}).to_string()
}

/// Events of one `claude -p --output-format stream-json --verbose --include-partial-messages` line.
pub fn parse_claude(line: &str) -> Vec<ChatEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Vec::new() };
    // Subagent traffic (Task tool) is not the main conversation.
    if v["parent_tool_use_id"].is_string() {
        return Vec::new();
    }
    match v["type"].as_str().unwrap_or("") {
        "system" if v["subtype"] == "init" => str_of(&v["session_id"]).map(ChatEvent::Session).into_iter().collect(),
        "stream_event" => {
            let e = &v["event"];
            match (e["type"].as_str(), e["delta"]["type"].as_str()) {
                (Some("content_block_delta"), Some("text_delta")) => {
                    str_of(&e["delta"]["text"]).map(ChatEvent::TextDelta).into_iter().collect()
                }
                _ => Vec::new(),
            }
        }
        "assistant" => blocks(&v["message"]["content"])
            .filter_map(|b| match b["type"].as_str()? {
                "text" => Some(ChatEvent::Text(str_of(&b["text"])?)),
                "tool_use" => Some(ChatEvent::ToolStart {
                    id: str_of(&b["id"])?,
                    name: str_of(&b["name"]).unwrap_or_default(),
                    input: summarize_input(&b["input"]),
                }),
                _ => None,
            })
            .collect(),
        "user" => blocks(&v["message"]["content"])
            .filter(|b| b["type"] == "tool_result")
            .filter_map(|b| {
                Some(ChatEvent::ToolResult {
                    id: str_of(&b["tool_use_id"])?,
                    output: cap(&content_text(&b["content"])),
                    error: b["is_error"].as_bool().unwrap_or(false),
                })
            })
            .collect(),
        "result" => {
            let failed = v["is_error"].as_bool().unwrap_or(false) || v["subtype"].as_str().is_some_and(|s| s != "success");
            let error = failed.then(|| {
                str_of(&v["result"])
                    .filter(|r| !r.is_empty())
                    .or_else(|| str_of(&v["subtype"]))
                    .unwrap_or_else(|| "Turn failed".into())
            });
            vec![ChatEvent::TurnEnd { error }]
        }
        _ => Vec::new(),
    }
}

/// Events of one `codex exec --json` line.
pub fn parse_codex(line: &str) -> Vec<ChatEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Vec::new() };
    match v["type"].as_str().unwrap_or("") {
        "thread.started" => str_of(&v["thread_id"]).map(ChatEvent::Session).into_iter().collect(),
        "turn.completed" => vec![ChatEvent::TurnEnd { error: None }],
        "turn.failed" => vec![ChatEvent::TurnEnd {
            error: Some(str_of(&v["error"]["message"]).unwrap_or_else(|| "Turn failed".into())),
        }],
        "error" => str_of(&v["message"]).map(ChatEvent::Notice).into_iter().collect(),
        kind @ ("item.started" | "item.updated" | "item.completed") => codex_item(&v["item"], kind == "item.completed"),
        _ => Vec::new(),
    }
}

fn codex_item(item: &Value, completed: bool) -> Vec<ChatEvent> {
    let Some(id) = str_of(&item["id"]) else { return Vec::new() };
    let (name, input) = match item["type"].as_str().unwrap_or("") {
        "agent_message" if completed => return str_of(&item["text"]).map(ChatEvent::Text).into_iter().collect(),
        "error" if completed => return str_of(&item["message"]).map(ChatEvent::Notice).into_iter().collect(),
        "command_execution" => ("shell".to_owned(), one_line(&str_of(&item["command"]).unwrap_or_default())),
        "file_change" => {
            let paths: Vec<&str> = item["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["path"].as_str())
                .collect();
            ("edit".to_owned(), one_line(&paths.join(", ")))
        }
        "mcp_tool_call" => (
            format!("{}.{}", item["server"].as_str().unwrap_or("mcp"), item["tool"].as_str().unwrap_or("?")),
            summarize_input(&item["arguments"]),
        ),
        "web_search" => ("web_search".to_owned(), one_line(&str_of(&item["query"]).unwrap_or_default())),
        _ => return Vec::new(),
    };
    let mut events = vec![ChatEvent::ToolStart { id: id.clone(), name, input }];
    if completed {
        let status = item["status"].as_str().unwrap_or("");
        let exit_code = item["exit_code"].as_i64().unwrap_or(0);
        let output = str_of(&item["aggregated_output"])
            .or_else(|| item["result"].as_object().map(|_| content_text(&item["result"]["content"])))
            .or_else(|| str_of(&item["error"]["message"]))
            .unwrap_or_default();
        events.push(ChatEvent::ToolResult { id, output: cap(&output), error: status == "failed" || exit_code != 0 });
    }
    events
}

fn str_of(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}

fn blocks(content: &Value) -> impl Iterator<Item = &Value> {
    content.as_array().into_iter().flatten()
}

/// Text of a tool result: a string, or an array of `{type: "text", text}` blocks.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The most telling field of a tool input on one line: command, path, pattern, url... else the JSON.
fn summarize_input(input: &Value) -> String {
    const KEYS: [&str; 8] = ["command", "file_path", "path", "pattern", "url", "query", "description", "prompt"];
    let text = KEYS
        .iter()
        .find_map(|k| input[k].as_str().map(str::to_owned))
        .unwrap_or_else(|| match input {
            Value::Object(map) if map.is_empty() => String::new(),
            Value::Null => String::new(),
            other => other.to_string(),
        });
    one_line(&text)
}

fn one_line(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(MAX_SUMMARY) {
        Some((i, _)) => format!("{}…", &flat[..i]),
        None => flat,
    }
}

fn cap(text: &str) -> String {
    if text.len() <= MAX_OUTPUT {
        return text.to_owned();
    }
    let mut end = MAX_OUTPUT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (cut)", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lines: &[&str], parse: fn(&str) -> Vec<ChatEvent>) -> Transcript {
        let mut t = Transcript::default();
        t.push_user("hi");
        for line in lines {
            for event in parse(line) {
                t.apply(event);
            }
        }
        t
    }

    // Trimmed from a real `claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages` run.
    const CLAUDE: &[&str] = &[
        r#"{"type":"system","subtype":"hook_started","hook_name":"SessionStart:startup","session_id":"s1"}"#,
        r#"{"type":"system","subtype":"init","cwd":"C:\\x","session_id":"0de9e883","tools":["Bash"]}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"Bash","input":{}}},"session_id":"0de9e883","parent_tool_use_id":null}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"echo hi","description":"Print hi"}}]},"parent_tool_use_id":null}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_1","type":"tool_result","content":"hi","is_error":false}]},"parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"do"}},"parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"n"}},"parent_tool_use_id":null}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"done"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"0de9e883"}"#,
    ];

    #[test]
    fn claude_turn() {
        let t = run(CLAUDE, parse_claude);
        assert_eq!(t.session.as_deref(), Some("0de9e883"));
        assert!(!t.busy);
        assert_eq!(
            t.items,
            vec![
                Item::User("hi".into()),
                Item::Tool(Tool {
                    id: "toolu_1".into(),
                    name: "Bash".into(),
                    input: "echo hi".into(),
                    output: Some("hi".into()),
                    error: false
                }),
                // Deltas streamed "don", the final block replaced them.
                Item::Assistant("done".into()),
            ]
        );
    }

    #[test]
    fn claude_streaming_then_error() {
        let mut t = Transcript::default();
        t.push_user("x");
        for e in parse_claude(CLAUDE[5]).into_iter().chain(parse_claude(CLAUDE[6])) {
            t.apply(e);
        }
        assert_eq!(t.items.last(), Some(&Item::Assistant("don".into())));
        assert!(t.busy);
        let failed = r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#;
        for e in parse_claude(failed) {
            t.apply(e);
        }
        assert_eq!(t.items.last(), Some(&Item::Error("error_during_execution".into())));
        assert!(!t.busy);
    }

    #[test]
    fn claude_ignores_subagents_and_junk() {
        let sub = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"inner"}]},"parent_tool_use_id":"toolu_9"}"#;
        assert!(parse_claude(sub).is_empty());
        assert!(parse_claude("not json").is_empty());
        assert!(parse_claude("").is_empty());
    }

    #[test]
    fn claude_input_lines() {
        let v: Value = serde_json::from_str(&claude_user_line("a \"quoted\"\nline")).unwrap();
        assert_eq!(v["message"]["content"], "a \"quoted\"\nline");
        let v: Value = serde_json::from_str(&claude_interrupt_line("r1")).unwrap();
        assert_eq!(v["request"]["subtype"], "interrupt");
    }

    // From a real `codex exec --json` run (codex-cli 0.167).
    const CODEX: &[&str] = &[
        r#"{"type":"thread.started","thread_id":"01a1d0d8-79a5"}"#,
        r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Codex is ignoring 1 unrecognized configuration setting."}}"#,
        r#"{"type":"turn.started"}"#,
        r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"I'll run the command now.\n"}}"#,
        r#"{"type":"item.started","item":{"id":"item_3","type":"command_execution","command":"powershell.exe -Command 'echo hi'","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#,
        r#"{"type":"item.completed","item":{"id":"item_3","type":"command_execution","command":"powershell.exe -Command 'echo hi'","aggregated_output":"hi\r\n","exit_code":0,"status":"completed"}}"#,
        r#"{"type":"item.completed","item":{"id":"item_4","type":"agent_message","text":"done"}}"#,
        r#"{"type":"turn.completed","usage":{"input_tokens":40084,"output_tokens":46}}"#,
    ];

    #[test]
    fn codex_turn() {
        let t = run(CODEX, parse_codex);
        assert_eq!(t.session.as_deref(), Some("01a1d0d8-79a5"));
        assert!(!t.busy);
        assert_eq!(t.items.len(), 5, "{:?}", t.items);
        assert!(matches!(&t.items[1], Item::Notice(n) if n.starts_with("Codex is ignoring")));
        assert_eq!(t.items[2], Item::Assistant("I'll run the command now.\n".into()));
        assert_eq!(
            t.items[3],
            Item::Tool(Tool {
                id: "item_3".into(),
                name: "shell".into(),
                input: "powershell.exe -Command 'echo hi'".into(),
                output: Some("hi\r\n".into()),
                error: false
            })
        );
        assert_eq!(t.last_answer(), Some("done"));
    }

    #[test]
    fn codex_failures() {
        let failed_cmd = r#"{"type":"item.completed","item":{"id":"i","type":"command_execution","command":"false","aggregated_output":"","exit_code":1,"status":"failed"}}"#;
        assert!(matches!(parse_codex(failed_cmd).last(), Some(ChatEvent::ToolResult { error: true, .. })));
        let edit = r#"{"type":"item.completed","item":{"id":"f","type":"file_change","changes":[{"path":"src/a.rs","kind":"update"}],"status":"completed"}}"#;
        assert!(matches!(&parse_codex(edit)[0], ChatEvent::ToolStart { name, input, .. } if name == "edit" && input == "src/a.rs"));
        let turn = r#"{"type":"turn.failed","error":{"message":"stream disconnected"}}"#;
        assert_eq!(parse_codex(turn), vec![ChatEvent::TurnEnd { error: Some("stream disconnected".into()) }]);
    }

    #[test]
    fn long_inputs_are_cut() {
        let long = "x".repeat(500);
        assert!(summarize_input(&json!({"command": long})).chars().count() <= MAX_SUMMARY + 1);
        assert!(cap(&"é".repeat(MAX_OUTPUT)).ends_with("(cut)"));
    }
}
