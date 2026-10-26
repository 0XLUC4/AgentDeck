//! PTY management: one pseudo-terminal per pane, output batched to the frontend.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tauri::{AppHandle, Emitter, State};

use crate::agents::{self, AgentKind};
use crate::AppState;

/// Output is flushed to the webview at most every 16 ms (~60 fps).
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);

pub struct Pane {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

#[derive(Default)]
pub struct Panes(pub Mutex<HashMap<String, Pane>>);

impl Panes {
    /// Writes raw bytes to a pane's PTY. Used by commands and by the voice module.
    pub fn write(&self, pane_id: &str, data: &[u8]) -> Result<(), String> {
        let mut panes = self.0.lock().map_err(|e| e.to_string())?;
        let pane = panes.get_mut(pane_id).ok_or("pane not found")?;
        pane.writer.write_all(data).map_err(|e| e.to_string())?;
        pane.writer.flush().map_err(|e| e.to_string())
    }
}

/// Spawns an agent (or a plain shell) in a new PTY under `pane_id`.
/// The frontend picks the id so it can subscribe to output before spawning.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn spawn_pane(
    app: AppHandle,
    state: State<'_, AppState>,
    pane_id: String,
    agent: AgentKind,
    cwd: String,
    args: Vec<String>,
    worktree: bool,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    if state.panes.0.lock().map_err(|e| e.to_string())?.contains_key(&pane_id) {
        return Err("pane id already in use".into());
    }
    let cwd = if worktree {
        agents::create_worktree(&cwd, &pane_id)?
    } else {
        cwd
    };
    agents::prepare_hooks(agent, &cwd, &state.hooks)?;

    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())?;

    let args = [agents::hook_args(agent, &pane_id, &state.hooks), args].concat();
    let mut cmd: CommandBuilder = agents::command_for(agent, &args);
    cmd.cwd(&cwd);
    cmd.env("AGENTDECK_PANE_ID", &pane_id);
    cmd.env("AGENTDECK_HOOK_URL", state.hooks.url());
    cmd.env("AGENTDECK_HOOK_TOKEN", &state.hooks.token);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");

    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave); // child owns it now; dropping lets EOF propagate on exit
    let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    stream_output(app, pane_id.clone(), reader);

    state.panes.0.lock().map_err(|e| e.to_string())?.insert(
        pane_id.clone(),
        Pane { master: pair.master, writer, child },
    );
    Ok(())
}

#[tauri::command]
pub fn write_to_pane(state: State<'_, AppState>, pane_id: String, data: String) -> Result<(), String> {
    state.panes.write(&pane_id, data.as_bytes())
}

#[tauri::command]
pub fn resize_pane(state: State<'_, AppState>, pane_id: String, cols: u16, rows: u16) -> Result<(), String> {
    let panes = state.panes.0.lock().map_err(|e| e.to_string())?;
    let pane = panes.get(&pane_id).ok_or("pane not found")?;
    pane.master
        .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn kill_pane(state: State<'_, AppState>, pane_id: String) -> Result<(), String> {
    let pane = state.panes.0.lock().map_err(|e| e.to_string())?.remove(&pane_id);
    if let Some(mut pane) = pane {
        // Already-exited children return an error here; that's fine.
        let _ = pane.child.kill();
    }
    Ok(())
}

/// Marks which pane receives voice transcripts.
#[tauri::command]
pub fn set_active_pane(state: State<'_, AppState>, pane_id: Option<String>) -> Result<(), String> {
    *state.active_pane.lock().map_err(|e| e.to_string())? = pane_id;
    Ok(())
}

/// Reader thread -> batching thread -> `pane-output:{id}` events, then `pane-exit:{id}`.
fn stream_output(app: AppHandle, pane_id: String, mut reader: Box<dyn Read + Send>) {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();

    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    std::thread::spawn(move || {
        let output_event = format!("pane-output:{pane_id}");
        let mut pending: Vec<u8> = Vec::new();
        let mut last_flush = Instant::now();
        loop {
            let closed = match rx.recv_timeout(FLUSH_INTERVAL) {
                Ok(chunk) => {
                    pending.extend_from_slice(&chunk);
                    false
                }
                Err(RecvTimeoutError::Timeout) => false,
                Err(RecvTimeoutError::Disconnected) => true,
            };
            if !pending.is_empty() && (closed || last_flush.elapsed() >= FLUSH_INTERVAL) {
                let text = take_valid_utf8(&mut pending, closed);
                if !text.is_empty() {
                    let _ = app.emit(&output_event, text);
                }
                last_flush = Instant::now();
            }
            if closed {
                let _ = app.emit(&format!("pane-exit:{pane_id}"), ());
                break;
            }
        }
    });
}

/// Drains the longest valid UTF-8 prefix, keeping an incomplete trailing
/// multi-byte sequence for the next chunk (unless `flush_all`).
fn take_valid_utf8(pending: &mut Vec<u8>, flush_all: bool) -> String {
    let cut = match std::str::from_utf8(pending) {
        Ok(_) => pending.len(),
        // error_len() == None means "truncated sequence at the end".
        Err(e) if e.error_len().is_none() && !flush_all => e.valid_up_to(),
        Err(_) => pending.len(),
    };
    let rest = pending.split_off(cut);
    let text = String::from_utf8_lossy(pending).into_owned();
    *pending = rest;
    text
}

#[cfg(test)]
mod tests {
    use super::take_valid_utf8;

    #[test]
    fn keeps_truncated_multibyte_tail() {
        let mut buf = vec![b'a', 0xC3]; // 'a' + first byte of 'é'
        assert_eq!(take_valid_utf8(&mut buf, false), "a");
        assert_eq!(buf, vec![0xC3]);
        buf.push(0xA9);
        assert_eq!(take_valid_utf8(&mut buf, false), "é");
        assert!(buf.is_empty());
    }

    #[test]
    fn invalid_bytes_are_replaced_not_stuck() {
        let mut buf = vec![b'x', 0xFF, b'y'];
        assert_eq!(take_valid_utf8(&mut buf, false), "x\u{FFFD}y");
        assert!(buf.is_empty());
    }
}
