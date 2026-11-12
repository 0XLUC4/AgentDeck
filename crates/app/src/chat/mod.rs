//! Chat panes: the agent runs headless behind a message list instead of a terminal.
//! Claude: one persistent `claude -p --input-format stream-json --output-format stream-json`
//! process per pane. Codex: one `codex exec --json` process per turn, resumed by thread id.
//!
//! Wiring: `panes::runtime` hands a chat pane's spawn to [`spawn`] (same args, MCP servers, env and
//! hooks as a terminal pane) and routes prompts, interrupts and kills here. The layout shows
//! [`view`] in place of the terminal. Claude's status comes from its hooks, as in a terminal;
//! Codex's comes from here (`agent-status` + `mcp::STATUS`, so the orchestrator's `wait_for` works).

mod view;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use agentdeck_core::agents::AgentKind;
use agentdeck_core::chat::{ChatEvent, Transcript, claude_interrupt_line, claude_user_line, parse_claude, parse_codex};
use agentdeck_core::hooks_server::{AgentStatusEvent, PaneStatus};
use agentdeck_core::logic::deck::Pane;
use agentdeck_core::{AppHandle, mcp, pty};
use gpui_kit::*;

pub use view::view;

/// Kept from the end of stderr, shown when a process dies mid-turn.
const STDERR_TAIL: usize = 2048;

/// Claude flags of a chat pane (prompts in, events out, streamed text).
const CLAUDE_ARGS: [&str; 7] = [
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--include-partial-messages",
];

/// How to start the agent again (Codex starts one process per turn).
#[derive(Clone)]
struct Launch {
    backend: AppHandle,
    agent: AgentKind,
    cwd: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    trust: bool,
}

struct Proc {
    child: Child,
    /// Claude only: prompts and interrupts go here. Codex gets its prompt and EOF at spawn.
    stdin: Option<ChildStdin>,
    run: u64,
    /// Kill-on-close job: the agent's MCP servers and shells die with it.
    #[cfg(windows)]
    _job: Option<pty::job::Job>,
}

#[derive(Default)]
struct Chat {
    transcript: Transcript,
    launch: Option<Launch>,
    proc: Option<Proc>,
    /// Prompts waiting for the process (still spawning, asleep) or, for Codex, for the running turn.
    pending: Vec<String>,
}

#[derive(Default)]
struct Chats {
    map: HashMap<String, Chat>,
    next_run: u64,
}

static CHATS: LazyLock<Mutex<Chats>> = LazyLock::new(Mutex::default);
/// Pane ids whose transcript changed, drained by the views' repaint task.
static CHANGED: LazyLock<(smol::channel::Sender<String>, smol::channel::Receiver<String>)> =
    LazyLock::new(smol::channel::unbounded);

fn chats() -> MutexGuard<'static, Chats> {
    CHATS.lock().unwrap_or_else(PoisonError::into_inner)
}

fn changed(pane_id: &str) {
    let _ = CHANGED.0.try_send(pane_id.to_owned());
}

/// Copy of the pane's transcript for rendering.
fn transcript(pane_id: &str) -> Transcript {
    chats().map.get(pane_id).map(|c| c.transcript.clone()).unwrap_or_default()
}

fn has_flag(args: &[String], flags: &[&str]) -> bool {
    args.iter().any(|a| flags.iter().any(|f| a == f || a.starts_with(&format!("{f}="))))
}

