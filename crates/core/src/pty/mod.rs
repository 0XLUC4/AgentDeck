//! PTY management: one pseudo-terminal per pane, output batched to the frontend.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use crate::{AppHandle, AppState};
use crate::agents::{self, AgentKind};

#[cfg(all(windows, target_arch = "x86_64"))]
pub mod conpty;

/// Output is flushed to subscribers at most every 16 ms (~60 fps).
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
/// Wake-up period with nothing pending: matches the history recorder's flush cadence.
const IDLE_WAIT: Duration = Duration::from_secs(2);
/// Max bytes per output message; the rest waits for the next tick.
const MAX_EMIT_BYTES: usize = 256 * 1024;
/// Reader -> batcher queue depth (8 KB chunks): a full queue blocks the reader.
const QUEUE_CHUNKS: usize = 64;
/// ConPTY often keeps the output pipe open after the child exits: poll for exit.
const EXIT_POLL: Duration = Duration::from_millis(250);

/// Distinguishes successive processes spawned under the same pane id (restart).
static NEXT_RUN: AtomicU64 = AtomicU64::new(0);
static NEXT_SINK: AtomicU64 = AtomicU64::new(0);

/// Output subscriber: (owner label, e.g. a window id; sink id; channel).
type Sink = (String, u64, mpsc::Sender<String>);
/// Output subscribers per pane id. Keyed by id, not run, so they survive a restart.
static SINKS: LazyLock<Mutex<HashMap<String, Vec<Sink>>>> = LazyLock::new(Mutex::default);

fn sinks() -> std::sync::MutexGuard<'static, HashMap<String, Vec<Sink>>> {
    SINKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Subscribes `owner` (a window label) to a pane's output. Returns the id for `detach_pane_output`.
pub fn attach_pane_output(owner: &str, pane_id: String, on_output: mpsc::Sender<String>) -> u64 {
    let id = NEXT_SINK.fetch_add(1, Ordering::Relaxed);
    sinks()
        .entry(pane_id)
        .or_default()
        .push((owner.to_string(), id, on_output));
    id
}

pub fn detach_pane_output(pane_id: String, sink: u64) {
    let mut sinks = sinks();
    if let Some(list) = sinks.get_mut(&pane_id) {
        list.retain(|(_, id, _)| *id != sink);
        if list.is_empty() {
            sinks.remove(&pane_id);
        }
    }
}

/// Forgets every subscription of a window that closed (its channels are dead).
pub fn drop_owner_sinks(label: &str) {
    sinks().retain(|_, list| {
        list.retain(|(owner, _, _)| owner != label);
        !list.is_empty()
    });
}

fn send_output(pane_id: &str, text: &str) {
    // Clone out so the lock isn't held while sending.
    let targets: Vec<(u64, mpsc::Sender<String>)> = sinks()
        .get(pane_id)
        .map(|list| list.iter().map(|(_, id, ch)| (*id, ch.clone())).collect())
        .unwrap_or_default();
    for (sink, channel) in targets {
        if channel.send(text.to_owned()).is_err() {
            detach_pane_output(pane_id.to_owned(), sink);
        }
    }
}

/// Starts the thread that writes a pane's input in order. A child slow to read its input (large
/// paste, busy agent) blocks this thread, never the caller. It ends when the sender is dropped
/// with the pane, so input queued for an old process never reaches a restarted one.
pub(crate) fn spawn_writer(
    pane_id: &str,
    mut writer: Box<dyn Write + Send>,
) -> std::io::Result<mpsc::Sender<Vec<u8>>> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let id = pane_id.to_owned();
    std::thread::Builder::new()
        .name(format!("pty-in-{pane_id}"))
        .spawn(move || {
            while let Ok(data) = rx.recv() {
                if let Err(e) = writer.write_all(&data).and_then(|()| writer.flush()) {
                    eprintln!("pane {id}: write failed: {e}");
                    break;
                }
            }
        })?;
    Ok(tx)
}

/// A pseudoconsole owned by this process: it dies with the app.
struct Local {
    master: Box<dyn MasterPty + Send>,
    /// Input queue drained by the pane's writer thread (`spawn_writer`).
    writer: mpsc::Sender<Vec<u8>>,
    child: Box<dyn Child + Send + Sync>,
    /// Dropped last: closing it kills the agent's whole process tree (MCP servers, dev servers...).
    #[cfg(windows)]
    _job: Option<job::Job>,
}

