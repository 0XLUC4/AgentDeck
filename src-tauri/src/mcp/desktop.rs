//! Desktop control MCP server at `/desktop`: screenshots of the primary screen, mouse and keyboard.
//! One pane at a time holds it (the screen, mouse and keyboard are shared): registering a pane revokes
//! the previous one. Every call still goes through the agent's own permission prompt.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, State};
use tiny_http::{Request, Response};
use xcap::Monitor;
use xcap::image::codecs::png::PngEncoder;
use xcap::image::imageops::{FilterType, resize};
use xcap::image::{ExtendedColorType, ImageEncoder};

use super::{Registration, json_reply, lock, parse, rpc};
use crate::AppState;
use crate::hooks_server::{PANE_HEADER, TOKEN_HEADER};

const SECRET_HEADER: &str = "X-AgentDeck-Desktop";
/// Screenshots are scaled down to fit this box: smaller images, and the coordinates the model sends back.
const MAX_W: u32 = 1366;
const MAX_H: u32 = 768;
const MAX_TEXT: usize = 4096;

/// The pane holding the desktop -> its secret. At most one entry.
static HOLDER: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

/// Gives the desktop to `pane_id` (revoking any other pane) and writes its MCP config.
#[tauri::command]
pub fn desktop_register(
    app: AppHandle,
    state: State<'_, AppState>,
    pane_id: String,
) -> Result<Registration, String> {
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
        .join("desktop");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let config = dir.join("mcp.json");
    let text = serde_json::to_string_pretty(&config_for(state.hooks.port)).unwrap_or_default();
    std::fs::write(&config, text).map_err(|e| format!("{}: {e}", config.display()))?;
    let secret = uuid::Uuid::new_v4().to_string();
    let mut holder = lock(&HOLDER);
    holder.clear();
    holder.insert(pane_id, secret.clone());
    let args = vec!["--mcp-config".into(), config.to_string_lossy().into_owned()];
    Ok(Registration { args, secret })
}

/// Takes the desktop back from `pane_id` (no-op if another pane holds it).
#[tauri::command]
pub fn desktop_release(pane_id: String) {
    lock(&HOLDER).remove(&pane_id);
}

/// Token, pane id and secret are expanded by Claude Code from the pane's env: the file holds no secret.
fn config_for(port: u16) -> Value {
    json!({ "mcpServers": { "desktop": {
        "type": "http",
        "url": format!("http://127.0.0.1:{port}/desktop"),
        "headers": {
            TOKEN_HEADER: "${AGENTDECK_HOOK_TOKEN}",
            PANE_HEADER: "${AGENTDECK_PANE_ID}",
            SECRET_HEADER: "${AGENTDECK_DESKTOP_SECRET}",
        },
    }}})
}

/// Answers one HTTP request on `/desktop`.
pub fn serve(app: &AppHandle, mut req: Request, token: &str) {
    let reply = match parse(&mut req, token, SECRET_HEADER, &HOLDER) {
        Err(code) => Response::empty(code).boxed(),
        Ok((pane_id, msg)) => {
            let alive = app
                .try_state::<AppState>()
                .is_some_and(|s| s.panes.0.lock().is_ok_and(|p| p.contains_key(&pane_id)));
            if !alive {
                lock(&HOLDER).remove(&pane_id);
                Response::empty(403).boxed()
            } else {
                match handle(&msg) {
                    Some(body) => json_reply(200, &body),
                    None => Response::empty(202).boxed(),
                }
            }
        }
    };
    let _ = req.respond(reply);
}

