//! App side of the session host: one connection, requests matched to replies by id, output and exit
//! events handed to `pty` (which treats hosted panes like its own).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant};

use super::{Event, HOST_ARG, LOG_FILE, PROTOCOL, PaneInfo, Reply, Request, encode};
use crate::AppHandle;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const HELLO_TIMEOUT: Duration = Duration::from_secs(2);
/// A started host gets this long to listen.
const START_TIMEOUT: Duration = Duration::from_secs(5);
const START_POLL: Duration = Duration::from_millis(100);
/// A local request answers in milliseconds; past this the host is considered gone.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// Settings > General "Keep agents running after quit"; off = in-process pseudoconsoles.
/// `AGENTDECK_NO_SESSION_HOST=1` forces it off.
const DISABLE_ENV: &str = "AGENTDECK_NO_SESSION_HOST";

type OnReply = Box<dyn FnOnce(Reply) + Send>;

pub struct Client {
    /// Lines to the writer thread.
    out: mpsc::Sender<String>,
    pending: Mutex<HashMap<u64, OnReply>>,
    next_id: AtomicU64,
}

static CLIENT: Mutex<Option<Arc<Client>>> = Mutex::new(None);
/// Serializes connecting (and starting the host): parallel spawns must not start two hosts.
static CONNECTING: Mutex<()> = Mutex::new(());
static ENABLED: AtomicBool = AtomicBool::new(true);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// New agents go through the host (the setting is on and not forced off).
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed) && std::env::var_os(DISABLE_ENV).is_none()
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// The live connection, if any.
pub fn client() -> Option<Arc<Client>> {
    lock(&CLIENT).clone()
}

/// Connects to a host that is already running (startup: its panes are adopted). Never starts one.
pub fn connect_existing(app: &AppHandle) -> Option<Arc<Client>> {
    let _guard = lock(&CONNECTING);
    if let Some(client) = client() {
        return Some(client);
    }
    let dir = app.app_data_dir().ok()?;
    let (stream, reader) = probe(&dir).ok()?;
    Some(install(app, stream, reader))
}

/// Connects to the host, starting it when none runs. None: unavailable (the caller runs the pane
/// in process).
pub fn connect_or_start(app: &AppHandle) -> Option<Arc<Client>> {
    let _guard = lock(&CONNECTING);
    if let Some(client) = client() {
        return Some(client);
    }
    let opened = app.app_data_dir().and_then(|dir| match probe(&dir) {
        Ok(io) => Ok(io),
        Err(_) => start_host(&dir).and_then(|()| wait_for_host(&dir)),
    });
    match opened {
        Ok((stream, reader)) => Some(install(app, stream, reader)),
        Err(e) => {
            eprintln!("session host unavailable, agents run in process: {e}");
            None
        }
    }
}

/// Opens a connection to the host named in the info file and does the handshake.
pub(super) fn probe(data_dir: &Path) -> Result<(TcpStream, BufReader<TcpStream>), String> {
    let info = super::read_info(data_dir)?;
    let addr = SocketAddr::from(([127, 0, 0, 1], info.port));
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| e.to_string())?;
    let _ = stream.set_nodelay(true);
    let hello = Request::Hello { token: info.token, version: PROTOCOL };
    stream.write_all(encode(&hello).as_bytes()).map_err(|e| e.to_string())?;
    // The timeout goes on the clone that reads: on Windows a cloned socket keeps its own, so clearing
    // it on `stream` alone left the event reader timing out (connection "lost", every agent ended).
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    reader.get_ref().set_read_timeout(Some(HELLO_TIMEOUT)).map_err(|e| e.to_string())?;
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| format!("no answer: {e}"))?;
    match serde_json::from_str::<Event>(&line) {
        Ok(Event::Hello { .. }) => {}
        _ => return Err("not a session host of this version".into()),
    }
    reader.get_ref().set_read_timeout(None).map_err(|e| e.to_string())?;
    Ok((stream, reader))
}

/// A copy of our exe for the host, in `%LOCALAPPDATA%/com.agentdeck.app/session-host`: a host
/// running the exe itself would lock it (no `cargo build`, no update while agents run). One copy
/// per build (size + modified time); older copies not running are removed.
fn host_exe() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let meta = std::fs::metadata(&exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let stamp = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let dir = dirs::data_local_dir()
        .ok_or("no local app data directory")?
        .join(crate::APP_IDENTIFIER)
        .join("session-host");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = format!("agentdeck-host-{}-{stamp}{}", meta.len(), std::env::consts::EXE_SUFFIX);
    let path = dir.join(name);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() == meta.len()) {
        return Ok(path);
    }
    // Temp file + rename: a concurrent start never runs a half-written copy.
    let tmp = dir.join(format!("copy-{}.tmp", std::process::id()));
    std::fs::copy(&exe, &tmp)
        .and_then(|_| std::fs::rename(&tmp, &path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("{}: {e}", path.display())
        })?;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.path() != path {
                let _ = std::fs::remove_file(entry.path()); // a running host keeps its copy
            }
        }
    }
    Ok(path)
}

