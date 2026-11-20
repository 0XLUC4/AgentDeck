//! SSH hosts (Settings > Hosts): saved machines agents run on, through the system `ssh` binary.
//! Nothing secret is stored: authentication is the user's keys, ssh-agent and `~/.ssh/config`.
//! Agent panes run inside tmux on the host, so they survive a dropped connection (laptop asleep)
//! and a restart of the pane reattaches to the same session.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use portable_pty::CommandBuilder;
use serde::{Deserialize, Serialize};

use crate::agents::AgentKind;
use crate::logic::deck::new_id;
use crate::persist::JsonStore;
use crate::pty::{login_run, sh_quote, ssh_target};

pub const HOSTS_FILE: &str = "hosts.json";
pub const HOSTS_KEY: &str = "hosts";
/// Env key carrying a pane's [`Host`] (JSON) to `pty::spawn_pane` (never given to the process).
pub const HOST_ENV: &str = "AGENTDECK_SSH_HOST";
/// Remote files bigger than this are not opened.
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
/// Never prompt (no terminal to answer in), give up quickly on a dead host.
const ONE_SHOT_OPTS: [&str; 8] = [
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=5",
    "-o",
    "ServerAliveInterval=5",
    "-o",
    "ServerAliveCountMax=2",
];
/// A pane notices a dead connection (laptop woke up) in under a minute instead of hanging.
const PANE_OPTS: [&str; 2] = ["-o", "ServerAliveInterval=15"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub id: String,
    pub name: String,
    /// `user@host`, or an alias of `~/.ssh/config`.
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Private key file (`ssh -i`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// Default remote folder of new panes ("" = the remote home).
    #[serde(default)]
    pub folder: String,
}

/// One entry of a remote folder listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    pub name: String,
    pub dir: bool,
}

impl Host {
    pub fn new(name: &str, target: &str) -> Self {
        Self {
            id: new_id(),
            name: name.trim().to_owned(),
            target: target.trim().to_owned(),
            port: None,
            identity: None,
            folder: String::new(),
        }
    }

