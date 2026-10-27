//! Browser panel for a Claude pane: a child webview of the main window, laid over its dock panel by the UI,
//! that the agent drives through chrome-devtools-mcp over the DevTools protocol (navigate, click, fill,
//! screenshot, console, network). Each pane gets its own webview, profile and debugging port.
//! Ref: https://github.com/ChromeDevTools/chrome-devtools-mcp
//!
//! shortcut: the debugging port listens on 127.0.0.1 without auth, so a local process could drive this
//! webview (not the app UI: separate WebView2 environment). Fine for a dev tool on a personal machine.

use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::{LazyLock, Mutex, PoisonError};

use serde_json::{Value, json};
use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl};

const START_URL: &str = "https://www.google.com";
const MAIN_WINDOW: &str = "main";
/// wry's defaults, lost when browser args are set.
const WRY_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

/// pane id -> debugging port of its browser webview.
static PORTS: LazyLock<Mutex<HashMap<String, u16>>> = LazyLock::new(Default::default);

fn label(pane_id: &str) -> String {
    format!("browser-{pane_id}")
}

fn valid_id(pane_id: &str) -> Result<(), String> {
    let ok = (1..=64).contains(&pane_id.len())
        && pane_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    ok.then_some(()).ok_or_else(|| "invalid pane id".into())
}

fn webview(app: &AppHandle, pane_id: &str) -> Result<Webview, String> {
    app.get_webview(&label(pane_id))
        .ok_or_else(|| "no browser for this agent".into())
}

/// Creates (or reuses) the pane's browser webview, hidden until the UI places it, and returns the Claude
/// args that connect the agent to it. Async: creating a webview from a sync command deadlocks on Windows.
#[tauri::command]
pub async fn browser_open(app: AppHandle, pane_id: String) -> Result<Vec<String>, String> {
    valid_id(&pane_id)?;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("browser");
    let known = PORTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&pane_id)
        .copied();
    let port = match known.filter(|_| app.get_webview(&label(&pane_id)).is_some()) {
        Some(port) => port,
        None => {
            let port = free_port()?;
            let window = app.get_window(MAIN_WINDOW).ok_or("main window not found")?;
            let builder = WebviewBuilder::new(
                label(&pane_id),
                WebviewUrl::External(START_URL.parse().expect("static url")),
            )
            // Different browser args need their own WebView2 environment, hence data directory.
            .data_directory(dir.join("profiles").join(&pane_id))
            .additional_browser_args(&format!("{WRY_ARGS} --remote-debugging-port={port}"));
            let child = window
                .add_child(
                    builder,
                    LogicalPosition::new(0.0, 0.0),
                    LogicalSize::new(1.0, 1.0),
                )
                .map_err(|e| format!("cannot open the browser: {e}"))?;
            let _ = child.hide();
            PORTS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(pane_id.clone(), port);
            port
        }
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let config = dir.join(format!("{pane_id}.json"));
    let text = serde_json::to_string_pretty(&config_for(port)).unwrap_or_default();
    std::fs::write(&config, text).map_err(|e| format!("{}: {e}", config.display()))?;
    Ok(vec![
        "--mcp-config".into(),
        config.to_string_lossy().into_owned(),
    ])
}

/// Lays the webview over its dock panel (CSS pixels of the main window), or hides it.
#[tauri::command]
pub fn browser_bounds(
    app: AppHandle,
    pane_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
) -> Result<(), String> {
    let view = webview(&app, &pane_id)?;
    if !visible || width < 1.0 || height < 1.0 {
        return view.hide().map_err(|e| e.to_string());
    }
    view.set_position(LogicalPosition::new(x, y))
        .and_then(|()| view.set_size(LogicalSize::new(width, height)))
        .and_then(|()| view.show())
        .map_err(|e| e.to_string())
}

/// Header controls: "back", "forward", "reload", or an http(s) URL to open.
#[tauri::command]
pub fn browser_go(app: AppHandle, pane_id: String, to: String) -> Result<(), String> {
    let view = webview(&app, &pane_id)?;
    let result = match to.as_str() {
        "back" => view.eval("history.back()"),
        "forward" => view.eval("history.forward()"),
        "reload" => view.reload(),
        _ => view.navigate(parse_url(&to)?),
    };
    result.map_err(|e| e.to_string())
}

/// Current URL of the pane's browser, for the header.
#[tauri::command]
pub fn browser_url(app: AppHandle, pane_id: String) -> Result<String, String> {
    let url = webview(&app, &pane_id)?.url().map_err(|e| e.to_string())?;
    Ok(url.to_string())
}

/// Closes the pane's browser for good (pane closed or browser turned off).
#[tauri::command]
pub fn browser_close(app: AppHandle, pane_id: String) {
    PORTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&pane_id);
    if let Some(view) = app.get_webview(&label(&pane_id)) {
        let _ = view.close();
    }
}

/// "example.com" or "https://example.com/x"; only http(s).
fn parse_url(text: &str) -> Result<Url, String> {
    let text = text.trim();
    let full = if text.contains("://") {
        text.to_owned()
    } else {
        format!("https://{text}")
    };
    let url = Url::parse(&full).map_err(|_| format!("not a URL: {text}"))?;
    if matches!(url.scheme(), "http" | "https") {
        Ok(url)
    } else {
        Err("only http and https URLs".into())
    }
}

fn free_port() -> Result<u16, String> {
    // shortcut: the port is released before WebView2 binds it; a race is unlikely on 127.0.0.1.
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("no free port: {e}"))
}

fn config_for(port: u16) -> Value {
    let mut args = vec![
        "-y".to_string(),
        "chrome-devtools-mcp@latest".into(),
        "--browserUrl".into(),
        format!("http://127.0.0.1:{port}"),
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
    fn config_connects_to_the_webview_port() {
        let c = config_for(9333);
        let args: Vec<&str> = c["mcpServers"]["browser"]["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        assert!(args.ends_with(&[
            "chrome-devtools-mcp@latest",
            "--browserUrl",
            "http://127.0.0.1:9333"
        ]));
        assert!(valid_id("a-1").is_ok());
        assert!(valid_id("a b").is_err());
    }

    #[test]
    fn urls_default_to_https_and_refuse_other_schemes() {
        assert_eq!(
            parse_url("example.com").unwrap().as_str(),
            "https://example.com/"
        );
        assert!(parse_url("http://localhost:3000").is_ok());
        assert!(parse_url("file:///c:/x").is_err());
        assert!(parse_url("javascript:alert(1)").is_err());
    }
}