/// Blocking (runs in `run_spawn`'s background task): records how to launch the agent and, for
/// Claude, starts its process. `args` and `env` are the terminal pane's (browser, orchestrator,
/// --resume, user args).
pub fn spawn(backend: &AppHandle, pane: &Pane, mut args: Vec<String>, env: HashMap<String, String>, trust: bool) -> Result<(), String> {
    if !matches!(pane.agent, AgentKind::Claude | AgentKind::Codex) {
        return Err("chat mode supports Claude and Codex only".into());
    }
    let cwd = if pane.worktree {
        agentdeck_core::agents::create_worktree(&pane.cwd, &pane.id)?
    } else {
        pane.cwd.clone()
    };
    if pane.agent == AgentKind::Claude {
        args.extend(CLAUDE_ARGS.map(String::from));
        // -p cannot show a permission prompt: allow edits unless the user chose a mode.
        if !has_flag(&args, &["--permission-mode", "--dangerously-skip-permissions"]) {
            args.extend(["--permission-mode".into(), "acceptEdits".into()]);
        }
    }
    let launch = Launch { backend: backend.clone(), agent: pane.agent, cwd, args, env, trust };
    let mut chats = chats();
    let chat = chats.map.entry(pane.id.clone()).or_default();
    // No conversation to continue (first start, restart): a fresh transcript.
    if pane.resume_id.is_none() {
        chat.transcript = Transcript::default();
    }
    if chat.transcript.session.is_none() {
        chat.transcript.session = pane.resume_id.clone();
    }
    chat.transcript.busy = false;
    chat.launch = Some(launch);
    drop(chats);
    changed(&pane.id);
    match pane.agent {
        AgentKind::Claude => start_claude(&pane.id),
        _ => {
            next_codex_turn(&pane.id);
            Ok(())
        }
    }
}

fn start_claude(pane_id: &str) -> Result<(), String> {
    let mut chats = chats();
    chats.next_run += 1;
    let run = chats.next_run;
    let chat = chats.map.get_mut(pane_id).ok_or("chat pane closed")?;
    let launch = chat.launch.clone().ok_or("chat not launched")?;
    let mut proc = start_process(&launch, pane_id, &launch.args, run)?;
    for text in std::mem::take(&mut chat.pending) {
        write_line(&mut proc, &claude_user_line(&text));
        chat.transcript.push_user(&text);
    }
    chat.proc = Some(proc);
    Ok(())
}

/// Starts the next queued Codex prompt, if none runs. Errors end up in the transcript.
fn next_codex_turn(pane_id: &str) {
    let mut chats = chats();
    chats.next_run += 1;
    let run = chats.next_run;
    let Some(chat) = chats.map.get_mut(pane_id) else { return };
    let Some(launch) = chat.launch.clone() else { return };
    if chat.proc.is_some() || chat.pending.is_empty() {
        return;
    }
    let text = chat.pending.remove(0);
    let mut args = vec!["exec".into(), "--json".into(), "--skip-git-repo-check".into()];
    args.extend(launch.args.iter().cloned());
    // exec cannot ask for approval: work in the folder unless the user chose a sandbox.
    if !has_flag(&args, &["-s", "--sandbox", "--full-auto", "--dangerously-bypass-approvals-and-sandbox", "--yolo"]) {
        args.extend(["-s".into(), "workspace-write".into()]);
    }
    if let Some(session) = &chat.transcript.session {
        args.extend(["resume".into(), session.clone()]);
    }
    args.push("-".into()); // the prompt comes on stdin: no quoting through cmd.exe
    chat.transcript.push_user(&text);
    let session = chat.transcript.session.clone();
    match start_process(&launch, pane_id, &args, run) {
        Ok(mut proc) => {
            // Dropped right after: EOF ends the prompt.
            if let Some(mut stdin) = proc.stdin.take()
                && let Err(e) = stdin.write_all(text.as_bytes())
            {
                eprintln!("chat {pane_id}: prompt not written: {e}");
            }
            chat.proc = Some(proc);
            drop(chats);
            report(&launch.backend, pane_id, PaneStatus::Working, None, Some(text), None, session);
        }
        Err(e) => {
            chat.transcript.apply(ChatEvent::TurnEnd { error: Some(e.clone()) });
            drop(chats);
            report(&launch.backend, pane_id, PaneStatus::Error, Some(e), None, None, session);
        }
    }
    changed(pane_id);
}