    /// Everything ends up on an ssh command line: plain characters only, no option injection.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Name the host.".into());
        }
        if self.target.contains(':') || ssh_target(&self.target).is_none() {
            return Err("Target: user@host (letters, digits, . _ - @).".into());
        }
        if !valid_folder(&self.folder) {
            return Err("Folder: letters, digits and / . _ - ~ only.".into());
        }
        if self.port == Some(0) {
            return Err("Port: 1 to 65535.".into());
        }
        if let Some(key) = &self.identity
            && (key.trim().is_empty() || key.starts_with('-') || key.contains(char::is_control))
        {
            return Err("Identity file: a path to a private key.".into());
        }
        Ok(())
    }

    /// `pane.ssh` of a pane on this host in `folder`.
    pub fn ssh_spec(&self, folder: &str) -> String {
        if folder.is_empty() { self.target.clone() } else { format!("{}:{folder}", self.target) }
    }

    /// `[-p port] [-i key] target`: how every ssh call reaches this host.
    fn connect_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(port) = self.port {
            args.extend(["-p".to_owned(), port.to_string()]);
        }
        if let Some(key) = &self.identity {
            args.extend(["-i".to_owned(), key.clone()]);
        }
        args.push(self.target.clone());
        args
    }

    /// Argv of an agent pane: `ssh -t [-p port] [-i key] target <remote_agent_command>`.
    pub fn agent_argv(&self, agent: AgentKind, dir: &str, args: &[String], pane_id: &str) -> Vec<String> {
        let mut argv = vec!["ssh".to_owned(), "-t".to_owned()];
        argv.extend(PANE_OPTS.map(String::from));
        argv.extend(self.connect_args());
        argv.push(remote_agent_command(agent, dir, args, pane_id));
        argv
    }

    /// [`Host::agent_argv`] for the PTY.
    pub fn agent_command(&self, agent: AgentKind, dir: &str, args: &[String], pane_id: &str) -> CommandBuilder {
        let argv = self.agent_argv(agent, dir, args, pane_id);
        let mut cmd = CommandBuilder::new(&argv[0]);
        cmd.args(&argv[1..]);
        cmd
    }

    /// Runs `remote` on the host (blocking, never prompts). Returns its stdout.
    fn run(&self, remote: &str, stdin: Option<&[u8]>) -> Result<Vec<u8>, String> {
        let mut cmd = Command::new("ssh");
        cmd.args(ONE_SHOT_OPTS).args(self.connect_args()).arg(remote);
        cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().map_err(|e| format!("ssh: {e} (is OpenSSH installed?)"))?;
        if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(bytes).map_err(|e| format!("ssh: {e}"))?;
        } // pipe dropped: the remote `cat` sees EOF
        let out = child.wait_with_output().map_err(|e| format!("ssh: {e}"))?;
        if out.status.success() {
            return Ok(out.stdout);
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let reason = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        Err(if reason.is_empty() { format!("ssh failed ({})", out.status) } else { reason.to_owned() })
    }

    /// `ssh -o BatchMode=yes -o ConnectTimeout=5 <target> echo ok`.
    pub fn test_connection(&self) -> Result<(), String> {
        let out = self.run("echo ok", None)?;
        (String::from_utf8_lossy(&out).trim() == "ok")
            .then_some(())
            .ok_or_else(|| "unexpected answer from the host".into())
    }

    pub fn has_tmux(&self) -> Result<bool, String> {
        let out = self.run("command -v tmux >/dev/null 2>&1 && echo yes || echo no", None)?;
        Ok(String::from_utf8_lossy(&out).trim() == "yes")
    }

    /// Ends the tmux session of a closed pane (and its agent).
    pub fn kill_session(&self, pane_id: &str) -> Result<(), String> {
        let remote = format!("tmux kill-session -t {} 2>/dev/null; true", session_name(pane_id));
        self.run(&remote, None).map(drop)
    }

    /// Absolute path of `dir` (resolved on the host) and its entries, folders first.
    pub fn list_dir(&self, dir: &str) -> Result<(String, Vec<RemoteEntry>), String> {
        let out = self.run(&format!("cd {} && pwd && ls -1Ap", remote_path(dir)), None)?;
        parse_listing(&String::from_utf8_lossy(&out)).ok_or_else(|| "empty answer from the host".into())
    }

    pub fn read_file(&self, path: &str) -> Result<String, String> {
        let remote = format!("head -c {} {}", MAX_FILE_BYTES + 1, remote_path(path));
        let bytes = self.run(&remote, None)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(format!("larger than {} MB", MAX_FILE_BYTES / (1024 * 1024)));
        }
        String::from_utf8(bytes).map_err(|_| "not a text file".into())
    }

    /// Replaces the file with `text` (`cat > file`, content through stdin).
    pub fn write_file(&self, path: &str, text: &str) -> Result<(), String> {
        self.run(&format!("cat > {}", remote_path(path)), Some(text.as_bytes())).map(drop)
    }

    /// `git worktree add` on the host (see [`remote_worktree_command`]). Returns the worktree's
    /// absolute path, usable as a pane folder on this host.
    pub fn create_worktree(&self, dir: &str, short: &str, branch: &str, base: &str) -> Result<String, String> {
        let out = self.run(&remote_worktree_command(dir, short, branch, base)?, None)?;
        let out = String::from_utf8_lossy(&out);
        let path = out.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_owned();
        if path.is_empty() || !valid_folder(&path) {
            return Err(format!("unexpected worktree folder on the host: {path:?}"));
        }
        Ok(path)
    }
}

/// Remote shell command: in the repository of `dir`, worktree `<repo>-agentdeck-<short>` next to it,
/// on `branch` when it exists (local, or on `origin`), else on a new `branch` from `base` ("" = HEAD).
/// Prints the worktree's path last.
pub fn remote_worktree_command(dir: &str, short: &str, branch: &str, base: &str) -> Result<String, String> {
    use crate::logic::new_worktree::valid_branch;
    let base = if base.trim().is_empty() { "HEAD" } else { base.trim() };
    if !valid_branch(branch) || (base != "HEAD" && !valid_branch(base)) {
        return Err("branch names: letters, digits and - _ . / only".into());
    }
    if short.is_empty() || !short.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("invalid worktree id".into());
    }
    let (b, local, remote) =
        (sh_quote(branch), sh_quote(&format!("refs/heads/{branch}")), sh_quote(&format!("refs/remotes/origin/{branch}")));
    Ok(format!(
        "cd {} && root=$(git rev-parse --show-toplevel) && dest=\"$(dirname \"$root\")/$(basename \"$root\")-agentdeck-{short}\" && \
         {{ if git -C \"$root\" show-ref --verify --quiet {local} || git -C \"$root\" show-ref --verify --quiet {remote}; \
         then git -C \"$root\" worktree add \"$dest\" {b}; else git -C \"$root\" worktree add -b {b} \"$dest\" {}; fi; }} >&2 && echo \"$dest\"",
        remote_path(dir),
        sh_quote(base),
    ))
}

