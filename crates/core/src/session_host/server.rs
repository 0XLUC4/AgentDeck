//! The host process: owns the pseudoconsoles, keeps each pane's recent output, serves the app.
//! Ends by itself once it has had no pane and no client for [`IDLE_EXIT`].

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Duration;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use super::{ALREADY_RUNNING, Event, Info, Meta, PROTOCOL, PaneInfo, Reply, Request, SCROLLBACK_BYTES, encode};
use crate::logic::ring_buffer::RingBuffer;

/// No pane and no client for this long: the host exits.
const IDLE_EXIT: Duration = Duration::from_secs(5 * 60);
const IDLE_CHECK: Duration = Duration::from_secs(15);
/// Same cadence as the app's in-process panes (ConPTY may keep the pipe open after the exit).
const EXIT_POLL: Duration = Duration::from_millis(250);
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Clients receiving a pane's output: (client id, its outgoing line queue).
type Subscribers = Vec<(u64, mpsc::Sender<String>)>;

/// Output side of a pane, shared with its reader thread (which outlives the pane on kill).
struct Shared {
    scrollback: RingBuffer,
    subscribers: Subscribers,
}

struct HostPane {
    run: u64,
    meta: Meta,
    master: Box<dyn MasterPty + Send>,
    input: mpsc::Sender<Vec<u8>>,
    child: Box<dyn Child + Send + Sync>,
    shared: Arc<Mutex<Shared>>,
    /// Dropped last: kills the agent's whole tree, as in the app.
    #[cfg(windows)]
    _job: Option<crate::pty::job::Job>,
}

#[derive(Default)]
struct Host {
    panes: Mutex<HashMap<String, HostPane>>,
    clients: AtomicUsize,
    next_client: AtomicU64,
    next_run: AtomicU64,
}

/// Runs the host until it is idle or told to shut down.
pub fn run(data_dir: &Path) -> Result<(), String> {
    if super::client::probe(data_dir).is_ok() {
        return Ok(()); // another host already serves this data folder
    }
    #[cfg(all(windows, target_arch = "x86_64"))]
    crate::pty::conpty::preload();
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("cannot listen: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token = uuid::Uuid::new_v4().to_string();
    let info = Info { port, token: token.clone(), pid: std::process::id() };
    // Temp file + rename: the app never reads a half-written file.
    let path = super::info_path(data_dir);
    let tmp = path.with_extension(format!("{}.tmp", info.pid));
    std::fs::write(&tmp, encode(&info))
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let host = Arc::new(Host::default());
    watch_idle(host.clone(), data_dir.to_owned());
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let (host, token, dir) = (host.clone(), token.clone(), data_dir.to_owned());
        let started = std::thread::Builder::new().name("host-client".into()).spawn(move || {
            if let Err(e) = serve(&host, stream, &token, &dir) {
                eprintln!("client: {e}");
            }
        });
        if let Err(e) = started {
            eprintln!("client thread: {e}");
        }
    }
    Ok(())
}

/// Removes the info file if it still names this process, then exits.
fn exit_host(data_dir: &Path) -> ! {
    if super::read_info(data_dir).is_ok_and(|info| info.pid == std::process::id()) {
        let _ = std::fs::remove_file(super::info_path(data_dir));
    }
    std::process::exit(0)
}

fn watch_idle(host: Arc<Host>, data_dir: PathBuf) {
    std::thread::spawn(move || {
        let mut idle = Duration::ZERO;
        loop {
            std::thread::sleep(IDLE_CHECK);
            let busy = host.clients.load(Ordering::SeqCst) > 0 || !lock(&host.panes).is_empty();
            idle = if busy { Duration::ZERO } else { idle + IDLE_CHECK };
            if idle >= IDLE_EXIT {
                exit_host(&data_dir);
            }
        }
    });
}