fn start_process(launch: &Launch, pane_id: &str, args: &[String], run: u64) -> Result<Proc, String> {
    let mut cmd = pty::pipe_command(&launch.backend, pane_id, launch.agent, &launch.cwd, args, &launch.env, launch.trust)?;
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", launch.agent.binary().unwrap_or("agent")))?;
    #[cfg(windows)]
    let job = pty::job::Job::adopt(child.id());
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    let tail = Arc::new(Mutex::new(String::new()));
    let err_tail = tail.clone();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut text);
        let start = text.len().saturating_sub(STDERR_TAIL);
        let start = (start..text.len()).find(|&i| text.is_char_boundary(i)).unwrap_or(text.len());
        *err_tail.lock().unwrap_or_else(PoisonError::into_inner) = text[start..].trim().to_owned();
    });
    let (id, agent, backend) = (pane_id.to_owned(), launch.agent, launch.backend.clone());
    std::thread::Builder::new()
        .name(format!("chat-{pane_id}"))
        .spawn(move || read_events(&backend, &id, agent, run, stdout, &tail))
        .map_err(|e| e.to_string())?;
    Ok(Proc {
        stdin: child.stdin.take(),
        child,
        run,
        #[cfg(windows)]
        _job: job,
    })
}

fn write_line(proc: &mut Proc, line: &str) {
    if let Some(stdin) = proc.stdin.as_mut()
        && let Err(e) = writeln!(stdin, "{line}").and_then(|()| stdin.flush())
    {
        eprintln!("chat input: {e}");
    }
}

/// Reader thread of one process: applies its events, reports Codex turns, then handles the exit.
fn read_events(backend: &AppHandle, pane_id: &str, agent: AgentKind, run: u64, stdout: impl Read, tail: &Mutex<String>) {
    let parse = if agent == AgentKind::Claude { parse_claude } else { parse_codex };
    let current = |chats: &mut Chats| chats.map.get(pane_id).is_some_and(|c| c.proc.as_ref().is_some_and(|p| p.run == run));
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        let events = parse(&line);
        if events.is_empty() {
            continue;
        }
        let mut chats = chats();
        if !current(&mut chats) {
            return; // killed or replaced
        }
        let Some(chat) = chats.map.get_mut(pane_id) else { return };
        let mut ended = None;
        for event in events {
            if let ChatEvent::TurnEnd { error } = &event {
                ended = Some(error.clone());
            }
            chat.transcript.apply(event);
        }
        let (answer, session) = (chat.transcript.last_answer().map(str::to_owned), chat.transcript.session.clone());
        drop(chats);
        // Claude's own hooks report its turns; Codex's turns are reported here.
        if agent == AgentKind::Codex
            && let Some(error) = ended
        {
            let status = if error.is_some() { PaneStatus::Error } else { PaneStatus::Done };
            report(backend, pane_id, status, error, None, answer, session);
        }
        changed(pane_id);
    }
    // EOF: the process is gone (or about to be).
    let mut chats = chats();
    if !current(&mut chats) {
        return;
    }
    let Some(chat) = chats.map.get_mut(pane_id) else { return };
    if let Some(mut proc) = chat.proc.take() {
        let _ = proc.child.wait();
    }
    if chat.transcript.busy {
        let error = std::mem::take(&mut *tail.lock().unwrap_or_else(PoisonError::into_inner));
        let error = if error.is_empty() { "The agent stopped.".to_owned() } else { error };
        chat.transcript.apply(ChatEvent::TurnEnd { error: Some(error.clone()) });
        let session = chat.transcript.session.clone();
        drop(chats);
        if agent == AgentKind::Codex {
            report(backend, pane_id, PaneStatus::Error, Some(error), None, None, session);
        }
    } else {
        drop(chats);
    }
    changed(pane_id);
    match agent {
        // Claude's process is the pane's: its exit is the pane's, as for a terminal.
        AgentKind::Claude => {
            let _ = backend.emit(&format!("pane-exit:{pane_id}"), ());
        }
        _ => next_codex_turn(pane_id),
    }
}

