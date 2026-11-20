//! Session host: a small background process (`agentdeck --session-host <data dir>`) that owns the
//! agents' pseudoconsoles, so quitting AgentDeck leaves them running. The app reattaches on its next
//! start and replays what they printed meanwhile.
//!
//! IPC: TCP on 127.0.0.1, port and secret token in `session-host.json` in the data folder (like
//! `api.json` for the command line). One JSON message per line ([`Request`] up, [`Event`] down);
//! the first line must be a [`Request::Hello`] with the token and [`PROTOCOL`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub mod client;
pub mod server;

pub use client::{client, connect_existing, connect_or_start, enabled, set_enabled, shutdown};

/// Hidden command line switch that runs the host instead of the app.
pub const HOST_ARG: &str = "--session-host";
/// `{port, token, pid}` of the running host, in the data folder.
pub const INFO_FILE: &str = "session-host.json";
/// The host's stderr (truncated at each host start).
pub const LOG_FILE: &str = "session-host.log";
/// Bumped on any incompatible message change: a host of another version is not used.
pub const PROTOCOL: u32 = 1;
/// Output kept per pane for a reattach (the app's own buffer keeps 1 MB of it).
pub const SCROLLBACK_BYTES: usize = 2 * 1024 * 1024;
/// Same text as `pty::spawn_pane`'s: the runtime adopts the live process on this error.
pub const ALREADY_RUNNING: &str = "pane id already in use";

/// Where the host listens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Info {
    pub port: u16,
    pub token: String,
    pub pid: u32,
}

pub fn info_path(data_dir: &Path) -> PathBuf {
    data_dir.join(INFO_FILE)
}

pub fn read_info(data_dir: &Path) -> Result<Info, String> {
    let text = std::fs::read_to_string(info_path(data_dir)).map_err(|e| format!("no session host: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("unreadable {INFO_FILE}: {e}"))
}

/// What the app shows for a pane; kept by the host so a reattach can restart its history recording.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub title: Option<String>,
}

/// A live pane of the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneInfo {
    pub pane: String,
    pub run: u64,
    #[serde(default)]
    pub meta: Meta,
}

/// App -> host. `id` is echoed in the [`Event::Reply`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Request {
    Hello { token: String, version: u32 },
    /// Starts `argv` (full env given, nothing inherited) and attaches this client to its output.
    Spawn {
        id: u64,
        pane: String,
        argv: Vec<String>,
        cwd: String,
        env: Vec<(String, String)>,
        cols: u16,
        rows: u16,
        meta: Meta,
    },
    Write { pane: String, data: String },
    Resize { pane: String, cols: u16, rows: u16 },
    /// Kills the pane's whole process tree. No reply; its `exit` event follows.
    Kill { pane: String },
    List { id: u64 },
    /// Replies with the scrollback, then streams the pane's output to this client.
    Attach { id: u64, pane: String },
    /// Kills every pane, replies, and ends the host ("Quit and stop all agents", update).
    Shutdown { id: u64 },
}

/// Answer to a request: `error`, or the fields that request fills.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Spawn / attach: the host's run number of the process (output and exit carry it).
    #[serde(default)]
    pub run: u64,
    /// Spawn / attach: output kept so far, to replay before the live stream.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scrollback: String,
    #[serde(default)]
    pub meta: Meta,
    /// List.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panes: Vec<PaneInfo>,
}

impl Reply {
    pub fn error(id: u64, error: impl Into<String>) -> Self {
        Self { id, error: Some(error.into()), ..Self::default() }
    }

    pub fn into_result(self) -> Result<Self, String> {
        match self.error.clone() {
            Some(e) => Err(e),
            None => Ok(self),
        }
    }
}

/// Host -> app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "camelCase")]
pub enum Event {
    Hello { pid: u32 },
    Reply(Reply),
    Output { pane: String, run: u64, data: String },
    /// The process ended (or was killed); sent after its last output.
    Exit { pane: String, run: u64 },
}

/// One protocol line, newline included.
pub fn encode<T: Serialize>(message: &T) -> String {
    // Our own types always serialize; an empty line is skipped by the reader anyway.
    let mut line = serde_json::to_string(message).unwrap_or_default();
    line.push('\n');
    line
}

/// `agentdeck --session-host <data dir>`: runs the host and returns its exit code. None: not the host.
pub fn run_from_args() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(HOST_ARG) {
        return None;
    }
    let Some(dir) = args.next().map(PathBuf::from) else {
        eprintln!("usage: agentdeck {HOST_ARG} <data dir>");
        return Some(2);
    };
    Some(match server::run(&dir) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("session host: {e}");
            1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_as_tagged_lines() {
        let spawn = Request::Spawn {
            id: 7,
            pane: "p1".into(),
            argv: vec!["cmd.exe".into(), "/c".into(), "echo hi".into()],
            cwd: "D:/x".into(),
            env: vec![("A".into(), "1".into())],
            cols: 80,
            rows: 24,
            meta: Meta { agent: "shell".into(), cwd: "D:/x".into(), title: None },
        };
        let line = encode(&spawn);
        assert!(line.ends_with('\n') && line.starts_with(r#"{"op":"spawn""#));
        assert_eq!(serde_json::from_str::<Request>(line.trim()).unwrap(), spawn);
        let kill: Request = serde_json::from_str(r#"{"op":"kill","pane":"p1"}"#).unwrap();
        assert_eq!(kill, Request::Kill { pane: "p1".into() });
        assert!(serde_json::from_str::<Request>(r#"{"op":"format","pane":"p1"}"#).is_err());
    }

    #[test]
    fn events_round_trip_and_replies_default_missing_fields() {
        let output = Event::Output { pane: "p".into(), run: 3, data: "\u{1b}[31mé\r\n".into() };
        assert_eq!(serde_json::from_str::<Event>(encode(&output).trim()).unwrap(), output);
        let reply: Event = serde_json::from_str(r#"{"ev":"reply","id":4,"run":9}"#).unwrap();
        let Event::Reply(reply) = reply else { panic!("not a reply") };
        assert_eq!((reply.id, reply.run, reply.scrollback.as_str()), (4, 9, ""));
        assert!(reply.clone().into_result().is_ok());
        assert_eq!(Reply::error(4, "boom").into_result(), Err("boom".to_owned()));
    }

    #[test]
    fn info_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("agentdeck-host-info-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_info(&dir).is_err());
        let info = Info { port: 4100, token: "t".into(), pid: 12 };
        std::fs::write(info_path(&dir), serde_json::to_string(&info).unwrap()).unwrap();
        assert_eq!(read_info(&dir).unwrap(), info);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