/// One client connection: handshake, then requests until it disconnects.
fn serve(host: &Arc<Host>, stream: TcpStream, token: &str, data_dir: &Path) -> Result<(), String> {
    let _ = stream.set_nodelay(true);
    stream.set_read_timeout(Some(HELLO_TIMEOUT)).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| format!("no hello: {e}"))?;
    match serde_json::from_str::<Request>(&line) {
        Ok(Request::Hello { token: given, version }) if given == token && version == PROTOCOL => {}
        _ => return Err("bad hello".into()),
    }
    stream.set_read_timeout(None).map_err(|e| e.to_string())?;

    // Every line to this client goes through one writer thread, in order.
    let (tx, rx) = mpsc::channel::<String>();
    let mut out = stream;
    std::thread::Builder::new()
        .name("host-client-out".into())
        .spawn(move || {
            for line in rx {
                if out.write_all(line.as_bytes()).is_err() {
                    break;
                }
            }
            let _ = out.shutdown(std::net::Shutdown::Both);
        })
        .map_err(|e| e.to_string())?;
    let _ = tx.send(encode(&Event::Hello { pid: std::process::id() }));

    let client = host.next_client.fetch_add(1, Ordering::SeqCst);
    host.clients.fetch_add(1, Ordering::SeqCst);
    let mut result = Ok(());
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                result = Err(e.to_string());
                break;
            }
        }
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Request>(&line) {
            Ok(request) => handle(host, client, &tx, request, data_dir),
            Err(e) => eprintln!("bad request: {e}"),
        }
    }
    // Its panes keep running; only the output stops going to it.
    for pane in lock(&host.panes).values() {
        lock(&pane.shared).subscribers.retain(|(id, _)| *id != client);
    }
    host.clients.fetch_sub(1, Ordering::SeqCst);
    result
}

fn handle(host: &Arc<Host>, client: u64, tx: &mpsc::Sender<String>, request: Request, data_dir: &Path) {
    match request {
        Request::Hello { .. } => {}
        Request::Spawn { id, pane, argv, cwd, env, cols, rows, meta } => {
            let spawned = spawn(host, &pane, argv, &cwd, env, (cols, rows), meta);
            match spawned {
                Ok(()) => attach(host, client, tx, id, &pane),
                Err(e) => send(tx, Event::Reply(Reply::error(id, e))),
            }
        }
        Request::Write { pane, data } => {
            if let Some(p) = lock(&host.panes).get(&pane) {
                let _ = p.input.send(data.into_bytes());
            }
        }
        Request::Resize { pane, cols, rows } => {
            if let Some(p) = lock(&host.panes).get(&pane)
                && let Err(e) = p.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            {
                eprintln!("resize {pane}: {e}");
            }
        }
        Request::Kill { pane } => kill(host, &pane),
        Request::List { id } => {
            let panes = lock(&host.panes)
                .iter()
                .map(|(pane, p)| PaneInfo { pane: pane.clone(), run: p.run, meta: p.meta.clone() })
                .collect();
            send(tx, Event::Reply(Reply { id, panes, ..Reply::default() }));
        }
        Request::Attach { id, pane } => attach(host, client, tx, id, &pane),
        Request::Shutdown { id } => {
            let ids: Vec<String> = lock(&host.panes).keys().cloned().collect();
            for pane in ids {
                kill(host, &pane);
            }
            send(tx, Event::Reply(Reply { id, ..Reply::default() }));
            // Lets the writer thread send the reply before the process ends.
            std::thread::sleep(Duration::from_millis(200));
            exit_host(data_dir);
        }
    }
}

fn send(tx: &mpsc::Sender<String>, event: Event) {
    let _ = tx.send(encode(&event));
}

/// Replies with the scrollback and subscribes the client, under the pane's output lock: no chunk is
/// both in the scrollback and streamed, none is lost between the two.
fn attach(host: &Host, client: u64, tx: &mpsc::Sender<String>, id: u64, pane: &str) {
    let panes = lock(&host.panes);
    let Some(p) = panes.get(pane) else {
        return send(tx, Event::Reply(Reply::error(id, "pane not found")));
    };
    let mut shared = lock(&p.shared);
    shared.subscribers.retain(|(c, _)| *c != client);
    shared.subscribers.push((client, tx.clone()));
    let reply = Reply { id, run: p.run, scrollback: shared.scrollback.text(), meta: p.meta.clone(), ..Reply::default() };
    send(tx, Event::Reply(reply));
}

fn kill(host: &Host, pane: &str) {
    // Removed under the lock, killed and dropped outside it (ConPTY close can block).
    let removed = lock(&host.panes).remove(pane);
    if let Some(mut p) = removed {
        let _ = p.child.kill(); // already exited: fine
    }
}