/// Status of a Codex chat turn, through the same path as a hook (deck, notifications, journal, `wait_for`).
fn report(
    backend: &AppHandle,
    pane_id: &str,
    status: PaneStatus,
    last_action: Option<String>,
    prompt: Option<String>,
    summary: Option<String>,
    session_id: Option<String>,
) {
    mcp::STATUS.set(pane_id, status);
    let event = match status {
        PaneStatus::Working => "UserPromptSubmit",
        PaneStatus::Error => "StopFailure",
        _ => "Stop",
    };
    let payload = AgentStatusEvent {
        pane_id: pane_id.to_owned(),
        status,
        last_action,
        event: event.into(),
        prompt,
        summary,
        session_id,
        edited_file: None,
    };
    if let Err(e) = backend.emit("agent-status", payload) {
        eprintln!("chat status: {e}");
    }
}

/// Sends a prompt (chat input, orchestrator, queue). An exited pane is woken (asleep) or restarted
/// first; the prompt waits for its process.
pub fn send(pane: &Pane, text: &str, cx: &mut App) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    {
        let mut chats = chats();
        let chat = chats.map.entry(pane.id.clone()).or_default();
        match chat.proc.as_mut() {
            Some(proc) if pane.agent == AgentKind::Claude && !pane.exited => {
                write_line(proc, &claude_user_line(text));
                chat.transcript.push_user(text);
            }
            _ => chat.pending.push(text.to_owned()),
        }
    }
    changed(&pane.id);
    if pane.exited {
        if crate::panes::runtime::is_asleep(&pane.id, cx) {
            crate::panes::runtime::wake_pane(&pane.id, cx);
        } else {
            crate::panes::runtime::restart_pane(&pane.id, cx);
        }
    } else if pane.agent == AgentKind::Codex {
        let id = pane.id.clone();
        cx.background_spawn(async move { next_codex_turn(&id) }).detach();
    }
}

/// Stop button: Claude interrupts its turn (the process stays); Codex's turn process is killed.
pub fn interrupt(pane_id: &str) {
    let mut chats = chats();
    let Some(chat) = chats.map.get_mut(pane_id) else { return };
    chat.pending.clear();
    let Some(mut proc) = chat.proc.take() else { return };
    if proc.stdin.is_some() {
        let request_id = format!("agentdeck-{}", proc.run);
        write_line(&mut proc, &claude_interrupt_line(&request_id));
        chat.proc = Some(proc);
        return;
    }
    let _ = proc.child.kill();
    chat.transcript.apply(ChatEvent::TurnEnd { error: Some("Interrupted".into()) });
    let (backend, session) = (chat.launch.as_ref().map(|l| l.backend.clone()), chat.transcript.session.clone());
    drop(chats);
    if let Some(backend) = backend {
        report(&backend, pane_id, PaneStatus::Done, Some("Interrupted".into()), None, None, session);
    }
    changed(pane_id);
}

/// Kills the pane's process (stop, restart, close, sleep); a no-op for other panes. The transcript
/// stays for a wake. Emits `pane-exit`, as the PTY does, so `stop_pane` returns at once.
pub fn kill(pane_id: &str) {
    let mut chats = chats();
    let Some(chat) = chats.map.get_mut(pane_id) else { return };
    let proc = chat.proc.take();
    chat.pending.clear();
    chat.transcript.busy = false;
    let backend = chat.launch.as_ref().map(|l| l.backend.clone());
    drop(chats);
    if let Some(mut proc) = proc {
        let _ = proc.child.kill();
    }
    if let Some(backend) = backend {
        let _ = backend.emit(&format!("pane-exit:{pane_id}"), ());
    }
    changed(pane_id);
}

#[cfg(test)]
mod tests {
    use super::has_flag;

    #[test]
    fn flags_match_with_or_without_value() {
        let args: Vec<String> = ["--model", "x", "--permission-mode=plan"].map(String::from).into();
        assert!(has_flag(&args, &["--permission-mode"]));
        assert!(!has_flag(&args, &["-s", "--sandbox"]));
    }
}