/// A pseudoconsole owned by the session host: it outlives the app.
struct Hosted {
    /// The host's run number: output and exit events of an older process are ignored.
    host_run: u64,
    /// Feeds `stream_output` (through [`ChannelReader`]); dropped on exit, which ends the stream.
    output: Option<mpsc::Sender<Vec<u8>>>,
}

enum Proc {
    Local(Local),
    Hosted(Hosted),
}

pub struct Pane {
    proc: Proc,
    run: u64,
}

#[derive(Default)]
pub struct Panes(pub Mutex<HashMap<String, Pane>>);

impl Panes {
    /// Writes raw bytes to a pane's PTY. Used by commands and by the voice module.
    /// Never blocks on the child: the bytes are queued for the pane's writer thread (or the session
    /// host's connection), which logs write errors.
    pub fn write(&self, pane_id: &str, data: &[u8]) -> Result<(), String> {
        {
            let panes = self.0.lock().map_err(|e| e.to_string())?;
            if let Proc::Local(local) = &panes.get(pane_id).ok_or("pane not found")?.proc {
                return local
                    .writer
                    .send(data.to_vec())
                    .map_err(|_| "pane input closed".to_string());
            }
        }
        let client = crate::session_host::client().ok_or("session host disconnected")?;
        let data = String::from_utf8_lossy(data).into_owned();
        client.send(&crate::session_host::Request::Write { pane: pane_id.to_owned(), data })
    }
}