fn spawn(
    host: &Arc<Host>,
    pane: &str,
    argv: Vec<String>,
    cwd: &str,
    env: Vec<(String, String)>,
    (cols, rows): (u16, u16),
    meta: Meta,
) -> Result<(), String> {
    if argv.is_empty() {
        return Err("empty command".into());
    }
    if lock(&host.panes).contains_key(pane) {
        return Err(ALREADY_RUNNING.into());
    }
    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())?;
    let mut cmd = CommandBuilder::from_argv(argv.into_iter().map(OsString::from).collect());
    cmd.env_clear();
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.cwd(cwd);
    let mut child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let job = child.process_id().and_then(crate::pty::job::Job::adopt);
    drop(pair.slave); // the child owns it now
    let started = (|| {
        let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let input = crate::pty::spawn_writer(pane, writer).map_err(|e| e.to_string())?;
        Ok::<_, String>((reader, input))
    })();
    let (reader, input) = match started {
        Ok(io) => io,
        Err(e) => {
            let _ = child.kill();
            return Err(e);
        }
    };
    let run = host.next_run.fetch_add(1, Ordering::SeqCst) + 1;
    let shared = Arc::new(Mutex::new(Shared {
        scrollback: RingBuffer::new(SCROLLBACK_BYTES),
        subscribers: Vec::new(),
    }));
    let mut panes = lock(&host.panes);
    if panes.contains_key(pane) {
        drop(panes);
        let _ = child.kill();
        return Err(ALREADY_RUNNING.into());
    }
    panes.insert(
        pane.to_owned(),
        HostPane {
            run,
            meta,
            master: pair.master,
            input,
            child,
            shared: shared.clone(),
            #[cfg(windows)]
            _job: job,
        },
    );
    drop(panes);
    pump(pane.to_owned(), run, reader, shared);
    watch_exit(host.clone(), pane.to_owned(), run);
    Ok(())
}

/// Reader thread: output -> scrollback + subscribers; `exit` once the pseudoconsole is closed.
fn pump(pane: String, run: u64, mut reader: Box<dyn Read + Send>, shared: Arc<Mutex<Shared>>) {
    let name = format!("host-out-{pane}");
    let started = std::thread::Builder::new().name(name).spawn(move || {
        let mut buf = [0u8; 8192];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            pending.extend_from_slice(&buf[..n]);
            let text = crate::pty::take_valid_utf8(&mut pending, usize::MAX, false);
            broadcast(&shared, &pane, run, text);
        }
        let text = crate::pty::take_valid_utf8(&mut pending, usize::MAX, true);
        broadcast(&shared, &pane, run, text);
        let exit = encode(&Event::Exit { pane, run });
        for (_, tx) in lock(&shared).subscribers.drain(..) {
            let _ = tx.send(exit.clone());
        }
    });
    if let Err(e) = started {
        eprintln!("output thread: {e}");
    }
}