/// A folder the remote shell takes as is (same rule as `pty::ssh_target`).
pub fn valid_folder(folder: &str) -> bool {
    ssh_target(&format!("x:{folder}")).is_some()
}

/// tmux session of a pane on a host.
pub fn session_name(pane_id: &str) -> String {
    format!("agentdeck-{pane_id}")
}

/// The agent in `dir` inside tmux session `agentdeck-<pane id>` (`new -A`: attach when it
/// exists). Without tmux it runs directly, after a notice. POSIX `sh` whatever the login shell;
/// the agent itself still goes through the login shell (its PATH).
pub fn remote_agent_command(agent: AgentKind, dir: &str, args: &[String], pane_id: &str) -> String {
    let cd = if dir.is_empty() { String::new() } else { format!("cd {dir} || exit 1; ") };
    let script = format!(
        "{cd}if command -v tmux >/dev/null 2>&1; then exec tmux new -A -s {} \"$0\"; fi; \
         echo 'agentdeck: tmux not found, this agent stops when the connection drops'; eval \"$0\"",
        session_name(pane_id)
    );
    format!("sh -c {} {}", sh_quote(&script), sh_quote(&login_run(agent, args)))
}

/// `path` for the remote shell: quoted, except a leading `~` (expanded by the shell).
pub fn remote_path(path: &str) -> String {
    match path {
        "" | "~" => "~".to_owned(),
        _ => match path.strip_prefix("~/") {
            Some(rest) => format!("~/{}", sh_quote(rest)),
            None => sh_quote(path),
        },
    }
}

/// `pwd` then `ls -1Ap` output: the folder and its entries (folders end with `/`), folders first.
pub fn parse_listing(out: &str) -> Option<(String, Vec<RemoteEntry>)> {
    let mut lines = out.lines();
    let cwd = lines.next()?.trim_end().to_owned();
    let mut entries: Vec<RemoteEntry> = lines
        .filter(|l| !l.is_empty())
        .map(|l| match l.strip_suffix('/') {
            Some(name) => RemoteEntry { name: name.to_owned(), dir: true },
            None => RemoteEntry { name: l.to_owned(), dir: false },
        })
        .collect();
    entries.sort_by_cached_key(|e| (!e.dir, e.name.to_lowercase()));
    Some((cwd, entries))
}

/// `name` inside remote folder `dir` (POSIX separators).
pub fn join_path(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

/// Parent of a remote folder ("/" stays "/").
pub fn parent_path(dir: &str) -> String {
    match dir.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) | None => "/".to_owned(),
        Some((parent, _)) => parent.to_owned(),
    }
}

/// Saved hosts; a missing file is none, one unreadable entry costs only itself.
pub fn load_hosts(dir: &Path) -> Result<Vec<Host>, String> {
    let value = JsonStore::load(dir, HOSTS_FILE)?.get(HOSTS_KEY);
    Ok(value
        .as_array()
        .map(|list| list.iter().filter_map(|h| serde_json::from_value(h.clone()).ok()).collect())
        .unwrap_or_default())
}