/// Starts `agentdeck --session-host <dir>` outside our console and job: it must outlive us.
fn start_host(data_dir: &Path) -> Result<(), String> {
    let exe = host_exe()?;
    let log = std::fs::File::create(data_dir.join(LOG_FILE)).map_err(|e| e.to_string())?;
    let command = || {
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg(HOST_ARG).arg(data_dir).current_dir(data_dir);
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
        if let Ok(log) = log.try_clone() {
            cmd.stderr(log);
        }
        cmd
    };
    #[cfg(windows)]
    let started = {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        // A job that forbids breakaway refuses the flag: then the host may end with us (as agents did).
        command()
            .creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB)
            .spawn()
            .or_else(|_| command().creation_flags(flags).spawn())
    };
    #[cfg(not(windows))]
    let started = command().spawn();
    started.map(drop).map_err(|e| format!("cannot start the session host: {e}"))
}

fn wait_for_host(data_dir: &Path) -> Result<(TcpStream, BufReader<TcpStream>), String> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match probe(data_dir) {
            Ok(io) => return Ok(io),
            Err(e) if Instant::now() >= deadline => return Err(format!("session host did not start: {e}")),
            Err(_) => std::thread::sleep(START_POLL),
        }
    }
}

/// Starts the writer and reader threads and makes this the current connection.
fn install(app: &AppHandle, stream: TcpStream, reader: BufReader<TcpStream>) -> Arc<Client> {
    let (out, rx) = mpsc::channel::<String>();
    let client = Arc::new(Client { out, pending: Mutex::default(), next_id: AtomicU64::new(1) });
    let mut writer = stream;
    let spawned = std::thread::Builder::new().name("session-host-out".into()).spawn(move || {
        for line in rx {
            if writer.write_all(line.as_bytes()).is_err() {
                break;
            }
        }
    });
    if let Err(e) = spawned {
        eprintln!("session host writer: {e}");
    }
    let (app, me) = (app.clone(), client.clone());
    let spawned = std::thread::Builder::new()
        .name("session-host-in".into())
        .spawn(move || read_events(&app, &me, reader));
    if let Err(e) = spawned {
        eprintln!("session host reader: {e}");
    }
    *lock(&CLIENT) = Some(client.clone());
    client
}

/// Dispatches host events in order until the connection drops; then hosted panes end.
fn read_events(app: &AppHandle, me: &Arc<Client>, mut reader: BufReader<TcpStream>) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Event>(&line) {
            Ok(Event::Reply(reply)) => {
                let on_reply = lock(&me.pending).remove(&reply.id);
                if let Some(on_reply) = on_reply {
                    on_reply(reply);
                }
            }
            Ok(Event::Output { pane, run, data }) => crate::pty::hosted_output(app, &pane, run, data.into_bytes()),
            Ok(Event::Exit { pane, run }) => crate::pty::hosted_exit(app, &pane, run),
            Ok(Event::Hello { .. }) => {}
            Err(e) => eprintln!("session host: bad event: {e}"),
        }
    }
    eprintln!("session host connection lost");
    {
        let mut current = lock(&CLIENT);
        if current.as_ref().is_some_and(|c| Arc::ptr_eq(c, me)) {
            *current = None;
        }
    }
    lock(&me.pending).clear(); // waiters see their channel close
    crate::pty::hosted_lost(app);
}

impl Client {
    /// Fire and forget (write, resize, kill).
    pub fn send(&self, request: &Request) -> Result<(), String> {
        self.out.send(encode(request)).map_err(|_| "session host disconnected".to_owned())
    }

    /// Sends the request built with its id and waits for the reply. `on_reply` runs on the reader
    /// thread before any later event is handled (an attach installs the pane there, so no output
    /// is missed). None: no answer (host gone or too slow).
    pub fn call<T: Send + 'static>(
        &self,
        make: impl FnOnce(u64) -> Request,
        on_reply: impl FnOnce(Reply) -> T + Send + 'static,
    ) -> Option<T> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        lock(&self.pending).insert(id, Box::new(move |reply| drop(tx.send(on_reply(reply)))));
        if self.send(&make(id)).is_err() {
            lock(&self.pending).remove(&id);
            return None;
        }
        let answer = rx.recv_timeout(REPLY_TIMEOUT).ok();
        lock(&self.pending).remove(&id);
        answer
    }

    pub fn list(&self) -> Option<Vec<PaneInfo>> {
        self.call(|id| Request::List { id }, |reply| reply.panes)
    }
}

/// Kills every hosted agent and ends the host (waits for its answer, so it is safe right before exit).
pub fn shutdown() {
    if let Some(client) = client()
        && client.call(|id| Request::Shutdown { id }, drop).is_none()
    {
        eprintln!("session host did not confirm the shutdown");
    }
}
