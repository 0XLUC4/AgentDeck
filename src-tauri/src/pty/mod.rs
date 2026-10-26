//! PTY management: one pseudo-terminal per pane, output batched to the frontend.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::AppState;
use crate::agents::{self, AgentKind};

/// Output is flushed to the webview at most every 16 ms (~60 fps).
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
/// Max bytes per `pane-output` event; the rest waits for the next tick.
const MAX_EMIT_BYTES: usize = 256 * 1024;
/// Reader -> batcher queue depth (8 KB chunks): a full queue blocks the reader.
const QUEUE_CHUNKS: usize = 64;
/// ConPTY often keeps the output pipe open after the child exits: poll for exit.
const EXIT_POLL: Duration = Duration::from_millis(250);

/// Distinguishes successive processes spawned under the same pane id (restart).
static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

pub struct Pane {
    master: Box<dyn MasterPty + Send>,
    writer: SharedWriter,
    child: Box<dyn Child + Send + Sync>,
    run: u64,
}

#[derive(Default)]
pub struct Panes(pub Mutex<HashMap<String, Pane>>);

impl Panes {
    /// Writes raw bytes to a pane's PTY. Used by commands and by the voice module.
    /// The map lock is released before the (possibly blocking) write.
    pub fn write(&self, pane_id: &str, data: &[u8]) -> Result<(), String> {
        let writer = self
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .get(pane_id)
            .ok_or("pane not found")?
            .writer
            .clone();
        let mut writer = writer.lock().map_err(|e| e.to_string())?;
        writer.write_all(data).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())
    }
}

/// Spawns an agent (or a plain shell) in a new PTY under `pane_id`.
/// The frontend picks the id so it can subscribe to output before spawning.
#[tauri::command(async)]
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
    title: Option<String>,
    env: Option<HashMap<String, String>>,
) -> Result<(), String> {
    validate_spawn_input(&pane_id, &args)?;
    let env = env.unwrap_or_default();
    validate_extra_env(&env)?;
    if state
        .panes
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .contains_key(&pane_id)
    {
        return Err("pane id already in use".into());
    }
    let cwd = if worktree {
        agents::create_worktree(&cwd, &pane_id)?
    } else {
        cwd
    };
    agents::prepare_hooks(agent, &cwd, &state.hooks)?;

    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let args = [agents::hook_args(agent, &pane_id, &state.hooks), args].concat();
    let mut cmd: CommandBuilder = agents::command_for(agent, &args);
    cmd.cwd(&cwd);
    cmd.env("AGENTDECK_PANE_ID", &pane_id);
    cmd.env("AGENTDECK_HOOK_URL", state.hooks.url());
    cmd.env("AGENTDECK_HOOK_TOKEN", &state.hooks.token);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    for (key, value) in &env {
        cmd.env(key, value);
    }

    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave); // child owns it now; dropping lets EOF propagate on exit
    let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    let run = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
    // Insert before streaming so the exit handler always finds this run.
    state.panes.0.lock().map_err(|e| e.to_string())?.insert(
        pane_id.clone(),
        Pane {
            master: pair.master,
            writer: Arc::new(Mutex::new(writer)),
            child,
            run,
        },
    );
    watch_exit(app.clone(), pane_id.clone(), run);
    let agent_name = serde_json::to_value(agent).ok();
    let agent_name = agent_name.as_ref().and_then(|v| v.as_str()).unwrap_or("");
    let recorder = crate::history::Recorder::start(&pane_id, agent_name, &cwd, title);
    stream_output(app, pane_id, run, reader, recorder);
    Ok(())
}

#[tauri::command(async)]
pub fn write_to_pane(
    state: State<'_, AppState>,
    pane_id: String,
    data: String,
) -> Result<(), String> {
    state.panes.write(&pane_id, data.as_bytes())
}

