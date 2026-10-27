//! Browser control for a Claude pane: an extra MCP server (Playwright) that opens its own Chrome window.
//! Each pane gets its own Chrome profile, so logins stay per agent and never touch the user's Chrome.
//! Ref: https://github.com/microsoft/playwright-mcp

use std::path::Path;

use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

/// Claude Code args that add the `browser` MCP server for `pane_id`.
#[tauri::command]
pub fn browser_mcp_args(app: AppHandle, pane_id: String) -> Result<Vec<String>, String> {
    if pane_id.is_empty()
        || pane_id.len() > 64
        || !pane_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("invalid pane id".into());
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("browser");
    let profile = dir.join("profiles").join(&pane_id);
    std::fs::create_dir_all(&profile).map_err(|e| format!("{}: {e}", profile.display()))?;
    let config = dir.join(format!("{pane_id}.json"));
    let text = serde_json::to_string_pretty(&config_for(&profile)).unwrap_or_default();
    std::fs::write(&config, text).map_err(|e| format!("{}: {e}", config.display()))?;
    Ok(vec![
        "--mcp-config".into(),
        config.to_string_lossy().into_owned(),
    ])
}

fn config_for(profile: &Path) -> Value {
    let mut args = vec![
        "-y".to_string(),
        "@playwright/mcp@latest".into(),
        "--user-data-dir".into(),
        profile.to_string_lossy().into_owned(),
    ];
    // npx is a .cmd script on Windows: Claude Code can only start it through cmd.
    let command = if cfg!(windows) {
        args.splice(0..0, ["/c".to_string(), "npx".into()]);
        "cmd"
    } else {
        "npx"
    };
    json!({ "mcpServers": { "browser": { "command": command, "args": args } } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_runs_playwright_with_own_profile() {
        let c = config_for(Path::new("P"));
        let args = c["mcpServers"]["browser"]["args"].as_array().unwrap();
        let args: Vec<&str> = args.iter().map(|a| a.as_str().unwrap()).collect();
        assert!(args.ends_with(&["-y", "@playwright/mcp@latest", "--user-data-dir", "P"]));
    }
}
