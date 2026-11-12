//! Pane lifecycle (store/runtime.ts): spawn / adopt / restart / stop / close, output activity tracking,
//! input and size forwarding, status derived from output for agents without hooks, conflict and
//! rate-limit watchers, session restore at startup and the debounced autosave of store.json.
//!
//! Every function takes `&mut App` and finds the deck registered by [`start`].

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Duration;

use agentdeck_core::hooks_server::{AgentStatusEvent, PaneStatus, Step, Subagent};
use agentdeck_core::logic::deck::{Pane, PaneSpec, new_pane};
use agentdeck_core::logic::queue::{SUBMIT_DELAY_MS, prompt_text};
use agentdeck_core::logic::ring_buffer::RingBuffer;
use agentdeck_core::agents::AgentKind;
use agentdeck_core::logic::text::{TERMINAL_NOTICE, agent_name, interrupt_key, now_ms, osc_notification};
use agentdeck_core::{AppHandle, AppState, Event, mcp, persist, pty};
use gpui_kit::*;
use smol::lock::Semaphore;

use crate::state::{Deck, DeckEvent};

/// Output within this window counts as "working" for panes without hooks.
pub const ACTIVITY_WINDOW_MS: i64 = 3000;
/// Output this soon after the process starts is its banner, not work.
const STARTUP_MS: i64 = 5000;
/// A starting agent is ready once its banner has been quiet this long.
const BOOT_QUIET_MS: i64 = 500;
const BOOT_POLL: Duration = Duration::from_millis(100);
/// Upper bound on waiting for a starting agent (spawn slot included); the prompt goes anyway after.
const BOOT_POLLS: u32 = 300;
/// A TUI repaints right after a click, a resize or a keystroke: that output must not light the pane up as busy.
const REPAINT_MS: i64 = 600;
/// Claude exits within this window when the conversation to resume no longer exists.
const RESUME_GRACE_MS: i64 = 8000;
const STOP_TIMEOUT: Duration = Duration::from_secs(3);
const ALREADY_RUNNING: &str = "pane id already in use";
const CONFLICT_WINDOW_MS: i64 = 15 * 60_000;
/// Sink owner label for `pty::attach_pane_output`.
const OWNER: &str = "panes-runtime";
/// At most this many agents start at once: a restore with many panes would otherwise spawn them all together.
static SPAWN_SLOTS: Semaphore = Semaphore::new(3);

#[derive(Default)]
struct Output {
    buffer: RingBuffer,
    last_output_at: i64,
    /// Output before this time is a repaint (focus, resize, typing echo), not activity.
    repaint_until: i64,
    /// Desktop notification the agent printed (OSC 9 / 777), not yet forwarded.
    notice: Option<String>,
    /// Set while the pane waits because of such a notification (cleared once output resumes).
    notice_waiting_since: Option<i64>,
}

struct Runtime {
    /// Distinguishes this runtime from a later one under the same id (closed and recreated).
    token: u64,
    output: Arc<Mutex<Output>>,
    sink: u64,
    cols: u16,
    rows: u16,
    /// The terminal has reported its size at least once.
    sized: bool,
    /// Adopted: the next size report forces the TUI to repaint.
    redraw: bool,
    /// When the current process was spawned (ms); pane usage counts from here.
    started_at: Option<i64>,
    /// Spawned with --resume: an exit right after means the conversation is gone, so retry fresh.
    resumed_at: Option<i64>,
    /// PORT given to this agent (settings.basePort + n), kept across restarts.
    port: Option<u16>,
    /// Last hook event: a hooked agent "working" with no event for long is likely stuck.
    last_hook_at: Option<i64>,
    /// Set while `stop_pane` waits for the exit event.
    on_exit: Option<smol::channel::Sender<()>>,
    /// Killed on purpose: its exit event must not close the pane.
    stopped: bool,
    /// Put to sleep to free memory: the next start picks its conversation up again.
    asleep: bool,
    restarting: bool,
}

struct Editor {
    pane_id: String,
    at: i64,
    warned_at: Option<i64>,
    in_worktree: bool,
}

#[derive(Default)]
struct Runtimes {
    deck: Option<Entity<Deck>>,
    map: HashMap<String, Runtime>,
    next_token: u64,
    /// Last pane that edited each file (normalized path), to spot two agents editing the same file.
    editors: HashMap<String, Editor>,
    rate_limit_retries: HashSet<String>,
    /// Panes whose plan is unfolded under the header.
    steps_open: HashSet<String>,
    /// Claude subagents running inside each pane (`agent-subagents`); panes without any have no entry.
    subagents: HashMap<String, Vec<Subagent>>,
    /// Plan of each Claude pane (`agent-steps`); panes without any have no entry.
    steps: HashMap<String, Vec<Step>>,
    /// Pane the backend routes voice input to.
    backend_active: Option<String>,
    save_pending: bool,
    /// The pending deck notify is the 1 s clock repaint: nothing to save.
    clock_repaint: bool,
    last_saved: String,
    /// Queues as last written to local.json.
    saved_queues: agentdeck_core::logic::queue::Queues,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Global for Runtimes {}

fn rts(cx: &mut App) -> &mut Runtimes {
    cx.default_global::<Runtimes>()
}

fn rt_mut<'a>(id: &str, cx: &'a mut App) -> Option<&'a mut Runtime> {
    rts(cx).map.get_mut(id)
}

pub(super) fn deck(cx: &App) -> Option<Entity<Deck>> {
    cx.try_global::<Runtimes>()?.deck.clone()
}

fn pane(id: &str, cx: &App) -> Option<Pane> {
    deck(cx)?.read(cx).state.panes.get(id).cloned()
}

fn lock(output: &Mutex<Output>) -> MutexGuard<'_, Output> {
    output.lock().unwrap_or_else(PoisonError::into_inner)
}

fn backend(cx: &App) -> Option<AppHandle> {
    Some(deck(cx)?.read(cx).backend.clone())
}

fn app_state(cx: &App) -> Option<Arc<AppState>> {
    backend(cx)?.try_state::<AppState>()
}

fn alive(id: &str, token: u64, cx: &App) -> bool {
    cx.try_global::<Runtimes>()
        .and_then(|r| r.map.get(id))
        .is_some_and(|rt| rt.token == token)
}

fn update_pane(id: &str, cx: &mut App, f: impl FnOnce(&mut Pane)) {
    if let Some(deck) = deck(cx) {
        deck.update(cx, |d, cx| {
            if d.state.update_pane(id, f) {
                cx.notify();
            }
        });
    }
}

fn toast(message: String, cx: &mut App) {
    if let Some(deck) = deck(cx) {
        deck.update(cx, |d, cx| d.toast(message, cx));
    }
}

