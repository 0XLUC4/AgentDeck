//! Agent definitions (Claude Code, Codex, Gemini CLI), install detection,
//! git worktrees and hook configuration.

use portable_pty::CommandBuilder;
use serde::{Deserialize, Serialize};

use crate::hooks_server::HookServer;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
    Gemini,
    Shell,
}

impl AgentKind {
    /// CLI binary name; `None` for a plain shell.
    pub fn binary(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("claude"),
            Self::Codex => Some("codex"),
            Self::Gemini => Some("gemini"),
            Self::Shell => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub kind: AgentKind,
    pub installed: bool,
    pub path: Option<String>,
    pub install_url: Option<&'static str>,
}

/// Reports which agent CLIs are available on PATH.
#[tauri::command]
pub fn detect_agents() -> Vec<AgentInfo> {
    // TODO(team-backend): real PATH lookup.
    [AgentKind::Claude, AgentKind::Codex, AgentKind::Gemini, AgentKind::Shell]
        .into_iter()
        .map(|kind| AgentInfo { kind, installed: true, path: None, install_url: None })
        .collect()
}

/// Builds the command that launches `kind` inside a PTY.
pub fn command_for(kind: AgentKind, args: &[String]) -> CommandBuilder {
    let mut cmd = match (kind.binary(), cfg!(windows)) {
        // npm installs agents as .cmd shims on Windows, which need cmd.exe.
        (Some(bin), true) => {
            let mut c = CommandBuilder::new("cmd.exe");
            c.args(["/c", bin]);
            c
        }
        (Some(bin), false) => CommandBuilder::new(bin),
        (None, true) => CommandBuilder::new("powershell.exe"),
        (None, false) => CommandBuilder::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())),
    };
    cmd.args(args);
    cmd
}

/// Creates a dedicated git worktree for the pane and returns its path.
pub fn create_worktree(cwd: &str, _pane_id: &str) -> Result<String, String> {
    // TODO(team-backend): `git worktree add`.
    Ok(cwd.to_string())
}

/// Merges AgentDeck hooks into the agent's config for `cwd` without clobbering user config.
pub fn prepare_hooks(_kind: AgentKind, _cwd: &str, _hooks: &HookServer) -> Result<(), String> {
    // TODO(team-backend): Claude settings.local.json, Codex notify.
    Ok(())
}