fn handle(msg: &Value) -> Option<Value> {
    let (Some(id), Some(method)) = (msg.get("id"), msg.get("method")) else {
        return None;
    };
    let result = match method.as_str().unwrap_or("") {
        "initialize" => {
            let asked = msg["params"]["protocolVersion"].as_str().unwrap_or("");
            let version = rpc::VERSIONS
                .iter()
                .find(|v| **v == asked)
                .unwrap_or(&rpc::VERSIONS[1]);
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "agentdeck-desktop", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Control this computer's screen. Take a screenshot first; \
                    coordinates are in screenshot pixels. Take another screenshot to check each step.",
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": definitions() }),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or("");
            let args = &msg["params"]["arguments"];
            // Tool failures are results with isError so the model can read and recover from them.
            match call(name, args) {
                Ok(content) => json!({ "content": content }),
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            }
        }
        other => {
            return Some(rpc::error(
                id.clone(),
                -32601,
                &format!("Method not found: {other}"),
            ));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn definitions() -> Value {
    let int = |d: &str| json!({ "type": "integer", "description": d });
    let tool = |name: &str, description: &str, properties: Value, required: &[&str]| {
        json!({
            "name": name,
            "description": description,
            "inputSchema": { "type": "object", "properties": properties, "required": required },
        })
    };
    json!([
        tool(
            "screenshot",
            "Screenshot of the primary screen (scaled down).",
            json!({}),
            &[]
        ),
        tool(
            "click",
            "Click at a point of the last screenshot.",
            json!({
                "x": int("x in screenshot pixels"),
                "y": int("y in screenshot pixels"),
                "button": { "type": "string", "enum": ["left", "right", "middle"] },
                "double": { "type": "boolean" },
            }),
            &["x", "y"],
        ),
        tool(
            "type_text",
            "Type text at the focused field.",
            json!({ "text": { "type": "string" } }),
            &["text"],
        ),
        tool(
            "press_keys",
            "Press a key or a shortcut, e.g. \"enter\", \"ctrl+s\", \"alt+tab\", \"win+r\".",
            json!({ "keys": { "type": "string" } }),
            &["keys"],
        ),
        tool(
            "scroll",
            "Scroll at a point; positive amount scrolls down.",
            json!({ "x": int("x in screenshot pixels"), "y": int("y in screenshot pixels"), "amount": int("-20 to 20") }),
            &["x", "y", "amount"],
        ),
    ])
}

fn call(name: &str, args: &Value) -> Result<Value, String> {
    let ok = |text: &str| Ok(json!([{ "type": "text", "text": text }]));
    match name {
        "screenshot" => screenshot(),
        "click" => {
            let button = match args["button"].as_str().unwrap_or("left") {
                "left" => Button::Left,
                "right" => Button::Right,
                "middle" => Button::Middle,
                other => return Err(format!("unknown button {other}")),
            };
            let mut input = enigo()?;
            move_to(&mut input, args)?;
            let times = if args["double"] == true { 2 } else { 1 };
            for _ in 0..times {
                input
                    .button(button, Direction::Click)
                    .map_err(|e| e.to_string())?;
            }
            ok("clicked")
        }
        "type_text" => {
            let text = args["text"].as_str().unwrap_or("");
            if text.is_empty() || text.len() > MAX_TEXT {
                return Err(format!("text: 1 to {MAX_TEXT} bytes"));
            }
            enigo()?.text(text).map_err(|e| e.to_string())?;
            ok("typed")
        }
        "press_keys" => {
            let keys = parse_keys(args["keys"].as_str().unwrap_or(""))?;
            let mut input = enigo()?;
            let (last, modifiers) = keys.split_last().ok_or("no key")?;
            let mut result = Ok(());
            for key in modifiers {
                result = result.and(input.key(*key, Direction::Press));
            }
            result = result.and(input.key(*last, Direction::Click));
            // Always release modifiers, even after a failure, so no key stays held down.
            for key in modifiers.iter().rev() {
                let _ = input.key(*key, Direction::Release);
            }
            result.map_err(|e| e.to_string())?;
            ok("pressed")
        }
        "scroll" => {
            let amount = args["amount"].as_i64().ok_or("amount: integer")?;
            if !(-20..=20).contains(&amount) {
                return Err("amount: -20 to 20".into());
            }
            let mut input = enigo()?;
            move_to(&mut input, args)?;
            input
                .scroll(amount as i32, Axis::Vertical)
                .map_err(|e| e.to_string())?;
            ok("scrolled")
        }
        _ => Err(format!("Unknown tool: {name}")),
    }
}

fn enigo() -> Result<Enigo, String> {
    Enigo::new(&Settings::default()).map_err(|e| format!("input unavailable: {e}"))
}

struct Screen {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn primary() -> Result<(Monitor, Screen), String> {
    let monitor = Monitor::all()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .ok_or("no primary screen")?;
    let err = |e: xcap::XCapError| e.to_string();
    let screen = Screen {
        x: monitor.x().map_err(err)?,
        y: monitor.y().map_err(err)?,
        width: monitor.width().map_err(err)?,
        height: monitor.height().map_err(err)?,
    };
    Ok((monitor, screen))
}

/// Screenshot size for a screen: fits MAX_W x MAX_H, never upscaled.
fn scaled(width: u32, height: u32) -> (u32, u32) {
    let ratio = f64::min(
        1.0,
        f64::min(
            f64::from(MAX_W) / f64::from(width),
            f64::from(MAX_H) / f64::from(height),
        ),
    );
    let fit = |v: u32| ((f64::from(v) * ratio).round() as u32).max(1);
    (fit(width), fit(height))
}

/// Screenshot pixel -> screen pixel; errors outside the screenshot.
fn to_screen(screen: &Screen, x: i64, y: i64) -> Result<(i32, i32), String> {
    let (w, h) = scaled(screen.width, screen.height);
    if !(0..i64::from(w)).contains(&x) || !(0..i64::from(h)).contains(&y) {
        return Err(format!("point outside the {w}x{h} screenshot"));
    }
    let map = |v: i64, from: u32, to: u32| (v as f64 * f64::from(to) / f64::from(from)) as i32;
    Ok((
        screen.x + map(x, w, screen.width),
        screen.y + map(y, h, screen.height),
    ))
}

fn move_to(input: &mut Enigo, args: &Value) -> Result<(), String> {
    let (x, y) = (args["x"].as_i64(), args["y"].as_i64());
    let (Some(x), Some(y)) = (x, y) else {
        return Err("x and y: integers".into());
    };
    let (_, screen) = primary()?;
    let (sx, sy) = to_screen(&screen, x, y)?;
    input
        .move_mouse(sx, sy, Coordinate::Abs)
        .map_err(|e| e.to_string())
}

fn screenshot() -> Result<Value, String> {
    let (monitor, screen) = primary()?;
    let image = monitor.capture_image().map_err(|e| e.to_string())?;
    let (w, h) = scaled(screen.width, screen.height);
    let image = resize(&image, w, h, FilterType::Triangle);
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(image.as_raw(), w, h, ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok(json!([
        { "type": "image", "mimeType": "image/png", "data": base64(&png) },
        { "type": "text", "text": format!("{w}x{h} screenshot; click coordinates use this size.") },
    ]))
}

fn parse_keys(combo: &str) -> Result<Vec<Key>, String> {
    if combo.is_empty() || combo.len() > 64 {
        return Err("keys: 1 to 64 characters".into());
    }
    combo
        .split('+')
        .map(|name| {
            let lower = name.trim().to_lowercase();
            Ok(match lower.as_str() {
                "ctrl" | "control" => Key::Control,
                "shift" => Key::Shift,
                "alt" => Key::Alt,
                "win" | "meta" | "super" | "cmd" => Key::Meta,
                "enter" | "return" => Key::Return,
                "tab" => Key::Tab,
                "esc" | "escape" => Key::Escape,
                "backspace" => Key::Backspace,
                "delete" | "del" => Key::Delete,
                "space" => Key::Space,
                "up" => Key::UpArrow,
                "down" => Key::DownArrow,
                "left" => Key::LeftArrow,
                "right" => Key::RightArrow,
                "home" => Key::Home,
                "end" => Key::End,
                "pageup" => Key::PageUp,
                "pagedown" => Key::PageDown,
                f if f.len() > 1 && f.starts_with('f') => match f[1..].parse::<u8>() {
                    Ok(n @ 1..=12) => [
                        Key::F1,
                        Key::F2,
                        Key::F3,
                        Key::F4,
                        Key::F5,
                        Key::F6,
                        Key::F7,
                        Key::F8,
                        Key::F9,
                        Key::F10,
                        Key::F11,
                        Key::F12,
                    ][usize::from(n - 1)],
                    _ => return Err(format!("unknown key {name}")),
                },
                _ => {
                    let mut chars = lower.chars();
                    match (chars.next(), chars.next()) {
                        (Some(c), None) => Key::Unicode(c),
                        _ => return Err(format!("unknown key {name}")),
                    }
                }
            })
        })
        .collect()
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                char::from(ABC[(n >> (18 - 6 * i) & 63) as usize])
            } else {
                '='
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn screenshot_coordinates_map_back_to_the_screen() {
        let screen = Screen {
            x: 0,
            y: 0,
            width: 2732,
            height: 1536,
        };
        assert_eq!(scaled(2732, 1536), (1366, 768));
        assert_eq!(scaled(800, 600), (800, 600));
        assert_eq!(to_screen(&screen, 683, 384), Ok((1366, 768)));
        assert!(to_screen(&screen, 1366, 0).is_err());
        assert!(to_screen(&screen, -1, 0).is_err());
    }

    #[test]
    fn parses_shortcuts() {
        assert_eq!(
            parse_keys("ctrl+shift+s"),
            Ok(vec![Key::Control, Key::Shift, Key::Unicode('s')])
        );
        assert_eq!(parse_keys("Enter"), Ok(vec![Key::Return]));
        assert_eq!(parse_keys("f5"), Ok(vec![Key::F5]));
        assert!(parse_keys("f13").is_err());
        assert!(parse_keys("ctrl+bogus").is_err());
        assert!(parse_keys("").is_err());
    }

    #[test]
    #[ignore = "captures the real screen"]
    fn captures_the_primary_screen() {
        let content = screenshot().unwrap();
        assert!(content[0]["data"].as_str().unwrap().len() > 1000);
        println!("{}", content[1]["text"]);
    }

    #[test]
    fn config_has_no_secret() {
        let c = config_for(47800);
        let server = &c["mcpServers"]["desktop"];
        assert_eq!(server["url"], "http://127.0.0.1:47800/desktop");
        assert_eq!(
            server["headers"][SECRET_HEADER],
            "${AGENTDECK_DESKTOP_SECRET}"
        );
    }
}
