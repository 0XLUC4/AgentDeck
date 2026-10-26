//! Local HTTP server (127.0.0.1 only, secret token) receiving agent hook events
//! and re-emitting them to the UI as `agent-status`.

use serde::Serialize;
use tauri::AppHandle;

pub struct HookServer {
    pub port: u16,
    pub token: String,
}

impl HookServer {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/hook", self.port)
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneStatus {
    Idle,
    Working,
    Waiting,
    Done,
    Error,
}

/// Payload of the `agent-status` event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatusEvent {
    pub pane_id: String,
    pub status: PaneStatus,
    pub last_action: Option<String>,
    /// Raw hook name, e.g. "PreToolUse".
    pub event: String,
}

/// Starts the hook server on a background thread.
pub fn start(_app: AppHandle) -> HookServer {
    // TODO(team-backend): bind tiny_http on 127.0.0.1:0, check token, emit `agent-status`.
    HookServer { port: 0, token: uuid::Uuid::new_v4().to_string() }
}