#[tauri::command(async)]
pub fn resize_pane(
    state: State<'_, AppState>,
    pane_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let panes = state.panes.0.lock().map_err(|e| e.to_string())?;
    let pane = panes.get(&pane_id).ok_or("pane not found")?;
    pane.master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn kill_pane(state: State<'_, AppState>, pane_id: String) -> Result<(), String> {
    // Removed under the lock (temporary guard), killed and dropped outside it.
    let pane = state
        .panes
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&pane_id);
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

/// Polls the child; on exit frees the slot if `run` still owns it and drops the
/// pane (closing the pseudoconsole -> reader EOF -> `pane-exit` from the batcher).
fn watch_exit(app: AppHandle, pane_id: String, run: u64) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(EXIT_POLL);
            let state = app.state::<AppState>();
            let Ok(mut panes) = state.panes.0.lock() else {
                return;
            };
            let exited = match panes.get_mut(&pane_id) {
                Some(pane) if pane.run == run => !matches!(pane.child.try_wait(), Ok(None)),
                _ => return, // killed, EOF already handled, or replaced by a newer run
            };
            if exited {
                let pane = panes.remove(&pane_id);
                drop(panes);
                drop(pane); // ConPTY close can block: never under the map lock
                return;
            }
        }
    });
}

/// Reader thread -> batching thread -> `pane-output:{id}` events, then `pane-exit:{id}`.
/// `pane-exit` is emitted only here, once, after all pending output is flushed.
fn stream_output(
    app: AppHandle,
    pane_id: String,
    run: u64,
    mut reader: Box<dyn Read + Send>,
    mut recorder: Option<crate::history::Recorder>,
) {
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(QUEUE_CHUNKS);

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
        let mut closed = false;
        loop {
            // Once closed, just drain `pending` in capped events.
            if !closed && pending.len() < MAX_EMIT_BYTES {
                match rx.recv_timeout(FLUSH_INTERVAL) {
                    Ok(chunk) => pending.extend_from_slice(&chunk),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => closed = true,
                }
            } else if !closed {
                // Backlog: stop pulling (the reader blocks on the full queue) until next tick.
                std::thread::sleep(FLUSH_INTERVAL.saturating_sub(last_flush.elapsed()));
            }
            if !pending.is_empty() && (closed || last_flush.elapsed() >= FLUSH_INTERVAL) {
                let text = take_valid_utf8(&mut pending, MAX_EMIT_BYTES, closed);
                if !text.is_empty() {
                    if let Some(rec) = recorder.as_mut() {
                        rec.write(&text); // buffered; errors stop the recording only
                    }
                    let _ = app.emit(&output_event, text);
                }
                last_flush = Instant::now();
            }
            if let Some(rec) = recorder.as_mut() {
                rec.flush_if_due(); // idle panes still reach the disk
            }
            if closed && pending.is_empty() {
                if let Some(rec) = recorder.take() {
                    rec.finish();
                }
                if is_current_run(&app, &pane_id, run) {
                    let _ = app.emit(&format!("pane-exit:{pane_id}"), ());
                }
                break;
            }
        }
    });
}

/// Pane ids end up in hook URLs and args, and agents run through `cmd.exe /c`
/// on Windows, which re-parses shell metacharacters: reject both up front.
fn validate_spawn_input(pane_id: &str, args: &[String]) -> Result<(), String> {
    let id_ok = (1..=64).contains(&pane_id.len())
        && pane_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !id_ok {
        return Err("invalid pane id".into());
    }
    let unsafe_arg = |a: &String| {
        a.contains(['&', '|', '<', '>', '^', '%', '"']) || a.contains(char::is_control)
    };
    if cfg!(windows) && args.iter().any(unsafe_arg) {
        return Err("arguments may not contain & | < > ^ % \" or control characters".into());
    }
    Ok(())
}