/// `Read` over the chunks a hosted pane receives: lets `stream_output` batch, record and report the
/// exit of hosted panes exactly as it does for local ones. EOF once the sender is dropped.
struct ChannelReader {
    rx: mpsc::Receiver<Vec<u8>>,
    chunk: Vec<u8>,
    pos: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.chunk.len() {
            match self.rx.recv() {
                Ok(chunk) => {
                    self.chunk = chunk;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.chunk.len() - self.pos);
        buf[..n].copy_from_slice(&self.chunk[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Registers a hosted pane and starts its output stream. `first_output` (what the host printed
/// before the reply) goes out as normal output.
fn start_hosted(
    app: &AppHandle,
    pane_id: &str,
    host_run: u64,
    first_output: String,
    recorder: Option<crate::history::Recorder>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    if !first_output.is_empty() {
        let _ = tx.send(first_output.into_bytes());
    }
    let run = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
    {
        let mut panes = state.panes.0.lock().map_err(|e| e.to_string())?;
        if panes.contains_key(pane_id) {
            return Err(crate::session_host::ALREADY_RUNNING.into());
        }
        let proc = Proc::Hosted(Hosted { host_run, output: Some(tx) });
        panes.insert(pane_id.to_owned(), Pane { proc, run });
    }
    let reader = ChannelReader { rx, chunk: Vec::new(), pos: 0 };
    stream_output(app.clone(), pane_id.to_owned(), run, Box::new(reader), recorder);
    Ok(())
}

/// The output sender of a hosted pane whose host run is `host_run`.
fn hosted_sender(app: &AppHandle, pane_id: &str, host_run: u64) -> Option<mpsc::Sender<Vec<u8>>> {
    let state = app.try_state::<AppState>()?;
    let panes = state.panes.0.lock().ok()?;
    match &panes.get(pane_id)?.proc {
        Proc::Hosted(h) if h.host_run == host_run => h.output.clone(),
        _ => None,
    }
}

/// Session host output for a pane (dropped when the pane is not, or no longer, ours).
pub(crate) fn hosted_output(app: &AppHandle, pane_id: &str, host_run: u64, data: Vec<u8>) {
    if let Some(tx) = hosted_sender(app, pane_id, host_run) {
        let _ = tx.send(data);
    }
}

/// The hosted process ended: ends its stream, which flushes and emits `pane-exit` as usual.
pub(crate) fn hosted_exit(app: &AppHandle, pane_id: &str, host_run: u64) {
    let Some(state) = app.try_state::<AppState>() else { return };
    let Ok(mut panes) = state.panes.0.lock() else { return };
    if let Some(Pane { proc: Proc::Hosted(h), .. }) = panes.get_mut(pane_id)
        && h.host_run == host_run
    {
        h.output = None;
    }
}

/// The host connection dropped: its processes are gone (they die with it), every hosted pane ends.
pub(crate) fn hosted_lost(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else { return };
    let Ok(mut panes) = state.panes.0.lock() else { return };
    for pane in panes.values_mut() {
        if let Proc::Hosted(h) = &mut pane.proc {
            h.output = None;
        }
    }
}

/// Reattaches to a pane the session host still runs (app restarted). `on_replay` gets the output
/// it kept, before any live output reaches the pane's subscribers. Ok without doing anything when
/// the pane is already ours.
pub fn attach_hosted(
    app: &AppHandle,
    pane_id: &str,
    on_replay: Box<dyn FnOnce(String) + Send>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.panes.0.lock().map_err(|e| e.to_string())?.contains_key(pane_id) {
        return Ok(());
    }
    let client = crate::session_host::client().ok_or("session host not connected")?;
    let (app, id) = (app.clone(), pane_id.to_owned());
    let pane = id.clone();
    client
        .call(
            move |req| crate::session_host::Request::Attach { id: req, pane },
            move |reply| {
                let reply = reply.into_result()?;
                let meta = &reply.meta;
                let recorder = crate::history::Recorder::start(&id, &meta.agent, &meta.cwd, meta.title.clone());
                on_replay(reply.scrollback);
                start_hosted(&app, &id, reply.run, String::new(), recorder)
            },
        )
        .unwrap_or_else(|| Err("session host did not answer".into()))
}

/// Live panes: ours and those the session host runs without us (after a restart).
pub fn list_all_panes(state: &AppState) -> Result<Vec<String>, String> {
    let mut ids = list_panes(state)?;
    if let Some(hosted) = crate::session_host::client().and_then(|c| c.list()) {
        for info in hosted {
            if !ids.contains(&info.pane) {
                ids.push(info.pane);
            }
        }
    }
    Ok(ids)
}

/// Panes whose process lives in this app (not in the session host): they end when the app quits.
pub fn local_pane_ids(state: &AppState) -> Vec<String> {
    state.panes.0.lock().map_or_else(
        |_| Vec::new(),
        |panes| {
            panes
                .iter()
                .filter(|(_, p)| matches!(p.proc, Proc::Local(_)))
                .map(|(id, _)| id.clone())
                .collect()
        },
    )
}

/// Spawns an agent (or a plain shell) in a new PTY under `pane_id`.
/// The frontend picks the id so it can subscribe to output before spawning.
#[allow(clippy::too_many_arguments)]
pub fn spawn_pane(
    app: &AppHandle,
    pane_id: String,
    agent: AgentKind,
    cwd: String,
    args: Vec<String>,
    worktree: bool,
    cols: u16,
    rows: u16,
    title: Option<String>,
    env: Option<HashMap<String, String>>,
    trust: Option<bool>,
) -> Result<(), String> {
    let app = app.clone();
    let state = app.state::<AppState>();
    let trust = trust.unwrap_or(false);
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
    let mut env = env;
    let ssh = env.remove(SSH_ENV);
    // Validated by `validate_extra_env`: a saved host (port, key, tmux) for this ssh pane.
    let host = env
        .remove(crate::hosts::HOST_ENV)
        .and_then(|v| serde_json::from_str::<crate::hosts::Host>(&v).ok());
    let cwd = if worktree && ssh.is_none() {
        agents::create_worktree(&cwd, &pane_id)?
    } else {
        cwd
    };
    // A remote agent cannot reach the hook server: nothing to install here.
    if ssh.is_none() {
        agents::prepare_hooks(agent, &cwd, &state.hooks)?;
        if trust {
            crate::trust::trust_folder(agent, &cwd);
        }
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    conpty::preload();

    let mut cmd: CommandBuilder = match ssh.as_deref().map(ssh_target) {
        Some(Some((target, dir))) => match &host {
            Some(host) => host.agent_command(agent, dir, &args, &pane_id),
            None => {
                let mut cmd = CommandBuilder::new("ssh");
                cmd.args(["-t", target, &ssh_command(agent, dir, &args)]);
                cmd
            }
        },
        Some(None) => return Err("SSH target: user@host:/folder".into()),
        None => {
            let args = [
                agents::hook_args(agent, &pane_id, &state.hooks),
                crate::mcp::hub::agent_args(agent),
                args,
            ]
            .concat();
            agents::command_for(agent, &args)
        }
    };
    cmd.cwd(&cwd);
    set_agent_env(&mut cmd, &app, &state, &pane_id, agent, &env, trust, &cwd);
    let agent_name = serde_json::to_value(agent).ok();
    let agent_name = agent_name.as_ref().and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let meta = crate::session_host::Meta { agent: agent_name, cwd: cwd.clone(), title };

    // In the session host when possible: the agent then survives quitting the app.
    if crate::session_host::enabled()
        && let Some(client) = crate::session_host::connect_or_start(&app)
    {
        match spawn_hosted(&app, &client, &pane_id, &cmd, &cwd, (cols, rows), meta.clone()) {
            Some(result) => return result,
            None => eprintln!("pane {pane_id}: session host did not answer, starting in process"),
        }
    }

    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;
    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let job = child.process_id().and_then(job::Job::adopt);
    drop(pair.slave); // child owns it now; dropping lets EOF propagate on exit
    let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let writer = spawn_writer(&pane_id, writer).map_err(|e| e.to_string())?;

    let run = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
    // Insert before streaming so the exit handler always finds this run.
    state.panes.0.lock().map_err(|e| e.to_string())?.insert(
        pane_id.clone(),
        Pane {
            proc: Proc::Local(Local {
                master: pair.master,
                writer,
                child,
                #[cfg(windows)]
                _job: job,
            }),
            run,
        },
    );
    watch_exit(app.clone(), pane_id.clone(), run);
    let recorder = crate::history::Recorder::start(&pane_id, &meta.agent, &cwd, meta.title);
    stream_output(app, pane_id, run, reader, recorder);
    Ok(())
}

/// Spawns the prepared command in the session host and registers the pane. None: the host did not
/// answer (the caller falls back to an in-process pane); Some(Err): the spawn itself failed.
fn spawn_hosted(
    app: &AppHandle,
    client: &crate::session_host::client::Client,
    pane_id: &str,
    cmd: &CommandBuilder,
    cwd: &str,
    (cols, rows): (u16, u16),
    meta: crate::session_host::Meta,
) -> Option<Result<(), String>> {
    let argv: Vec<String> = cmd.get_argv().iter().map(|a| a.to_string_lossy().into_owned()).collect();
    let env: Vec<(String, String)> = cmd
        .iter_full_env_as_str()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    let (app, id) = (app.clone(), pane_id.to_owned());
    let request = |req| crate::session_host::Request::Spawn {
        id: req,
        pane: pane_id.to_owned(),
        argv,
        cwd: cwd.to_owned(),
        env,
        cols,
        rows,
        meta,
    };
    client.call(request, move |reply| {
        let reply = reply.into_result()?;
        let meta = &reply.meta;
        let recorder = crate::history::Recorder::start(&id, &meta.agent, &meta.cwd, meta.title.clone());
        start_hosted(&app, &id, reply.run, reply.scrollback, recorder)
    })
}

/// Env of every agent process: AgentDeck's pane id and hook server, minus a parent Claude's session.
#[allow(clippy::too_many_arguments)]
fn set_agent_env(
    cmd: &mut CommandBuilder,
    app: &AppHandle,
    state: &AppState,
    pane_id: &str,
    agent: AgentKind,
    env: &HashMap<String, String>,
    trust: bool,
    cwd: &str,
) {
    // When AgentDeck itself is started from a Claude Code session, don't leak that session's
    // identity (messaging token/socket, child-session markers) into the agents we spawn.
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(is_parent_claude_var) {
            cmd.env_remove(&key);
        }
    }
    cmd.env("AGENTDECK_PANE_ID", pane_id);
    cmd.env("AGENTDECK_HOOK_URL", state.hooks.url());
    cmd.env("AGENTDECK_HOOK_TOKEN", &state.hooks.token);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    for (key, value) in env {
        cmd.env(key, value);
    }
    // A worktree builds into the main checkout's target: no from-scratch compile per agent.
    if !env.contains_key("CARGO_TARGET_DIR")
        && std::env::var_os("CARGO_TARGET_DIR").is_none()
        && let Some(root) = agents::agentdeck_worktree_root(std::path::Path::new(cwd))
    {
        cmd.env("CARGO_TARGET_DIR", root.join("target"));
    }
    // Grok documents no trust-file format, only this switch (docs/user-guide/10-hooks.md).
    if trust && agent == AgentKind::Grok {
        cmd.env("GROK_FOLDER_TRUST", "0");
    }
    if let Some((key, value)) = crate::mcp::hub::agent_env(agent) {
        cmd.env(key, value);
    }
    if agent == AgentKind::Gemini
        && let Ok(data) = app.app_data_dir()
        && let Some((key, path)) = agents::gemini_env(&data, &state.hooks)
    {
        cmd.env(key, path);
    }
}

/// The agent as a plain piped process (chat panes): same checks, hooks, MCP servers and env as
/// [`spawn_pane`], no PTY. `cwd` must already be the final folder (worktree created). Local only.
pub fn pipe_command(
    app: &AppHandle,
    pane_id: &str,
    agent: AgentKind,
    cwd: &str,
    args: &[String],
    env: &HashMap<String, String>,
    trust: bool,
) -> Result<std::process::Command, String> {
    let state = app.state::<AppState>();
    validate_spawn_input(pane_id, args)?;
    validate_extra_env(env)?;
    if env.contains_key(SSH_ENV) {
        return Err("chat mode runs agents on this machine only".into());
    }
    agents::prepare_hooks(agent, cwd, &state.hooks)?;
    if trust {
        crate::trust::trust_folder(agent, cwd);
    }
    let args = [crate::mcp::hub::agent_args(agent), args.to_vec()].concat();
    let mut builder = agents::command_for(agent, &args);
    set_agent_env(&mut builder, app, &state, pane_id, agent, env, trust, cwd);
    let argv = builder.get_argv();
    let mut cmd = std::process::Command::new(argv.first().ok_or("empty command")?);
    cmd.args(&argv[1..]).current_dir(cwd).env_clear();
    cmd.envs(builder.iter_full_env_as_str());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    Ok(cmd)
}

pub fn write_to_pane(
    state: &AppState,
    pane_id: String,
    data: String,
) -> Result<(), String> {
    state.panes.write(&pane_id, data.as_bytes())
}

pub fn resize_pane(
    state: &AppState,
    pane_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    {
        let panes = state.panes.0.lock().map_err(|e| e.to_string())?;
        if let Proc::Local(local) = &panes.get(&pane_id).ok_or("pane not found")?.proc {
            return local
                .master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| e.to_string());
        }
    }
    let client = crate::session_host::client().ok_or("session host disconnected")?;
    client.send(&crate::session_host::Request::Resize { pane: pane_id, cols, rows })
}

pub fn kill_pane(state: &AppState, pane_id: String) -> Result<(), String> {
    // Removed under the lock (temporary guard), killed and dropped outside it.
    let pane = state
        .panes
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&pane_id);
    match pane {
        // Already-exited children return an error here; that's fine.
        Some(Pane { proc: Proc::Local(mut local), .. }) => {
            let _ = local.child.kill();
        }
        // Hosted, or a host pane not attached (orphan after a restart): the host kills it.
        _ => {
            if let Some(client) = crate::session_host::client() {
                client.send(&crate::session_host::Request::Kill { pane: pane_id })?;
            }
        }
    }
    Ok(())
}

/// Ids of panes whose process is alive: a reloaded webview adopts them instead of respawning.
pub fn list_panes(state: &AppState) -> Result<Vec<String>, String> {
    Ok(state
        .panes
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .keys()
        .cloned()
        .collect())
}

/// Marks which pane receives voice transcripts.
pub fn set_active_pane(state: &AppState, pane_id: Option<String>) -> Result<(), String> {
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
                Some(Pane { proc: Proc::Local(local), run: r }) if *r == run => {
                    !matches!(local.child.try_wait(), Ok(None))
                }
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

/// Reader thread -> batching thread -> output channels, then the `pane-exit:{id}` event.
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
        let mut pending: Vec<u8> = Vec::new();
        let mut last_flush = Instant::now();
        let mut closed = false;
        loop {
            // Once closed, just drain `pending` in capped events.
            if !closed && pending.len() < MAX_EMIT_BYTES {
                // Idle: block until output arrives (waking only for the recorder's periodic flush),
                // instead of ~60 empty wakeups a second per pane.
                let wait = if pending.is_empty() {
                    IDLE_WAIT
                } else {
                    FLUSH_INTERVAL
                };
                match rx.recv_timeout(wait) {
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
                    send_output(&pane_id, &text);
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

/// Session variables set by a parent Claude Code process. User config like CLAUDE_CONFIG_DIR is kept.
/// AI_AGENT makes a child Claude Code think an agent drives it, so it drops the fullscreen TUI.
fn is_parent_claude_var(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    key.starts_with("CLAUDE_CODE_")
        || matches!(
            key.as_str(),
            "CLAUDECODE" | "CLAUDE_PID" | "CLAUDE_EFFORT" | "AI_AGENT"
        )
}

/// Pane ids end up in hook URLs and args, and agents may run through `cmd.exe /c`
/// on Windows, which re-parses shell metacharacters: reject both up front.
/// Env key carrying a pane's SSH target to `spawn_pane` (never given to the process).
pub const SSH_ENV: &str = "AGENTDECK_SSH";

/// Host and folder of "user@host:/dir" ("" = the remote home). Both go on a remote shell's
/// command line: plain characters only.
pub fn ssh_target(target: &str) -> Option<(&str, &str)> {
    let (host, dir) = target.split_once(':').unwrap_or((target, ""));
    let plain = |s: &str, extra: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c));
    let host_ok = !host.is_empty() && !host.starts_with('-') && plain(host, "@._-");
    (host_ok && plain(dir, "/._-~")).then_some((host, dir))
}

/// `s` as one single-quoted POSIX shell word.
pub(crate) fn sh_quote(s: &str) -> String {
    const QUOTE: char = '\x27';
    // 'it'\''s': the only way to put a quote inside single quotes.
    format!("{QUOTE}{}{QUOTE}", s.replace(QUOTE, "\x27\\\x27\x27"))
}

/// The agent with its arguments, through a login shell so its PATH is the user's.
pub(crate) fn login_run(agent: AgentKind, args: &[String]) -> String {
    let mut run = format!("exec {}", agent.binary().unwrap_or("\"$SHELL\" -l"));
    for arg in args {
        run.push(' ');
        run.push_str(&sh_quote(arg));
    }
    format!("exec \"$SHELL\" -lc {}", sh_quote(&run))
}

/// The remote command: the agent in `dir`, through a login shell so its PATH is the user's.
fn ssh_command(agent: AgentKind, dir: &str, args: &[String]) -> String {
    let cd = if dir.is_empty() { String::new() } else { format!("cd {dir} && ") };
    format!("{cd}{}", login_run(agent, args))
}

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
    let ok = env.iter().all(|(k, v)| match k.as_str() {
        "AGENTDECK_MCP_SECRET" => {
            (1..=64).contains(&v.len()) && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        }
        // Per-agent dev server port (settings.basePort), so parallel agents don't collide.
        "PORT" => v.parse::<u16>().is_ok_and(|port| port >= 1024),
        // Gemini's browser panel DevTools port, expanded in its settings file.
        crate::hooks_server::BROWSER_PORT_ENV => v.parse::<u16>().is_ok(),
        // Windows: links agents open go to AgentDeck's browser panel through its open.cmd.
        "BROWSER" => v.ends_with("open.cmd") && !v.contains(char::is_control),
        // OpenCode's browser MCP server and role, as inline JSON config.
        "OPENCODE_CONFIG_CONTENT" => serde_json::from_str::<serde_json::Value>(v).is_ok_and(|c| c.is_object()),
        SSH_ENV => ssh_target(v).is_some(),
        crate::hosts::HOST_ENV => {
            serde_json::from_str::<crate::hosts::Host>(v).is_ok_and(|host| host.validate().is_ok())
        }
        _ => false,
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
pub(crate) fn take_valid_utf8(pending: &mut Vec<u8>, max: usize, flush_all: bool) -> String {
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
    fn strips_parent_claude_session_vars_only() {
        use super::is_parent_claude_var as v;
        assert!(v("CLAUDE_CODE_MESSAGING_TOKEN"));
        assert!(v("CLAUDECODE"));
        assert!(v("claude_pid"));
        assert!(v("AI_AGENT"));
        assert!(!v("CLAUDE_CONFIG_DIR"));
        assert!(!v("PATH"));
    }

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
        assert!(v(&env("PORT", "4100")).is_ok());
        assert!(v(&env("PORT", "80")).is_err());
        assert!(v(&env("PORT", "41a")).is_err());
        assert!(v(&env("AGENTDECK_BROWSER_PORT", "9333")).is_ok());
        assert!(v(&env("AGENTDECK_BROWSER_PORT", "x")).is_err());
        assert!(v(&env("BROWSER", r"C:\data\browser\open.cmd")).is_ok());
        assert!(v(&env("BROWSER", "calc.exe")).is_err());
        assert!(v(&env("OPENCODE_CONFIG_CONTENT", r#"{"mcp":{}}"#)).is_ok());
        assert!(v(&env("OPENCODE_CONFIG_CONTENT", "nope")).is_err());
    }

    /// Input is written in the order it was queued, and the thread ends with its sender.
    #[test]
    fn writer_thread_keeps_order_and_ends_with_the_pane() {
        struct Out(std::sync::mpsc::Sender<Vec<u8>>);
        impl std::io::Write for Out {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.send(buf.to_vec()).map_err(std::io::Error::other)?;
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let (out_tx, out_rx) = std::sync::mpsc::channel();
        let input = super::spawn_writer("t", Box::new(Out(out_tx))).unwrap();
        for chunk in ["ab", "cd", "ef"] {
            input.send(chunk.as_bytes().to_vec()).unwrap();
        }
        drop(input);
        // The iterator ends when the thread exits and drops the writer.
        let written: Vec<u8> = out_rx.iter().flatten().collect();
        assert_eq!(written, b"abcdef");
    }

    /// Hosted panes read their output through a channel: chunks come out whole or split to fit the
    /// buffer, in order, and EOF follows the sender's drop.
    #[test]
    fn channel_reader_streams_chunks_then_eof() {
        use std::io::Read;
        let (tx, rx) = std::sync::mpsc::channel();
        let mut reader = super::ChannelReader { rx, chunk: Vec::new(), pos: 0 };
        tx.send(b"hello".to_vec()).unwrap();
        tx.send(Vec::new()).unwrap(); // an empty chunk is skipped, not read as EOF
        tx.send(b"!".to_vec()).unwrap();
        drop(tx);
        let mut buf = [0u8; 3];
        assert_eq!(reader.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf, b"hel");
        let mut rest = Vec::new();
        reader.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"lo!");
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
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

/// Windows Job Object with kill-on-close: the agent and everything it starts die with the pane,
/// even processes outside the pseudoconsole and even if AgentDeck crashes.
#[cfg(windows)]
pub mod job {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    pub struct Job(HANDLE);

    // SAFETY: a job handle is a plain kernel handle, usable from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        /// Puts `pid` in a new kill-on-close job. `None` (tree not tracked) if any step fails.
        pub fn adopt(pid: u32) -> Option<Self> {
            // SAFETY: plain Win32 calls; every handle is checked and owned by `Job` or closed here.
            unsafe {
                let job = Job(CreateJobObjectW(std::ptr::null(), std::ptr::null()));
                if job.0.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast::<c_void>(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if set == 0 {
                    return None;
                }
                let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
                if process.is_null() {
                    return None;
                }
                let assigned = AssignProcessToJobObject(job.0, process);
                CloseHandle(process);
                (assigned != 0).then_some(job)
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: we own this handle and close it once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod ssh_tests {
    use super::{AgentKind, ssh_command, ssh_target};

    #[test]
    fn ssh_target_and_command() {
        assert_eq!(ssh_target("luca@box:/srv/app"), Some(("luca@box", "/srv/app")));
        assert_eq!(ssh_target("box"), Some(("box", "")));
        for bad in ["", "-oProxyCommand=x", "box:/a b", "box:$(id)", "a;b", "box:/a\x27b"] {
            assert_eq!(ssh_target(bad), None, "{bad}");
        }
        let run = ssh_command(AgentKind::Claude, "/srv/app", &["--model".into(), "opus".into()]);
        assert_eq!(run, "cd /srv/app && exec \"$SHELL\" -lc \x27exec claude \x27\\\x27\x27--model\x27\\\x27\x27 \x27\\\x27\x27opus\x27\\\x27\x27\x27");
        assert_eq!(ssh_command(AgentKind::Shell, "", &[]), "exec \"$SHELL\" -lc \x27exec \"$SHELL\" -l\x27");
    }
}