fn broadcast(shared: &Mutex<Shared>, pane: &str, run: u64, text: String) {
    if text.is_empty() {
        return;
    }
    let mut shared = lock(shared);
    shared.scrollback.push(&text);
    let line = encode(&Event::Output { pane: pane.to_owned(), run, data: text });
    // A client gone away is dropped here.
    shared.subscribers.retain(|(_, tx)| tx.send(line.clone()).is_ok());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client over a real socket to an in-process host (no `run`: it exits the process).
    struct TestClient {
        out: TcpStream,
        lines: BufReader<TcpStream>,
    }

    impl TestClient {
        fn connect(host: &Arc<Host>, token: &str) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let addr = listener.local_addr().unwrap();
            let (host, token) = (host.clone(), token.to_owned());
            std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let _ = serve(&host, stream, &token, &std::env::temp_dir());
            });
            let out = TcpStream::connect(addr).unwrap();
            out.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
            let lines = BufReader::new(out.try_clone().unwrap());
            Self { out, lines }
        }

        fn send(&mut self, request: &Request) {
            self.out.write_all(encode(request).as_bytes()).unwrap();
        }

        /// Next event; ConPTY's cursor position query is answered on the way, as a terminal would.
        fn next(&mut self) -> Event {
            let mut line = String::new();
            assert!(self.lines.read_line(&mut line).unwrap() > 0, "host closed the connection");
            let event: Event = serde_json::from_str(&line).unwrap();
            if let Event::Output { pane, data, .. } = &event
                && data.contains("\x1b[6n")
            {
                self.send(&Request::Write { pane: pane.clone(), data: "\x1b[1;1R".into() });
            }
            event
        }

        fn reply(&mut self) -> Reply {
            loop {
                if let Event::Reply(reply) = self.next() {
                    return reply;
                }
            }
        }
    }

    fn long_running() -> Vec<String> {
        let argv: &[&str] = if cfg!(windows) { &["cmd.exe", "/c", "echo ready && ping -n 30 127.0.0.1"] } else { &["sh", "-c", "echo ready; sleep 30"] };
        argv.iter().map(|s| (*s).to_owned()).collect()
    }

    fn spawn_request(id: u64, argv: Vec<String>) -> Request {
        Request::Spawn {
            id,
            pane: "p1".into(),
            argv,
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            env: std::env::vars().collect(),
            cols: 80,
            rows: 24,
            meta: Meta { agent: "shell".into(), cwd: String::new(), title: Some("t".into()) },
        }
    }

    #[test]
    fn rejects_a_wrong_token() {
        let host = Arc::new(Host::default());
        let mut client = TestClient::connect(&host, "secret");
        client.send(&Request::Hello { token: "guess".into(), version: PROTOCOL });
        let mut line = String::new();
        // The host closes without a word.
        assert_eq!(client.lines.read_line(&mut line).unwrap_or(0), 0);
    }

    /// Spawn, output, a second client reattaching with the scrollback, list, kill and exit.
    #[test]
    fn spawn_attach_list_kill() {
        let host = Arc::new(Host::default());
        let mut first = TestClient::connect(&host, "secret");
        first.send(&Request::Hello { token: "secret".into(), version: PROTOCOL });
        assert!(matches!(first.next(), Event::Hello { .. }));

        first.send(&spawn_request(1, Vec::new()));
        assert_eq!(first.reply().error.as_deref(), Some("empty command"));

        first.send(&spawn_request(2, long_running()));
        let spawned = first.reply();
        assert_eq!((spawned.id, spawned.error.as_deref()), (2, None));
        assert!(spawned.run > 0);
        assert_eq!(spawned.meta.title.as_deref(), Some("t"));
        let mut seen = spawned.scrollback.clone();
        while !seen.contains("ready") {
            if let Event::Output { data, run, .. } = first.next() {
                assert_eq!(run, spawned.run);
                seen.push_str(&data);
            }
        }

        first.send(&spawn_request(3, long_running()));
        assert_eq!(first.reply().error.as_deref(), Some(ALREADY_RUNNING));

        // The app restarted: a new client gets what was printed, then the live stream.
        let mut second = TestClient::connect(&host, "secret");
        second.send(&Request::Hello { token: "secret".into(), version: PROTOCOL });
        assert!(matches!(second.next(), Event::Hello { .. }));
        second.send(&Request::List { id: 4 });
        let list = second.reply();
        assert_eq!(list.panes.len(), 1);
        assert_eq!((list.panes[0].pane.as_str(), list.panes[0].run), ("p1", spawned.run));
        second.send(&Request::Attach { id: 5, pane: "p1".into() });
        let attached = second.reply();
        assert_eq!(attached.run, spawned.run);
        assert!(attached.scrollback.contains("ready"));
        second.send(&Request::Attach { id: 6, pane: "nope".into() });
        assert_eq!(second.reply().error.as_deref(), Some("pane not found"));

        second.send(&Request::Kill { pane: "p1".into() });
        loop {
            if let Event::Exit { pane, run } = second.next() {
                assert_eq!((pane.as_str(), run), ("p1", spawned.run));
                break;
            }
        }
        assert!(lock(&host.panes).is_empty());
    }
}

/// Polls the child; on exit drops the pane (closing the pseudoconsole ends the reader thread).
fn watch_exit(host: Arc<Host>, pane: String, run: u64) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(EXIT_POLL);
            let mut panes = lock(&host.panes);
            let exited = match panes.get_mut(&pane) {
                Some(p) if p.run == run => !matches!(p.child.try_wait(), Ok(None)),
                _ => return, // killed, or replaced by a newer run
            };
            if exited {
                let p = panes.remove(&pane);
                drop(panes);
                drop(p);
                return;
            }
        }
    });
}
