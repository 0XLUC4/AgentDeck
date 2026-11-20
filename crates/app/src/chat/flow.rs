//! What happens around a chat's process: the follow-up queue (sent when the turn ends, paused
//! when one fails) and steering, model / permission changes (Claude restarts on the same
//! conversation once idle), /clear, fork, and transcripts saved for the next app start.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};

use agentdeck_core::AppHandle;
use agentdeck_core::agents::AgentKind;
use agentdeck_core::chat::{ChatEvent, Item, Transcript, claude_user_line, with_resume};
use agentdeck_core::logic::deck::{Pane, PaneSpec};
use gpui_kit::*;

use super::{Prompt, changed, chats, next_codex_turn, start_claude, write_line};
use crate::state::Deck;

/// A follow-up typed while a turn runs.
#[derive(Clone)]
pub(super) struct Queued {
    pub(super) id: u64,
    pub(super) prompt: Prompt,
}

/// A fork waiting for its pane to spawn: the conversation to resume and what it showed.
pub(super) struct Fork {
    pub(super) session: String,
    pub(super) transcript: Transcript,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static FORKS: LazyLock<Mutex<HashMap<String, Fork>>> = LazyLock::new(Mutex::default);

/// The pane's queue and whether it is paused (a turn failed or was stopped).
pub(super) fn queued(pane_id: &str) -> (Vec<Queued>, bool) {
    chats().map.get(pane_id).map(|c| (c.queue.clone(), c.paused)).unwrap_or_default()
}

pub(super) fn enqueue(pane_id: &str, text: &str, images: &[String]) {
    let prompt = Prompt { text: text.trim().to_owned(), images: images.to_vec() };
    if prompt.text.is_empty() && prompt.images.is_empty() {
        return;
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    chats().map.entry(pane_id.to_owned()).or_default().queue.push(Queued { id, prompt });
    changed(pane_id);
}

/// Takes a card out of the queue.
pub(super) fn unqueue(pane_id: &str, id: u64) -> Option<Prompt> {
    let mut chats = chats();
    let queue = &mut chats.map.get_mut(pane_id)?.queue;
    let at = queue.iter().position(|q| q.id == id)?;
    let prompt = queue.remove(at).prompt;
    drop(chats);
    changed(pane_id);
    Some(prompt)
}

pub(super) fn edit_queued(pane_id: &str, id: u64, text: &str) {
    if let Some(q) = chats().map.get_mut(pane_id).and_then(|c| c.queue.iter_mut().find(|q| q.id == id)) {
        q.prompt.text = text.trim().to_owned();
    }
    changed(pane_id);
}

/// Moves a card one place up (sent sooner).
pub(super) fn raise_queued(pane_id: &str, id: u64) {
    if let Some(chat) = chats().map.get_mut(pane_id)
        && let Some(at) = chat.queue.iter().position(|q| q.id == id).filter(|&at| at > 0)
    {
        chat.queue.swap(at, at - 1);
    }
    changed(pane_id);
}

/// "Resume": unpauses the queue and sends its first card when no turn runs.
pub(super) fn resume_queue(pane: &Pane, cx: &mut App) {
    let next = {
        let mut chats = chats();
        let Some(chat) = chats.map.get_mut(&pane.id) else { return };
        chat.paused = false;
        (!chat.transcript.busy && !chat.queue.is_empty()).then(|| chat.queue.remove(0).prompt)
    };
    changed(&pane.id);
    if let Some(prompt) = next {
        super::send_with_images(pane, &prompt.text, &prompt.images, cx);
    }
}

/// Ctrl+Enter: into the running turn now. Claude reads it between steps; Codex's turn is
/// stopped and the message starts the next one on the same thread.
pub(super) fn steer(pane: &Pane, text: &str, images: &[String], cx: &mut App) {
    let busy = chats().map.get(&pane.id).is_some_and(|c| c.transcript.busy);
    if !busy || pane.agent == AgentKind::Claude {
        return super::send_with_images(pane, text, images, cx);
    }
    {
        let mut chats = chats();
        let Some(chat) = chats.map.get_mut(&pane.id) else { return };
        if let Some(mut proc) = chat.proc.take() {
            let _ = proc.child.kill();
        }
        chat.transcript.apply(ChatEvent::TurnEnd { error: None });
        chat.pending.insert(0, Prompt { text: text.trim().to_owned(), images: images.to_vec() });
    }
    changed(&pane.id);
    let id = pane.id.clone();
    cx.background_spawn(async move { next_codex_turn(&id) }).detach();
}

/// A turn ended (reader thread): the queue moves on (or pauses on failure), Claude restarts if a
/// model / mode change waited for it, and the transcript is saved.
pub(super) fn after_turn(pane_id: &str, ok: bool) {
    let mut chats = chats();
    let Some(chat) = chats.map.get_mut(pane_id) else { return };
    if !ok {
        chat.paused = !chat.queue.is_empty();
    } else if !chat.paused && !chat.queue.is_empty() {
        // Codex: next_codex_turn takes it when this process exits. Claude: written below.
        let next = chat.queue.remove(0).prompt;
        chat.pending.push(next);
    }
    let claude = chat.launch.as_ref().is_some_and(|l| l.agent == AgentKind::Claude);
    let relaunch = std::mem::take(&mut chat.relaunch) && chat.proc.is_some();
    if claude
        && !relaunch
        && let Some(proc) = chat.proc.as_mut()
    {
        for prompt in std::mem::take(&mut chat.pending) {
            write_line(proc, &claude_user_line(&prompt.line(AgentKind::Claude)));
            chat.transcript.push_user(&prompt.text, &prompt.images);
        }
    }
    drop(chats);
    if claude && relaunch {
        relaunch_claude(pane_id);
    }
    changed(pane_id);
    save(pane_id);
}

/// Restarts Claude on the same conversation with the current launch arguments (pending prompts
/// go to the new process). Errors end up in the transcript.
fn relaunch_claude(pane_id: &str) {
    {
        let mut chats = chats();
        let Some(chat) = chats.map.get_mut(pane_id) else { return };
        let Some(launch) = chat.launch.as_mut() else { return };
        launch.args = with_resume(&launch.args, chat.transcript.session.as_deref(), false);
        if let Some(mut proc) = chat.proc.take() {
            let _ = proc.child.kill();
        }
        chat.transcript.busy = false;
    }
    if let Err(e) = start_claude(pane_id)
        && let Some(chat) = chats().map.get_mut(pane_id)
    {
        chat.transcript.items.push(Item::Error(e));
    }
    changed(pane_id);
}

/// Model / permission change: `edit` rewrites the running agent's arguments. Codex uses them on
/// its next turn; Claude restarts on the same conversation now when idle, else when the turn ends.
/// A pane not started yet gets them from its spec (the caller updates it).
pub(super) fn edit_args(pane_id: &str, edit: impl Fn(&[String]) -> Vec<String>, cx: &mut App) {
    let now = {
        let mut chats = chats();
        let Some(chat) = chats.map.get_mut(pane_id) else { return };
        let Some(launch) = chat.launch.as_mut() else { return };
        launch.args = edit(&launch.args);
        let restart = launch.agent == AgentKind::Claude && chat.proc.is_some();
        if restart && chat.transcript.busy {
            chat.relaunch = true;
        }
        restart && !chat.transcript.busy
    };
    if now {
        let id = pane_id.to_owned();
        cx.background_spawn(async move { relaunch_claude(&id) }).detach();
    }
}

/// /clear: a fresh conversation in the same pane. Nothing runs on (the caller refuses during a
/// turn); Claude starts again without --resume, Codex starts a new thread on the next message.
pub(super) fn clear(pane: &Pane, deck: &Entity<Deck>, cx: &mut App) {
    let restart = {
        let mut chats = chats();
        let chat = chats.map.entry(pane.id.clone()).or_default();
        if let Some(mut proc) = chat.proc.take() {
            let _ = proc.child.kill();
        }
        chat.transcript = Transcript::default();
        chat.queue.clear();
        chat.pending.clear();
        chat.paused = false;
        chat.relaunch = false;
        let claude = pane.agent == AgentKind::Claude;
        if let Some(launch) = chat.launch.as_mut().filter(|_| claude) {
            launch.args = with_resume(&launch.args, None, false);
        }
        if let Some(path) = chat.launch.as_ref().and_then(|l| saved_path(&l.backend, &pane.id)) {
            let _ = std::fs::remove_file(path);
        }
        claude && chat.launch.is_some() && !pane.exited
    };
    deck.update(cx, |d, cx| {
        d.state.update_pane(&pane.id, |p| {
            p.spec.resume_id = None;
            p.topic = None;
        });
        d.save(cx);
        cx.notify();
    });
    changed(&pane.id);
    if restart {
        let id = pane.id.clone();
        cx.background_spawn(async move {
            if let Err(e) = start_claude(&id) {
                eprintln!("chat {id}: restart after /clear: {e}");
            }
            changed(&id);
        })
        .detach();
    }
}

/// "Fork from here": a new chat pane (same agent, folder, model, mode) continuing a copy of this
/// conversation. Claude only (`--resume <id> --fork-session`).
pub(super) fn fork(pane: &Pane, deck: &Entity<Deck>, cx: &mut App) -> Result<(), String> {
    let (cwd, session, mut transcript) = {
        let chats = chats();
        let chat = chats.map.get(&pane.id).ok_or("No conversation to fork")?;
        let session = chat.transcript.session.clone().ok_or("No conversation to fork yet")?;
        let cwd = chat.launch.as_ref().map_or_else(|| pane.cwd.clone(), |l| l.cwd.clone());
        (cwd, session, chat.transcript.clone())
    };
    transcript.busy = false;
    transcript.started = None;
    // The worktree folder is the conversation's project: the fork works there, not in a new one.
    let mut spec = PaneSpec::new(pane.agent, format!("{} (fork)", pane.title), cwd);
    spec.args = pane.args.clone();
    spec.chat = true;
    spec.font_size = pane.font_size;
    spec.color = pane.color.clone();
    FORKS.lock().unwrap_or_else(PoisonError::into_inner).insert(spec.id.clone(), Fork { session, transcript });
    let state = &deck.read(cx).state;
    let tab = state
        .tabs
        .iter()
        .find(|t| t.pane_ids.contains(&pane.id))
        .map_or_else(|| state.active_tab_id.clone(), |t| t.id.clone());
    crate::panes::runtime::create_pane(spec, &tab, false, cx);
    Ok(())
}

/// The fork a new pane was created for (taken once, at its first spawn).
pub(super) fn take_fork(pane_id: &str) -> Option<Fork> {
    FORKS.lock().unwrap_or_else(PoisonError::into_inner).remove(pane_id)
}

fn saved_path(backend: &AppHandle, pane_id: &str) -> Option<PathBuf> {
    Some(backend.app_data_dir().ok()?.join("chats").join(format!("{pane_id}.json")))
}

/// Writes the pane's transcript (capped) under the app data folder. Blocking: call off the UI thread.
pub(super) fn save(pane_id: &str) {
    let (backend, json) = {
        let chats = chats();
        let Some(chat) = chats.map.get(pane_id) else { return };
        let Some(launch) = &chat.launch else { return };
        if chat.transcript.items.is_empty() {
            return;
        }
        (launch.backend.clone(), chat.transcript.to_json())
    };
    let Some(path) = saved_path(&backend, pane_id) else { return };
    let result = path
        .parent()
        .map_or(Ok(()), |dir| std::fs::create_dir_all(dir).map_err(|e| e.to_string()))
        .and_then(|()| agentdeck_core::persist::write_atomic(&path, json.as_bytes()));
    if let Err(e) = result {
        eprintln!("chat {pane_id}: transcript not saved: {e}");
    }
}

/// The transcript saved for this pane, when it belongs to `session` (the conversation resumed).
pub(super) fn load(backend: &AppHandle, pane_id: &str, session: &str) -> Option<Transcript> {
    let text = std::fs::read_to_string(saved_path(backend, pane_id)?).ok()?;
    Transcript::from_json(&text).filter(|t| t.session.as_deref() == Some(session))
}
