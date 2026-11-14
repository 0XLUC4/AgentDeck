//! Keeps every agent on AgentDeck's browser panel. Claude Code has a blocking hook
//! (`hooks_server::launch_denied`); Codex, Gemini and OpenCode have none, so the other browser MCP
//! servers they have configured (a Chrome DevTools, Playwright or Puppeteer server...) are switched off
//! for the agents AgentDeck starts, through each one's own override channel. Their config files are
//! only read, never edited: the same agents started outside AgentDeck keep their servers.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// A server or tool name that drives a browser window of its own.
pub fn names_a_browser(name: &str) -> bool {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)chrome|playwright|puppeteer|selenium|browser|computer[-_]use|webview")
            .expect("static regex")
    });
    RE.is_match(name)
}

/// An MCP server other than AgentDeck's own `browser` that drives a browser.
pub fn other_browser_server(name: &str) -> bool {
    name != "browser" && names_a_browser(name)
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

fn json_keys(path: &Path, key: &str) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    // shortcut: JSONC (comments) is not read, its servers stay on; parse it if one shows up.
    let Ok(value) = serde_json::from_str::<Value>(&text) else { return Vec::new() };
    value[key].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default()
}

/// Codex: `-c mcp_servers.<name>.enabled=false` for each browser server of `~/.codex/config.toml`
/// (or `$CODEX_HOME`) and the project's `.codex/config.toml`.
pub fn codex_args(cwd: &Path) -> Vec<String> {
    let codex_home = std::env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".codex")));
    let files = codex_home.into_iter().chain([cwd.join(".codex")]).map(|dir| dir.join("config.toml"));
    let mut names: Vec<String> = Vec::new();
    for file in files {
        let Ok(doc) = std::fs::read_to_string(&file).unwrap_or_default().parse::<toml_edit::DocumentMut>() else {
            continue;
        };
        if let Some(servers) = doc.get("mcp_servers").and_then(|s| s.as_table_like()) {
            names.extend(servers.iter().map(|(name, _)| name.to_owned()));
        }
    }
    names.sort();
    names.dedup();
    names
        .into_iter()
        // Bare TOML keys only: they pass cmd.exe and the npm shim unquoted (see `hook_args`).
        .filter(|n| other_browser_server(n) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .flat_map(|n| ["-c".to_owned(), format!("mcp_servers.{n}.enabled=false")])
        .collect()
}

/// Gemini: the browser servers of `~/.gemini/settings.json`, for `mcp.excluded` of AgentDeck's
/// system defaults layer.
pub fn gemini_excluded() -> Vec<String> {
    home()
        .map(|h| json_keys(&h.join(".gemini").join("settings.json"), "mcpServers"))
        .unwrap_or_default()
        .into_iter()
        .filter(|n| other_browser_server(n))
        .collect()
}

/// OpenCode: `{"<name>": {"enabled": false}}` for each browser server of its global config, merged
/// into the `mcp` of `OPENCODE_CONFIG_CONTENT`.
pub fn opencode_off() -> Map<String, Value> {
    let Some(home) = home() else { return Map::new() };
    let dir = home.join(".config").join("opencode");
    ["opencode.json", "config.json"]
        .iter()
        .flat_map(|file| json_keys(&dir.join(file), "mcp"))
        .filter(|n| other_browser_server(n))
        .map(|n| (n, json!({ "enabled": false })))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_other_browser_servers() {
        for name in ["chrome-devtools", "playwright", "puppeteer", "plugin_ecc_chrome-devtools", "browserbase", "computer-use"] {
            assert!(other_browser_server(name), "{name}");
        }
        for name in ["browser", "github", "context7", "filesystem"] {
            assert!(!other_browser_server(name), "{name}");
        }
    }

    #[test]
    fn codex_turns_off_project_browser_servers() {
        let dir = std::env::temp_dir().join(format!("agentdeck-guard-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join(".codex")).unwrap();
        std::fs::write(
            dir.join(".codex/config.toml"),
            "[mcp_servers.playwright]\ncommand = \"npx\"\n\n[mcp_servers.github]\ncommand = \"gh\"\n\n[mcp_servers.\"odd name chrome\"]\ncommand = \"x\"\n",
        )
        .unwrap();
        let args = codex_args(&dir);
        assert!(args.windows(2).any(|w| w == ["-c", "mcp_servers.playwright.enabled=false"]), "{args:?}");
        assert!(!args.iter().any(|a| a.contains("github") || a.contains("odd")), "{args:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