/// Extra env vars the frontend may set: only the orchestrator's MCP secret (a UUID).
fn validate_extra_env(env: &HashMap<String, String>) -> Result<(), String> {
    let ok = env.iter().all(|(k, v)| {
        k == "AGENTDECK_MCP_SECRET"
            && (1..=64).contains(&v.len())
            && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    ok.then_some(())
        .ok_or_else(|| "env variable not allowed".into())
}

/// On exit: frees the pane slot if `run` still owns it (so the id can be respawned).
/// Returns false when a newer run already reuses the id: its exit must not be reported.
fn is_current_run(app: &AppHandle, pane_id: &str, run: u64) -> bool {
    let state = app.state::<AppState>();
    let Ok(mut panes) = state.panes.0.lock() else {
        return true;
    };
    match panes.get(pane_id) {
        Some(pane) if pane.run == run => {
            let pane = panes.remove(pane_id);
            drop(panes);
            drop(pane); // ConPTY close can block: never under the map lock
            true
        }
        Some(_) => false,
        None => true, // killed via kill_pane or reaped by watch_exit
    }
}

/// Drains the longest valid UTF-8 prefix of at most `max` bytes, keeping an
/// incomplete trailing multi-byte sequence for the next chunk (unless `flush_all`).
fn take_valid_utf8(pending: &mut Vec<u8>, max: usize, flush_all: bool) -> String {
    let mut limit = pending.len().min(max);
    if limit < pending.len() {
        // Don't split a multi-byte char at the size cap (continuation bytes are 10xxxxxx).
        let floor = limit.saturating_sub(3);
        while limit > floor && pending[limit] & 0xC0 == 0x80 {
            limit -= 1;
        }
    }
    let cut = match std::str::from_utf8(&pending[..limit]) {
        Ok(_) => limit,
        // error_len() == None means "truncated sequence at the end".
        Err(e) if e.error_len().is_none() && !flush_all => e.valid_up_to(),
        Err(_) => limit,
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
    fn rejects_bad_pane_ids_and_shell_metacharacters() {
        use super::validate_spawn_input as v;
        assert!(v("3f2b-aa", &["--model".into(), "opus".into()]).is_ok());
        assert!(v("a&calc", &[]).is_err());
        assert!(v("", &[]).is_err());
        if cfg!(windows) {
            assert!(v("ok", &["x & calc".into()]).is_err());
        }
    }

    #[test]
    fn extra_env_is_whitelisted() {
        use super::validate_extra_env as v;
        use std::collections::HashMap;
        let env = |k: &str, val: &str| HashMap::from([(k.to_owned(), val.to_owned())]);
        assert!(v(&HashMap::new()).is_ok());
        assert!(v(&env("AGENTDECK_MCP_SECRET", "3f2b-aa")).is_ok());
        assert!(v(&env("PATH", "x")).is_err());
        assert!(v(&env("AGENTDECK_MCP_SECRET", "a b")).is_err());
    }

    #[test]
    fn keeps_truncated_multibyte_tail() {
        let mut buf = vec![b'a', 0xC3]; // 'a' + first byte of 'é'
        assert_eq!(take_valid_utf8(&mut buf, usize::MAX, false), "a");
        assert_eq!(buf, vec![0xC3]);
        buf.push(0xA9);
        assert_eq!(take_valid_utf8(&mut buf, usize::MAX, false), "é");
        assert!(buf.is_empty());
    }

    #[test]
    fn invalid_bytes_are_replaced_not_stuck() {
        let mut buf = vec![b'x', 0xFF, b'y'];
        assert_eq!(take_valid_utf8(&mut buf, usize::MAX, false), "x\u{FFFD}y");
        assert!(buf.is_empty());
    }

    #[test]
    fn caps_bytes_per_flush_on_char_boundary() {
        let mut buf = "abcdef".as_bytes().to_vec();
        assert_eq!(take_valid_utf8(&mut buf, 4, false), "abcd");
        assert_eq!(take_valid_utf8(&mut buf, 4, true), "ef");
        // "aé€": cap 2 would split 'é' (bytes 1..3) -> back off to "a".
        let mut buf = "aé€".as_bytes().to_vec();
        assert_eq!(take_valid_utf8(&mut buf, 2, false), "a");
        assert_eq!(take_valid_utf8(&mut buf, 3, false), "é");
        assert_eq!(take_valid_utf8(&mut buf, 3, true), "€");
        assert!(buf.is_empty());
    }

    /// The real child-exit path: try_wait sees the exit, and dropping the master
    /// (as watch_exit does) makes the reader hit EOF even on ConPTY.
    #[test]
    fn child_exit_is_detected_and_reader_ends() {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};
        use std::io::{Read, Write};
        use std::time::{Duration, Instant};
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = if cfg!(windows) {
            CommandBuilder::new("cmd")
        } else {
            CommandBuilder::new("sh")
        };
        cmd.args(if cfg!(windows) {
            ["/c", "exit 3"]
        } else {
            ["-c", "exit 3"]
        });
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        // ConPTY asks for the cursor position (ESC[6n) and waits for the answer,
        // which xterm.js gives in the app: answer it here.
        let mut writer = pair.master.take_writer().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                if buf[..n].windows(4).any(|w| w == b"\x1b[6n") {
                    let _ = writer.write_all(b"\x1b[1;1R");
                }
            }
            let _ = tx.send(());
        });

        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "child exit not detected");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(status.exit_code(), 3);
        drop(pair.master);
        rx.recv_timeout(Duration::from_secs(10))
            .expect("reader did not reach EOF after dropping the master");
    }
}