/// Registers the deck, restores the saved sessions, starts the 1 s clock, the autosave and the
/// handlers of pane exits and hook events. Call once, after the deck is created.
pub fn start(deck: &Entity<Deck>, cx: &mut App) {
    let events = cx.subscribe(deck, |_, event: &DeckEvent, cx| {
        let DeckEvent::Backend(event) = event;
        on_backend(event, cx);
    });
    let autosave = cx.observe(deck, |_, cx| schedule_save(cx));
    let clock = cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            cx.update(tick);
            // Its own update: the notify of a status `tick` changed must reach the autosave.
            cx.update(clock_repaint);
        }
    });
    let r = rts(cx);
    r.deck = Some(deck.clone());
    r._subscriptions = vec![events, autosave];
    r._tasks.push(clock);
    restore(cx);
}

/// Respawns the saved sessions (active session first: its agents get the first spawn slots).
/// Live panes (none after a fresh start) are adopted, live panes no session restores are killed.
fn restore(cx: &mut App) {
    let Some(deck) = deck(cx) else { return };
    let (sessions, active) = deck.update(cx, |d, _| {
        (std::mem::take(&mut d.pending_restore), d.state.active_tab_id.clone())
    });
    let alive = app_state(cx)
        .and_then(|s| pty::list_panes(&s).ok())
        .unwrap_or_default();
    let (adopt, orphans) = persist::plan_restore(&sessions, &alive);
    for id in orphans {
        kill_in_background(&id, cx);
    }
    let mut ordered = sessions;
    ordered.sort_by_key(|s| s.id != active);
    for session in ordered {
        for spec in session.panes {
            let adopted = adopt.contains(&spec.id);
            create_pane(spec, &session.id, adopted, cx);
        }
    }
}

/// Debounced (persist::SAVE_DEBOUNCE_MS) save of store.json when workspaces, tabs, pane specs,
/// archive, last folder or active tab changed. Runtime fields (status, last action) do not save.
fn schedule_save(cx: &mut App) {
    let r = rts(cx);
    // Taken here: observers run after `clock_repaint` returned, so it cannot clear the flag itself.
    if std::mem::take(&mut r.clock_repaint) || r.save_pending {
        return;
    }
    r.save_pending = true;
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(persist::SAVE_DEBOUNCE_MS))
            .await;
        cx.update(|cx| {
            rts(cx).save_pending = false;
            let Some(deck) = deck(cx) else { return };
            // Waiting prompts survive a restart; nothing is sent by itself when the app comes back.
            let queues = deck.read(cx).queues.clone();
            if queues != rts(cx).saved_queues {
                if let Err(e) = persist::save_queues(&deck.read(cx).data_dir, &queues) {
                    eprintln!("queues not saved: {e}");
                }
                rts(cx).saved_queues = queues;
            }
            let snapshot = serde_json::to_string(&persist::snapshot(&deck.read(cx).state, 0)).unwrap_or_default();
            if snapshot == rts(cx).last_saved {
                return;
            }
            rts(cx).last_saved = snapshot;
            deck.update(cx, |d, cx| d.save_now(cx));
        });
    })
    .detach();
}

/// Every second: status from output for panes without hooks.
fn tick(cx: &mut App) {
    let now = now_ms();
    auto_sleep(now, cx);
    derive_statuses(now, cx);
    forward_terminal_notices(now, cx);
}

/// Every second while an agent is live: a repaint for timers and outlines.
fn clock_repaint(cx: &mut App) {
    let now = now_ms();
    let Some(deck) = deck(cx) else { return };
    let live = deck.read(cx).state.panes.values().any(|p| {
        p.task_started_at.is_some() || matches!(p.status, PaneStatus::Working | PaneStatus::Waiting)
    });
    if live || super::frame::voice_flash_pending(now, cx) {
        // Statuses and timers are not saved: the autosave skips this notify.
        rts(cx).clock_repaint = true;
        deck.update(cx, |_, cx| cx.notify());
    }
}

fn on_backend(event: &Event, cx: &mut App) {
    match event.name.as_str() {
        "agent-status" => {
            if let Ok(e) = serde_json::from_value::<AgentStatusEvent>(event.payload.clone()) {
                on_status(e, cx);
            }
        }
        // The agent is about to use its browser tools: turns the panel on, as the globe button does.
        // The hook is held meanwhile (`hooks_server::open_browser`).
        "agent-browser" => {
            if let Some(id) = event.payload["paneId"].as_str() {
                update_pane(id, cx, |p| p.spec.browser = true);
                show_browser(id, cx);
            }
        }
        // The deck notifies after every backend event: headers and the activity panel repaint.
        "agent-subagents" => {
            let list = serde_json::from_value::<Vec<Subagent>>(event.payload["subagents"].clone());
            if let (Some(id), Ok(list)) = (event.payload["paneId"].as_str(), list) {
                if list.is_empty() || !rts(cx).map.contains_key(id) {
                    rts(cx).subagents.remove(id);
                } else {
                    rts(cx).subagents.insert(id.to_owned(), list);
                }
            }
        }
        "agent-steps" => {
            let list = serde_json::from_value::<Vec<Step>>(event.payload["steps"].clone());
            if let (Some(id), Ok(list)) = (event.payload["paneId"].as_str(), list) {
                if list.is_empty() || !rts(cx).map.contains_key(id) {
                    rts(cx).steps.remove(id);
                } else {
                    rts(cx).steps.insert(id.to_owned(), list);
                }
            }
        }
        "voice-state" | "voice-level" => super::frame::on_voice_event(event, cx),
        name => {
            if let Some(id) = name.strip_prefix("pane-exit:") {
                on_exit(id, cx);
            }
        }
    }
}