pub fn save_hosts(dir: &Path, hosts: &[Host]) -> Result<(), String> {
    let mut store = JsonStore::load(dir, HOSTS_FILE)?;
    store.set(HOSTS_KEY, serde_json::to_value(hosts).map_err(|e| e.to_string())?);
    store.save()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Host {
        Host {
            port: Some(2222),
            identity: Some("C:\\Users\\me\\.ssh\\id_ed25519".into()),
            folder: "~/app".into(),
            ..Host::new("Box", "luca@box")
        }
    }

    #[test]
    fn validates_hosts() {
        assert_eq!(host().validate(), Ok(()));
        let bad = [
            Host { name: " ".into(), ..host() },
            Host { target: "-oProxyCommand=x".into(), ..host() },
            Host { target: "box:/srv".into(), ..host() },
            Host { target: "a;b".into(), ..host() },
            Host { folder: "/a b".into(), ..host() },
            Host { folder: "$(id)".into(), ..host() },
            Host { port: Some(0), ..host() },
            Host { identity: Some("-oX".into()), ..host() },
        ];
        for h in bad {
            assert!(h.validate().is_err(), "{h:?}");
        }
    }

    #[test]
    fn builds_the_agent_command() {
        let argv = host().agent_argv(AgentKind::Claude, "~/app", &["--model".into(), "opus".into()], "p-1");
        assert_eq!(
            argv[..9],
            ["ssh", "-t", "-o", "ServerAliveInterval=15", "-p", "2222", "-i", "C:\\Users\\me\\.ssh\\id_ed25519", "luca@box"]
        );
        let remote = &argv[9];
        assert!(remote.starts_with("sh -c 'cd ~/app || exit 1; if command -v tmux"), "{remote}");
        assert!(remote.contains("exec tmux new -A -s agentdeck-p-1 \"$0\""), "{remote}");
        assert!(remote.ends_with(&format!(" {}", sh_quote(&login_run(AgentKind::Claude, &["--model".into(), "opus".into()])))));
        let plain = Host::new("Box", "box").agent_argv(AgentKind::Shell, "", &[], "p");
        assert_eq!(plain[..5], ["ssh", "-t", "-o", "ServerAliveInterval=15", "box"]);
        assert!(plain[5].starts_with("sh -c 'if command -v tmux"));
    }

    #[test]
    fn quotes_remote_paths() {
        assert_eq!(remote_path(""), "~");
        assert_eq!(remote_path("~/my dir"), "~/'my dir'");
        assert_eq!(remote_path("/srv/it's"), "'/srv/it'\\''s'");
    }

    #[test]
    fn parses_listings_and_paths() {
        let (cwd, entries) = parse_listing("/home/luca\nsrc/\nb.txt\nA.md\n.git/\n").unwrap();
        assert_eq!(cwd, "/home/luca");
        let names: Vec<(&str, bool)> = entries.iter().map(|e| (e.name.as_str(), e.dir)).collect();
        assert_eq!(names, [(".git", true), ("src", true), ("A.md", false), ("b.txt", false)]);
        assert_eq!(parse_listing(""), None);
        assert_eq!(join_path("/", "etc"), "/etc");
        assert_eq!(join_path("/home/luca", "a"), "/home/luca/a");
        assert_eq!(parent_path("/home/luca"), "/home");
        assert_eq!(parent_path("/home"), "/");
        assert_eq!(parent_path("/"), "/");
    }

    #[test]
    fn builds_the_worktree_command() {
        let cmd = remote_worktree_command("~/app", "ab12cd34", "feat/x", "").unwrap();
        assert!(cmd.starts_with("cd ~/'app' && root=$(git rev-parse --show-toplevel)"), "{cmd}");
        assert!(cmd.contains("-agentdeck-ab12cd34\""), "{cmd}");
        assert!(cmd.contains("show-ref --verify --quiet 'refs/heads/feat/x'"), "{cmd}");
        assert!(cmd.contains("worktree add -b 'feat/x' \"$dest\" 'HEAD'"), "{cmd}");
        assert!(cmd.ends_with(">&2 && echo \"$dest\""), "{cmd}");
        assert!(remote_worktree_command("~", "ab12", "$(id)", "").is_err());
        assert!(remote_worktree_command("~", "ab12", "x", "a b").is_err());
        assert!(remote_worktree_command("~", "a;b", "x", "").is_err());
    }

    #[test]
    fn saves_and_loads_hosts() {
        let dir = std::env::temp_dir().join(format!("agentdeck-hosts-{}", std::process::id()));
        assert_eq!(load_hosts(&dir), Ok(Vec::new()));
        let hosts = vec![host(), Host::new("Other", "other")];
        save_hosts(&dir, &hosts).unwrap();
        assert_eq!(load_hosts(&dir), Ok(hosts));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