/// Creates the pane in the deck, subscribes to its output, then spawns the process.
/// `adopt`: the process is still alive in the backend, only reattach to it.
pub fn create_pane(spec: PaneSpec, tab_id: &str, adopt: bool, cx: &mut App) {
    let Some(deck) = deck(cx) else { return };
    let now = now_ms();
    let id = spec.id.clone();
    if rts(cx).map.contains_key(&id) {
        // Already running: only make sure it is listed in its tab.
        deck.update(cx, |d, cx| {
            let pane = d.state.panes.get(&id).cloned().unwrap_or_else(|| new_pane(spec, now));
            d.state.add_pane(pane, tab_id);
            cx.notify();
        });
        return;
    }
    let port = spec.port;
    deck.update(cx, |d, cx| {
        d.state.add_pane(new_pane(spec, now), tab_id);
        cx.notify();
    });
    let output = Arc::new(Mutex::new(Output::default()));
    let (tx, rx) = mpsc::channel::<String>();
    let sink = pty::attach_pane_output(OWNER, id.clone(), tx);
    let reader = output.clone();
    // Ends when the sink is detached (sender dropped) at close.
    let spawned = std::thread::Builder::new()
        .name(format!("pane-activity-{id}"))
        .spawn(move || {
            while let Ok(data) = rx.recv() {
                let now = now_ms();
                let mut out = lock(&reader);
                if let Some(text) = osc_notification(&data) {
                    out.notice = Some(text);
                }
                out.buffer.push(&data);
                if now > out.repaint_until {
                    out.last_output_at = now;
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("pane activity thread: {e}");
    }
    let r = rts(cx);
    r.next_token += 1;
    let token = r.next_token;
    r.map.insert(
        id.clone(),
        Runtime {
            token,
            output,
            sink,
            cols: 80,
            rows: 24,
            sized: false,
            redraw: false,
            started_at: None,
            resumed_at: None,
            port,
            last_hook_at: None,
            on_exit: None,
            stopped: false,
            asleep: false,
            restarting: false,
        },
    );
    if adopt {
        adopt_pane(&id, cx);
    } else {
        spawn(&id, cx);
    }
}

/// Reattaches to a live process: no spawn; the screen is repainted on the first size report.
fn adopt_pane(id: &str, cx: &mut App) {
    let Some(rt) = rt_mut(id, cx) else { return };
    rt.started_at = Some(now_ms());
    let nudge = rt.sized.then_some((rt.cols, rt.rows));
    rt.redraw = !rt.sized;
    if let Some((cols, rows)) = nudge {
        nudge_size(id, cols, rows, cx);
    }
    // No spawn opened it: the webview takes the port the live agent was started with.
    if pane(id, cx).is_some_and(|p| p.browser) {
        show_browser(id, cx);
    }
    let now = now_ms();
    update_pane(id, cx, |p| {
        p.exited = false;
        p.status_since = now;
    });
}

fn resize(id: &str, cols: u16, rows: u16, cx: &App) -> Result<(), String> {
    let state = app_state(cx).ok_or("backend not started")?;
    pty::resize_pane(&state, id.to_owned(), cols, rows)
}

/// TUIs repaint on resize: nudge the width so even an unchanged size redraws the adopted screen.
fn nudge_size(id: &str, cols: u16, rows: u16, cx: &App) {
    // Fails harmlessly before the spawn completes.
    let _ = resize(id, cols.saturating_sub(1).max(1), rows, cx).and_then(|()| resize(id, cols, rows, cx));
}

/// Conversation ids are UUIDs; anything else (a value starting with "-" would read as a flag) is ignored.
fn valid_resume_id(id: &str) -> bool {
    let mut chars = id.chars();
    id.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Smallest free PORT from `base` among running agents.
fn free_port(base: u16, map: &HashMap<String, Runtime>) -> u16 {
    let used: HashSet<u16> = map.values().filter_map(|r| r.port).collect();
    (base..=u16::MAX).find(|p| !used.contains(p)).unwrap_or(base)
}

/// Opens the pane's browser webview in the main window. Not from inside a window update.
fn open_browser(id: &str, cx: &mut App) -> Result<(), String> {
    let dir = deck(cx).ok_or("deck not ready")?.read(cx).data_dir.clone();
    let window = crate::system::main_window(cx).ok_or("main window not ready")?;
    window
        .update(cx, |_, window, cx| crate::browser::open(id, &dir, window, cx))
        .map_err(|e| e.to_string())?
}

/// The Claude args that connect the pane to its browser, given at every spawn so that the panel
/// opens and closes without a restart; the webview itself is only opened for a pane with its panel on.
fn browser_args(id: &str, cx: &mut App) -> Result<Vec<String>, String> {
    let dir = deck(cx).ok_or("deck not ready")?.read(cx).data_dir.clone();
    let args = crate::browser::args(id, &dir, cx)?;
    if pane(id, cx).is_some_and(|p| p.browser) {
        open_browser(id, cx)?;
    }
    Ok(args)
}

/// The browser panel was turned on: opens its webview. The running agent already has the browser
/// tools (see `browser_args`), so it is not restarted.
pub fn show_browser(id: &str, cx: &mut App) {
    let id = id.to_owned();
    // Deferred: the caller may be inside a window update.
    cx.spawn(async move |cx| {
        cx.update(|cx| {
            if let Err(e) = open_browser(&id, cx) {
                toast(format!("Browser: {e}"), cx);
            }
            // Redraws the panel, which waits for the webview.
            update_pane(&id, cx, |_| {});
        });
    })
    .detach();
}

/// `mcp::Registration` fields are private; read them through its serde form.
fn registration(backend: &AppHandle, state: &AppState, id: &str) -> Result<(Vec<String>, String), String> {
    let reg = mcp::mcp_register(backend, state, id.to_owned())?;
    let value = serde_json::to_value(&reg).map_err(|e| e.to_string())?;
    let args = serde_json::from_value(value["args"].clone()).map_err(|e| e.to_string())?;
    let secret = value["secret"].as_str().ok_or("orchestrator secret missing")?.to_owned();
    Ok((args, secret))
}

struct SpawnJob {
    backend: AppHandle,
    pane: Pane,
    port: Option<u16>,
    resume: Option<String>,
    cols: u16,
    rows: u16,
    trust: bool,
    /// Browser panel MCP args (Claude), filled on the main thread before the spawn.
    browser: Vec<String>,
    /// `settings.worktree_setup`: run once in a worktree created by this spawn.
    setup: String,
}

const PROMPT_FLAG: &str = "--append-system-prompt-file";

/// Claude takes one appended prompt file: when several features bring theirs (the browser, the
/// orchestrator role), they are joined into one file so none is dropped.
fn merge_prompt_files(args: &mut Vec<String>, dir: &std::path::Path, pane_id: &str) -> Result<(), String> {
    let files: Vec<String> = args.windows(2).filter(|pair| pair[0] == PROMPT_FLAG).map(|pair| pair[1].clone()).collect();
    if files.len() < 2 {
        return Ok(());
    }
    let text: Vec<String> = files.iter().filter_map(|file| std::fs::read_to_string(file).ok()).collect();
    let merged = dir.join(format!("{pane_id}.md"));
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(&merged, text.join("\n\n")).map_err(|e| format!("{}: {e}", merged.display()))?;
    let mut kept = Vec::with_capacity(args.len());
    let mut rest = std::mem::take(args).into_iter();
    while let Some(arg) = rest.next() {
        if arg == PROMPT_FLAG {
            rest.next(); // its file
        } else {
            kept.push(arg);
        }
    }
    kept.extend([PROMPT_FLAG.to_owned(), merged.to_string_lossy().into_owned()]);
    *args = kept;
    Ok(())
}

/// Blocking: MCP registration (orchestrator) and the PTY spawn.
fn run_spawn(job: SpawnJob) -> Result<(), String> {
    let state = job.backend.try_state::<AppState>().ok_or("backend not started")?;
    let pane = job.pane;
    let mut env = HashMap::new();
    let mut args = job.browser;
    // Orchestrator wiring is regenerated on each spawn; the secret only goes to this process's env.
    if pane.orchestrator {
        let (mcp_args, secret) = registration(&job.backend, &state, &pane.id)?;
        args.extend(mcp_args);
        env.insert("AGENTDECK_MCP_SECRET".to_owned(), secret);
    }
    if let Some(port) = job.port {
        env.insert("PORT".to_owned(), port.to_string());
    }
    if let Some(resume) = job.resume {
        args.push("--resume".to_owned());
        args.push(resume);
    }
    args.extend(pane.spec.args.iter().cloned());
    if let Ok(dir) = job.backend.app_data_dir() {
        merge_prompt_files(&mut args, &dir.join("prompts"), &pane.id)?;
    }
    // Over ssh the agent sees nothing of this machine: only its own arguments go.
    if let Some(target) = &pane.ssh {
        args = pane.spec.args.clone();
        env = HashMap::from([(pty::SSH_ENV.to_owned(), target.clone())]);
    }
    // Creates the worktree and sets it up; spawn_pane then reuses it.
    if pane.worktree && pane.ssh.is_none() && !job.setup.is_empty() {
        agentdeck_core::agents::create_worktree_with(&pane.cwd, &pane.id, &job.setup)?;
    }
    pty::spawn_pane(
        &job.backend,
        pane.id.clone(),
        pane.agent,
        pane.cwd.clone(),
        args,
        pane.worktree,
        job.cols,
        job.rows,
        Some(pane.title.clone()),
        (!env.is_empty()).then_some(env),
        Some(job.trust),
    )
}

fn spawn(id: &str, cx: &mut App) {
    // Waking resumes the conversation even when "resume sessions" is off: that is what sleep keeps.
    let waking = is_asleep(id, cx);
    let Some(deck) = deck(cx) else { return };
    let d = deck.read(cx);
    let Some(pane) = d.state.panes.get(id).cloned() else { return };
    let backend = d.backend.clone();
    let base_port = d.settings.base_port;
    let trust = d.settings.trust_agent_folders;
    let setup = d.settings.worktree_setup.clone();
    let resume = (pane.agent == agentdeck_core::agents::AgentKind::Claude && (d.settings.resume_sessions || waking))
        .then(|| pane.resume_id.clone())
        .flatten()
        .filter(|r| valid_resume_id(r));
    let r = rts(cx);
    let Some(own) = r.map.get(id).map(|rt| rt.port) else { return };
    let taken = own.is_some() && r.map.iter().any(|(k, rt)| k != id && rt.port == own);
    let port = if base_port != 0 && (own.is_none() || taken) {
        Some(free_port(base_port, &r.map))
    } else {
        own
    };
    let Some(rt) = r.map.get_mut(id) else { return };
    rt.started_at = Some(now_ms());
    rt.resumed_at = None;
    rt.stopped = false;
    rt.asleep = false;
    rt.port = port;
    let (token, cols, rows) = (rt.token, rt.cols, rt.rows);
    if port != pane.port {
        update_pane(id, cx, |p| p.spec.port = port);
    }
    let resumed = resume.is_some();
    let mut job = SpawnJob {
        backend,
        pane,
        port: port.filter(|_| base_port != 0),
        resume,
        cols,
        rows,
        trust,
        browser: Vec::new(),
        setup,
    };
    let id = id.to_owned();
    cx.spawn(async move |cx| {
        let _slot = SPAWN_SLOTS.acquire().await;
        // Closed while queued for a slot.
        if !cx.update(|cx| alive(&id, token, cx)) {
            return;
        }
        // Outside any window update: the webview is created in the main window.
        if job.pane.agent == agentdeck_core::agents::AgentKind::Claude {
            match cx.update(|cx| browser_args(&id, cx)) {
                Ok(args) => job.browser = args,
                Err(e) => {
                    cx.update(|cx| after_spawn(&id, token, Err(e), resumed, (cols, rows), cx));
                    return;
                }
            }
        }
        if job.pane.worktree && !job.setup.is_empty() {
            let (cwd, pane_id) = (job.pane.cwd.clone(), job.pane.id.clone());
            // The pane stays blank during a long install: say why. No worktree yet = the cwd comes back.
            let is_new = cx
                .background_spawn(async move { agentdeck_core::git::git_pane_dir(cwd.clone(), pane_id, true) == cwd })
                .await;
            if is_new {
                let title = job.pane.title.clone();
                cx.update(|cx| toast(format!("Setting up the worktree of {title}"), cx));
            }
        }
        let result = cx.background_spawn(async move { run_spawn(job) }).await;
        cx.update(|cx| after_spawn(&id, token, result, resumed, (cols, rows), cx));
    })
    .detach();
}

fn after_spawn(id: &str, token: u64, result: Result<(), String>, resumed: bool, size: (u16, u16), cx: &mut App) {
    match result {
        // Race with a previous run: the process is already running under this id.
        Err(e) if e == ALREADY_RUNNING => adopt_pane(id, cx),
        Err(e) => fail_pane(id, &e, cx),
        Ok(()) => {
            // Closed while spawning: the close's kill ran before the backend registered the pane.
            let Some(rt) = rt_mut(id, cx).filter(|rt| rt.token == token) else {
                kill_in_background(id, cx);
                return;
            };
            // Measured from the real start: time queued for a slot must not eat the grace window.
            if resumed {
                rt.resumed_at = Some(now_ms());
            }
            // The terminal may have been fitted while spawning.
            let current = (rt.cols, rt.rows);
            if current != size
                && let Err(e) = resize(id, current.0, current.1, cx)
            {
                eprintln!("resize_pane: {e}");
            }
        }
    }
}

fn fail_pane(id: &str, err: &str, cx: &mut App) {
    let Some(p) = pane(id, cx) else { return };
    let now = now_ms();
    let message = err.to_owned();
    update_pane(id, cx, |p| {
        p.status = PaneStatus::Error;
        p.exited = true;
        p.last_action = Some(message);
        p.status_since = now;
    });
    toast(format!("Failed to start {}: {err}", agent_name(p.agent)), cx);
}

fn kill_in_background(id: &str, cx: &mut App) {
    let Some(state) = app_state(cx) else { return };
    let id = id.to_owned();
    cx.background_spawn(async move {
        if let Err(e) = pty::kill_pane(&state, id) {
            eprintln!("kill_pane failed: {e}");
        }
    })
    .detach();
}

/// Kills the process and waits for its exit event (3 s max), leaving the pane in the exited state.
pub fn stop_pane(id: &str, cx: &mut App) -> Task<()> {
    if pane(id, cx).is_none_or(|p| p.exited) {
        return Task::ready(());
    }
    let Some(rt) = rt_mut(id, cx) else {
        return Task::ready(());
    };
    let (tx, rx) = smol::channel::bounded::<()>(1);
    rt.on_exit = Some(tx);
    rt.stopped = true;
    kill_in_background(id, cx);
    let id = id.to_owned();
    cx.spawn(async move |cx| {
        let timeout = cx.background_executor().timer(STOP_TIMEOUT);
        smol::future::or(
            async {
                let _ = rx.recv().await;
            },
            timeout,
        )
        .await;
        cx.update(|cx| {
            if let Some(rt) = rt_mut(&id, cx) {
                rt.on_exit = None;
            }
            update_pane(&id, cx, |p| p.exited = true);
        });
    })
}

/// Stops the process and starts a new conversation in the same pane. A second call while one runs is ignored.
pub fn restart_pane(id: &str, cx: &mut App) {
    let Some(rt) = rt_mut(id, cx) else { return };
    if rt.restarting {
        return;
    }
    rt.restarting = true;
    let token = rt.token;
    let stop = stop_pane(id, cx);
    let id = id.to_owned();
    cx.spawn(async move |cx| {
        stop.await;
        cx.update(|cx| {
            let Some(rt) = rt_mut(&id, cx).filter(|rt| rt.token == token) else {
                return; // closed meanwhile
            };
            rt.restarting = false;
            lock(&rt.output).buffer.clear();
            rts(cx).subagents.remove(&id);
            rts(cx).steps.remove(&id);
            agentdeck_core::mcp::STATUS.set(&id, PaneStatus::Idle); // forget the old run's "done"
            let now = now_ms();
            update_pane(&id, cx, |p| {
                p.topic = None;
                p.status = PaneStatus::Idle;
                p.last_action = None;
                p.task_started_at = None;
                p.status_since = now;
                p.exited = false;
                p.hooked = false;
                p.spec.resume_id = None; // a restart starts a new conversation
                p.generation += 1;
            });
            spawn(&id, cx);
        });
    })
    .detach();
}

/// Removes the pane and kills its process (and its whole process tree).
pub fn close_pane(id: &str, cx: &mut App) {
    let r = rts(cx);
    let rt = r.map.remove(id);
    r.editors.retain(|_, e| e.pane_id != id);
    r.rate_limit_retries.remove(id);
    r.subagents.remove(id);
    r.steps.remove(id);
    let was_active = r.backend_active.as_deref() == Some(id);
    if was_active {
        r.backend_active = None;
    }
    if let Some(rt) = rt {
        pty::detach_pane_output(id.to_owned(), rt.sink);
    }
    crate::browser::close(id, cx);
    if was_active
        && let Some(state) = app_state(cx)
        && let Err(e) = pty::set_active_pane(&state, None)
    {
        eprintln!("set_active_pane failed: {e}");
    }
    if let Some(deck) = deck(cx) {
        deck.update(cx, |d, cx| {
            d.state.remove_pane(id);
            d.tasks.remove(id);
            cx.notify();
        });
    }
    kill_in_background(id, cx);
}

/// The process ended: wakes `stop_pane`, retries a vanished `--resume` fresh, or closes the pane
/// when the agent quit by itself (Ctrl+C, /exit).
fn on_exit(id: &str, cx: &mut App) {
    // Its subagents died with the process (no SubagentStop comes).
    rts(cx).subagents.remove(id);
    let Some(rt) = rt_mut(id, cx) else { return };
    if let Some(tx) = rt.on_exit.take() {
        let _ = tx.try_send(());
        return;
    }
    if rt.stopped {
        return;
    }
    let resumed_recently = rt.resumed_at.is_some_and(|at| now_ms() - at < RESUME_GRACE_MS);
    if resumed_recently {
        rt.resumed_at = None;
        lock(&rt.output).buffer.clear();
        update_pane(id, cx, |p| {
            p.spec.resume_id = None;
            p.generation += 1;
            p.exited = false;
        });
        spawn(id, cx);
        return;
    }
    if let Some(p) = pane(id, cx) {
        toast(format!("{} exited", p.title), cx);
    }
    close_pane(id, cx);
}

fn on_status(e: AgentStatusEvent, cx: &mut App) {
    let Some(deck) = deck(cx) else { return };
    if !deck.read(cx).state.panes.contains_key(&e.pane_id) {
        return;
    }
    let now = now_ms();
    if let Some(rt) = rt_mut(&e.pane_id, cx) {
        rt.last_hook_at = Some(now);
    }
    deck.update(cx, |d, cx| {
        if let Some(session) = &e.session_id {
            d.state.update_pane(&e.pane_id, |p| p.spec.resume_id = Some(session.clone()));
        }
        cx.notify();
    });
    // Saved in local.json too (Broadcast's recent prompts).
    if e.event == "UserPromptSubmit"
        && let Some(prompt) = &e.prompt
    {
        crate::palette::remember_prompt(&deck, prompt, cx);
    }
    if let Some(file) = &e.edited_file {
        check_conflict(&e.pane_id, file, now, cx);
    }
    if e.status == PaneStatus::Error && e.event == "StopFailure" && is_rate_limit(e.last_action.as_deref()) {
        schedule_rate_limit_retry(&e.pane_id, cx);
    }
}

/// /rate.?limit/i
fn is_rate_limit(text: Option<&str>) -> bool {
    let text = text.unwrap_or("").to_lowercase();
    text.match_indices("rate").any(|(i, _)| {
        let rest = &text[i + 4..];
        rest.starts_with("limit")
            || rest
                .chars()
                .next()
                .is_some_and(|c| rest[c.len_utf8()..].starts_with("limit"))
    })
}

/// Toasts once when another live agent edited the same file recently (no worktree between them).
fn check_conflict(pane_id: &str, file: &str, now: i64, cx: &mut App) {
    let key = agentdeck_core::git::shared_file_key(file);
    // In separate worktrees the clash waits for the merge, however long ago the other edit was.
    let in_worktree = key != file.replace('\\', "/").to_lowercase();
    let Some(deck) = deck(cx) else { return };
    let prev = rts(cx).editors.remove(&key);
    let mut warned_at = prev.as_ref().and_then(|e| e.warned_at);
    if let Some(prev) = &prev {
        let panes = &deck.read(cx).state.panes;
        let fresh_warning = warned_at.is_none_or(|w| now - w > CONFLICT_WINDOW_MS);
        if prev.pane_id != pane_id
            && (in_worktree || prev.in_worktree || now - prev.at < CONFLICT_WINDOW_MS)
            && fresh_warning
            && let Some(other) = panes.get(&prev.pane_id)
        {
            let name = key.rsplit('/').next().unwrap_or(&key).to_owned();
            let this = panes.get(pane_id).map_or("another agent".to_owned(), |p| p.title.clone());
            let message = if in_worktree || prev.in_worktree {
                format!("{} and {this} both edited {name}: expect a conflict when merging", other.title)
            } else {
                format!("{} and {this} both edited {name}", other.title)
            };
            toast(message, cx);
            warned_at = Some(now);
        }
    }
    rts(cx).editors.insert(
        key,
        Editor {
            pane_id: pane_id.to_owned(),
            at: now,
            warned_at,
            in_worktree,
        },
    );
}

/// Sends "continue" once the wait is over, if the agent is still stopped on that error.
fn schedule_rate_limit_retry(id: &str, cx: &mut App) {
    let Some(deck) = deck(cx) else { return };
    let minutes = deck.read(cx).settings.retry_rate_limit_minutes;
    let Some(since) = pane(id, cx).map(|p| p.status_since) else { return };
    if minutes == 0 || !rts(cx).rate_limit_retries.insert(id.to_owned()) {
        return;
    }
    let id = id.to_owned();
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_secs(u64::from(minutes) * 60))
            .await;
        cx.update(|cx| {
            if !rts(cx).rate_limit_retries.remove(&id) {
                return; // closed meanwhile
            }
            let Some(p) = pane(&id, cx) else { return };
            if p.exited || p.status != PaneStatus::Error || p.status_since != since {
                return;
            }
            toast(format!("{}: continuing after the rate limit", p.title), cx);
            send_prompt(&id, "continue", cx);
        });
    })
    .detach();
}

/// True while a fresh agent is still starting: queued for a spawn slot, or printing its banner.
/// Input sent then sits in the console buffer and is read with the Enter as one paste.
fn booting(started_at: Option<i64>, last_output_at: i64, now: i64) -> bool {
    let Some(start) = started_at else { return true };
    let quiet = last_output_at > start && now - last_output_at >= BOOT_QUIET_MS;
    now - start < STARTUP_MS && !quiet
}

/// Types the prompt once the agent is up, then presses Enter in a separate write (a TUI reads
/// text + Enter in one write as a paste). Works the same whether the pane is shown or not.
pub fn send_prompt(id: &str, text: &str, cx: &mut App) {
    let (id, text) = (id.to_owned(), prompt_text(text));
    cx.spawn(async move |cx| {
        for _ in 0..BOOT_POLLS {
            let wait = cx.update(|cx| {
                let rt = rts(cx).map.get(&id)?;
                let last_output_at = lock(&rt.output).last_output_at;
                Some(booting(rt.started_at, last_output_at, now_ms()))
            });
            match wait {
                None => return, // closed meanwhile
                Some(false) => break,
                Some(true) => cx.background_executor().timer(BOOT_POLL).await,
            }
        }
        cx.update(|cx| send_input(&id, &text, cx));
        cx.background_executor()
            .timer(Duration::from_millis(SUBMIT_DELAY_MS))
            .await;
        cx.update(|cx| send_input(&id, "\r", cx));
    })
    .detach();
}

/// Keystrokes / text to the pane. An exited pane restarts on R.
pub fn send_input(id: &str, data: &str, cx: &mut App) {
    let Some(p) = pane(id, cx) else { return };
    if p.exited {
        if is_asleep(id, cx) {
            wake_pane(id, cx); // any key wakes a sleeping agent
        } else if data == "r" || data == "R" {
            restart_pane(id, cx);
        }
        return;
    }
    // The echo of what the user types is not the agent working.
    if let Some(rt) = rt_mut(id, cx) {
        lock(&rt.output).repaint_until = now_ms() + REPAINT_MS;
    }
    let Some(state) = app_state(cx) else { return };
    if let Err(e) = state.panes.write(id, data.as_bytes()) {
        eprintln!("write_to_pane failed: {e}");
    }
}

/// Interrupts the current task (Esc, or Ctrl+C for a shell); the agent session stays alive.
pub fn interrupt_pane(id: &str, cx: &mut App) {
    if let Some(p) = pane(id, cx).filter(|p| !p.exited) {
        send_input(id, interrupt_key(p.agent), cx);
    }
}

/// Terminal size report: forwards changes to the PTY; an adopted pane gets its repaint nudge.
pub fn report_size(id: &str, cols: u16, rows: u16, cx: &mut App) {
    let Some(rt) = rt_mut(id, cx) else { return };
    lock(&rt.output).repaint_until = now_ms() + REPAINT_MS;
    rt.sized = true;
    if rt.redraw {
        rt.redraw = false;
        rt.cols = cols;
        rt.rows = rows;
        nudge_size(id, cols, rows, cx);
        return;
    }
    if (rt.cols, rt.rows) == (cols, rows) {
        return;
    }
    rt.cols = cols;
    rt.rows = rows;
    // Fails harmlessly before the spawn completes.
    let _ = resize(id, cols, rows, cx);
}

/// Makes the pane the active one (and the voice target); looking at a finished pane acknowledges it.
pub fn focus_pane(id: &str, cx: &mut App) {
    if let Some(rt) = rt_mut(id, cx) {
        lock(&rt.output).repaint_until = now_ms() + REPAINT_MS;
    }
    let Some(deck) = deck(cx) else { return };
    let changed = deck.update(cx, |d, cx| {
        d.state.acknowledge(id, now_ms());
        let changed = d.state.active_pane_id.as_deref() != Some(id);
        if changed {
            d.state.active_pane_id = Some(id.to_owned());
        }
        cx.notify();
        changed
    });
    if !changed {
        return;
    }
    rts(cx).backend_active = Some(id.to_owned());
    if let Some(state) = app_state(cx)
        && let Err(e) = pty::set_active_pane(&state, Some(id.to_owned()))
    {
        eprintln!("set_active_pane failed: {e}");
    }
}

/// Stops an idle Claude's process to free its memory (the agent and the tool servers it started).
/// The pane stays, asleep, and wakes on the same conversation. Only Claude can pick a conversation
/// up again, and only once it has one.
pub fn sleep_pane(id: &str, cx: &mut App) {
    let Some(p) = pane(id, cx).filter(|p| !p.exited) else { return };
    if p.agent != AgentKind::Claude || p.resume_id.is_none() {
        toast("Only a Claude agent that already has a conversation can sleep.".into(), cx);
        return;
    }
    if let Some(rt) = rt_mut(id, cx) {
        rt.asleep = true;
    }
    stop_pane(id, cx).detach();
    let now = now_ms();
    update_pane(id, cx, |p| {
        p.status = PaneStatus::Idle;
        p.last_action = Some("Asleep".into());
        p.task_started_at = None;
        p.status_since = now;
    });
}

pub fn is_asleep(id: &str, cx: &App) -> bool {
    cx.try_global::<Runtimes>().is_some_and(|r| r.map.get(id).is_some_and(|rt| rt.asleep))
}

/// Starts a sleeping agent again, on its conversation.
pub fn wake_pane(id: &str, cx: &mut App) {
    // Not before its process is gone: the stop is still under way.
    if !is_asleep(id, cx) || pane(id, cx).is_none_or(|p| !p.exited) {
        return;
    }
    if let Some(rt) = rt_mut(id, cx) {
        lock(&rt.output).buffer.clear();
    }
    update_pane(id, cx, |p| {
        p.exited = false;
        p.hooked = false;
        p.last_action = None;
        p.generation += 1;
    });
    spawn(id, cx);
}

/// Puts Claude agents to sleep once they sat idle for `settings.sleep_minutes`. Never the pane
/// being looked at, one with an unread result, queued prompts or running subagents.
fn auto_sleep(now: i64, cx: &mut App) {
    let Some(deck) = deck(cx) else { return };
    let d = deck.read(cx);
    let limit = i64::from(d.settings.sleep_minutes) * 60_000;
    if limit == 0 {
        return;
    }
    let idle: Vec<String> = d
        .state
        .panes
        .values()
        .filter(|p| p.agent == AgentKind::Claude && !p.exited && p.resume_id.is_some())
        .filter(|p| p.status == PaneStatus::Idle && now - p.status_since >= limit)
        .filter(|p| d.state.active_pane_id.as_deref() != Some(p.id.as_str()))
        .filter(|p| d.queues.get(&p.id).is_none_or(|q| q.items.is_empty()))
        .map(|p| p.id.clone())
        .collect();
    for id in idle {
        if pane_subagents(&id, cx).is_empty() && now - pane_last_output_at(&id, cx) >= limit {
            sleep_pane(&id, cx);
        }
    }
}

/// Worth a confirmation before closing: marked working and still printing. An agent interrupted with Esc
/// gets no Stop hook and stays "working" while silent; a busy one keeps animating its spinner.
pub fn is_busy(id: &str, now: i64, cx: &App) -> bool {
    let working = pane(id, cx).is_some_and(|p| p.status == PaneStatus::Working);
    working
        && cx
            .try_global::<Runtimes>()
            .and_then(|r| r.map.get(id))
            .is_some_and(|rt| now - lock(&rt.output).last_output_at < ACTIVITY_WINDOW_MS)
}

/// Codex (tui.notifications), Grok, OpenCode: a desktop notification printed to the terminal means
/// the agent needs the user, so the pane waits and the usual "waiting" notification fires. Claude
/// and Gemini report this through their hooks. Output resuming afterwards means it was answered.
fn forward_terminal_notices(now: i64, cx: &mut App) {
    let Some(deck) = deck(cx) else { return };
    let Some(r) = cx.try_global::<Runtimes>() else { return };
    let d = deck.read(cx);
    let mut events = Vec::new();
    for (id, rt) in &r.map {
        let Some(pane) = d.state.panes.get(id) else { continue };
        let mut out = lock(&rt.output);
        if let Some(text) = out.notice.take() {
            if !matches!(pane.agent, AgentKind::Claude | AgentKind::Gemini) && !pane.exited {
                out.notice_waiting_since = Some(now);
                events.push((id.clone(), PaneStatus::Waiting, Some(text)));
            }
        } else if let Some(since) = out.notice_waiting_since {
            if pane.status != PaneStatus::Waiting {
                out.notice_waiting_since = None; // a hook moved it on
            } else if out.last_output_at > since + 2000 {
                out.notice_waiting_since = None;
                events.push((id.clone(), PaneStatus::Working, None));
            }
        }
    }
    let backend = d.backend.clone();
    for (pane_id, status, last_action) in events {
        let event = AgentStatusEvent {
            pane_id,
            status,
            last_action,
            event: TERMINAL_NOTICE.into(),
            prompt: None,
            summary: None,
            session_id: None,
            edited_file: None,
        };
        if let Err(e) = backend.emit("agent-status", event) {
            eprintln!("terminal notification: {e}");
        }
    }
}

/// A stretch of output this long was a task: its end shows as "done". Shorter ones (a redraw, a
/// menu) just go back to idle, without a notification.
const TASK_MIN_MS: i64 = 5000;

/// Status of a pane read from its output. `busy`: output arrived within the activity window;
/// `last`: time of the latest output; `since`: when the pane got its current status.
fn derived_status(agent: AgentKind, status: PaneStatus, busy: bool, last: i64, since: i64, now: i64) -> PaneStatus {
    match status {
        // The tail of a finished turn is still inside the window: only later output is a new turn.
        PaneStatus::Done if busy && last > since + ACTIVITY_WINDOW_MS => PaneStatus::Working,
        PaneStatus::Idle if busy => PaneStatus::Working,
        PaneStatus::Working if !busy && agent != AgentKind::Shell && now - since >= TASK_MIN_MS => PaneStatus::Done,
        PaneStatus::Working if !busy => PaneStatus::Idle,
        other => other,
    }
}

/// Panes without hooks: working while output arrives, done (or idle) once it stops.
fn derive_statuses(now: i64, cx: &mut App) {
    let Some(deck) = deck(cx) else { return };
    let Some(r) = cx.try_global::<Runtimes>() else { return };
    let changes: Vec<(String, PaneStatus)> = deck
        .read(cx)
        .state
        .panes
        .values()
        // Codex only reports the end of a turn (notify): its "working" comes from its output too.
        .filter(|p| !p.exited && (!p.hooked || p.agent == AgentKind::Codex))
        .filter(|p| matches!(p.status, PaneStatus::Working | PaneStatus::Idle | PaneStatus::Done))
        // Claude and Gemini say when they work through their hooks: their welcome screen, a resumed
        // conversation or a menu is output, not work. Over ssh no hook comes back: output it is.
        .filter(|p| p.ssh.is_some() || !matches!(p.agent, AgentKind::Claude | AgentKind::Gemini))
        .filter_map(|p| {
            let rt = r.map.get(&p.id)?;
            let last = lock(&rt.output).last_output_at;
            // The other agents print a banner when they start: not work either.
            let busy = now - last < ACTIVITY_WINDOW_MS && rt.started_at.is_some_and(|start| last > start + STARTUP_MS);
            let status = derived_status(p.agent, p.status, busy, last, p.status_since, now);
            (status != p.status).then(|| (p.id.clone(), status))
        })
        .collect();
    if changes.is_empty() {
        return;
    }
    deck.update(cx, |d, cx| {
        for (id, status) in changes {
            // No hook will ever report these panes: the orchestrator's wait_for reads this instead.
            let done = if status == PaneStatus::Working { PaneStatus::Working } else { PaneStatus::Done };
            agentdeck_core::mcp::STATUS.set(&id, done);
            d.state.set_status(&id, status, None, now);
        }
        cx.notify();
    });
}

/// Spawn time of the pane's current process (ms), or None before it starts.
pub fn pane_started_at(id: &str, cx: &App) -> Option<i64> {
    cx.try_global::<Runtimes>()?.map.get(id)?.started_at
}

/// Claude subagents running inside the pane, oldest first.
pub fn pane_subagents(id: &str, cx: &App) -> Vec<Subagent> {
    cx.try_global::<Runtimes>()
        .and_then(|r| r.subagents.get(id).cloned())
        .unwrap_or_default()
}

/// The pane's plan is unfolded under its header.
pub fn steps_open(id: &str, cx: &App) -> bool {
    cx.try_global::<Runtimes>().is_some_and(|r| r.steps_open.contains(id))
}

pub fn toggle_steps(id: &str, cx: &mut App) {
    let r = rts(cx);
    if !r.steps_open.remove(id) {
        r.steps_open.insert(id.to_owned());
    }
    cx.refresh_windows();
}

/// Plan of the pane: its task list, else the subagents it launched.
pub fn pane_steps(id: &str, cx: &App) -> Vec<Step> {
    cx.try_global::<Runtimes>()
        .and_then(|r| r.steps.get(id).cloned())
        .unwrap_or_default()
}

/// Terminal title of a pane: Claude Code names the conversation there, other agents show actions.
pub fn set_topic(id: &str, title: &str, cx: &mut App) {
    let Some(topic) = agentdeck_core::logic::text::topic(title) else { return };
    if pane(id, cx).is_some_and(|p| p.agent == AgentKind::Claude && p.topic.as_deref() != Some(topic.as_str())) {
        update_pane(id, cx, |p| p.topic = Some(topic));
    }
}

/// Last hook event time of a pane (for stuck watchers).
pub fn pane_last_hook_at(id: &str, cx: &App) -> Option<i64> {
    cx.try_global::<Runtimes>()?.map.get(id)?.last_hook_at
}

/// Last time the pane printed anything (ms, 0 when unknown): Claude's spinner keeps printing while it works.
pub fn pane_last_output_at(id: &str, cx: &App) -> i64 {
    cx.try_global::<Runtimes>()
        .and_then(|r| r.map.get(id))
        .map_or(0, |rt| lock(&rt.output).last_output_at)
}

/// Buffered raw output of a pane ("" when unknown): replay for a new or detached terminal.
pub fn pane_output_text(id: &str, cx: &App) -> String {
    cx.try_global::<Runtimes>()
        .and_then(|r| r.map.get(id))
        .map(|rt| lock(&rt.output).buffer.text())
        .unwrap_or_default()
}

/// Last `chars` characters of a pane's buffered output.
pub fn pane_output_tail(id: &str, chars: usize, cx: &App) -> String {
    cx.try_global::<Runtimes>()
        .and_then(|r| r.map.get(id))
        .map(|rt| lock(&rt.output).buffer.tail(chars))
        .unwrap_or_default()
}

fn active_pane_ids(cx: &App) -> Vec<String> {
    deck(cx)
        .and_then(|d| d.read(cx).state.active_tab().map(|t| t.pane_ids.clone()))
        .unwrap_or_default()
}

pub fn stop_all(cx: &mut App) {
    for id in active_pane_ids(cx) {
        stop_pane(&id, cx).detach();
    }
}

pub fn restart_all(cx: &mut App) {
    for id in active_pane_ids(cx) {
        restart_pane(&id, cx);
    }
}

pub fn interrupt_all(cx: &mut App) {
    for id in active_pane_ids(cx) {
        interrupt_pane(&id, cx);
    }
}

/// Closes a session tab but keeps it (layout + panes) in the archive.
pub fn archive_session(tab_id: &str, cx: &mut App) {
    let Some(deck) = deck(cx) else { return };
    let ids = deck.update(cx, |d, cx| {
        cx.notify();
        d.state.archive_tab(tab_id, now_ms())
    });
    for id in ids {
        close_pane(&id, cx);
    }
}

/// Forgets a workspace and kills its agents. Files on disk are never touched.
pub fn unlink_workspace(workspace_id: &str, cx: &mut App) {
    let Some(deck) = deck(cx) else { return };
    let ids = deck.update(cx, |d, cx| {
        cx.notify();
        d.state.unlink_workspace(workspace_id)
    });
    for id in ids {
        close_pane(&id, cx);
    }
}

pub fn reopen_session(session_id: &str, cx: &mut App) {
    let Some(deck) = deck(cx) else { return };
    let reopened = deck.update(cx, |d, cx| {
        cx.notify();
        d.state.unarchive(session_id)
    });
    if let Some((tab_id, specs)) = reopened {
        for spec in specs {
            create_pane(spec, &tab_id, false, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_waits_for_a_starting_agent() {
        assert!(booting(None, 0, 10_000)); // queued for a spawn slot
        assert!(booting(Some(1000), 0, 2000)); // no banner yet
        assert!(booting(Some(1000), 1800, 2000)); // banner still printing
        assert!(!booting(Some(1000), 1400, 2000)); // banner done, quiet
        assert!(!booting(Some(1000), 0, 1000 + STARTUP_MS)); // silent agent: give up waiting
    }

    #[test]
    fn several_prompt_files_become_one() {
        let dir = std::env::temp_dir().join(format!("agentdeck-prompts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = |name: &str, text: &str| {
            let path = dir.join(name);
            std::fs::write(&path, text).unwrap();
            path.to_string_lossy().into_owned()
        };
        let (a, b) = (file("a.md", "browser"), file("b.md", "role"));
        let mut args: Vec<String> = vec![PROMPT_FLAG.into(), a, "--model".into(), "x".into(), PROMPT_FLAG.into(), b];
        merge_prompt_files(&mut args, &dir, "pane").unwrap();
        assert_eq!(args[..2], ["--model".to_owned(), "x".to_owned()]);
        assert_eq!(args[2], PROMPT_FLAG);
        assert_eq!(std::fs::read_to_string(&args[3]).unwrap(), "browser\n\nrole");
        // One file: left alone.
        let mut single: Vec<String> = vec![PROMPT_FLAG.into(), args[3].clone()];
        merge_prompt_files(&mut single, &dir, "pane").unwrap();
        assert_eq!(single.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_from_output() {
        use PaneStatus::{Done, Idle, Working};
        let codex = |status, busy, last, since, now| derived_status(AgentKind::Codex, status, busy, last, since, now);
        assert_eq!(codex(Idle, true, 100, 0, 100), Working);
        // A short burst goes back to idle; a real stretch of work ends as done.
        assert_eq!(codex(Working, false, 2_000, 0, 4_000), Idle);
        assert_eq!(codex(Working, false, 20_000, 0, 24_000), Done);
        // The tail of the finished turn does not restart it; later output does.
        assert_eq!(codex(Done, true, 1_000, 0, 1_500), Done);
        assert_eq!(codex(Done, true, 9_000, 0, 9_500), Working);
        assert_eq!(derived_status(AgentKind::Shell, Working, false, 20_000, 0, 24_000), Idle);
    }
    use core::prelude::v1::test;

    #[test]
    fn rate_limit_matches_like_the_regex() {
        assert!(is_rate_limit(Some("Rate limit reached")));
        assert!(is_rate_limit(Some("API error: rate_limit_error")));
        assert!(is_rate_limit(Some("ratelimit")));
        assert!(!is_rate_limit(Some("rate of the limit")));
        assert!(!is_rate_limit(None));
    }

    #[test]
    fn resume_ids_are_uuid_like() {
        assert!(valid_resume_id("3f2a-11ee-be56"));
        assert!(!valid_resume_id("--dangerously-skip"));
        assert!(!valid_resume_id(""));
        assert!(!valid_resume_id(&"a".repeat(65)));
    }
}
