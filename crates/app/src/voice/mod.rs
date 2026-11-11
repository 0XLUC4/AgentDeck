//! Voice: dictation pill on the target pane (level meter, check mark), dictation
//! routing to a pane ("Codex, run the tests"), voice glow data for the target pane, spoken task
//! summaries (TTS).
//! Original: src/components/VoicePill, src/voice.tsx, src/lib/{voiceListener,voiceRoute,tts}.ts.
//! Backend: agentdeck_core::voice; logic: agentdeck_core::logic::{voice_route,text::speech_text}.

mod pill;
pub mod tts;

use std::collections::HashMap;
use std::time::Duration;

use agentdeck_core::hooks_server::PaneStatus;
use agentdeck_core::logic::deck::DeckState;
use agentdeck_core::logic::queue::{SUBMIT_DELAY_MS, prompt_text};
use agentdeck_core::logic::text::{agent_name, now_ms, speech_text};
use agentdeck_core::logic::voice_route::{RouteCandidate, route_transcript};
use agentdeck_core::persist::{JsonStore, LOCAL_FILE};
use agentdeck_core::{AppHandle, AppState};
use gpui_kit::*;

pub use pill::VoicePill;

use crate::layout::DeckView;
use crate::state::{Deck, DeckEvent};

const RECENT_KEY: &str = "voice-recent";
const RECENT_MAX: usize = 50;
/// Distance between the pill and the bottom of the target pane, in logical px.
const PILL_BOTTOM: f32 = 16.0;

/// Dictation as seen by the main window, for the pane glow: recording state and the last delivery.
#[derive(Debug, Clone, Default)]
pub struct VoiceActivity {
    /// "idle" | "listening" | "downloading" | "transcribing" | "error".
    pub state: String,
    pub sent_to: Option<String>,
    pub sent_at: i64,
}

impl Global for VoiceActivity {}

/// Current dictation activity (`state` is "idle" before the first recording).
pub fn voice_activity(cx: &App) -> VoiceActivity {
    cx.try_global::<VoiceActivity>().cloned().unwrap_or_default()
}

/// Id of the pane the next unnamed dictation goes to (header indicator).
pub fn voice_target(cx: &App) -> Option<String> {
    let voice = cx.try_global::<VoiceGlobal>()?.0.read(cx);
    let state = &voice.deck.read(cx).state;
    target(&voice.recent, &candidates(state, &voice.recent))
}

/// The dictation pill, pinned bottom-center of the target pane while dictation runs.
pub fn pill_overlay(cx: &App) -> Option<AnyElement> {
    let voice = cx.try_global::<VoiceGlobal>()?.0.read(cx);
    voice.pill_visible.then(|| {
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(PILL_BOTTOM))
            .flex()
            .justify_center()
            .child(voice.pill.clone())
            .into_any_element()
    })
}

struct VoiceGlobal(Entity<Voice>);

impl Global for VoiceGlobal {}

struct Voice {
    deck: Entity<Deck>,
    /// Pane ids, most recently focused or addressed by voice first (saved in local.json).
    recent: Vec<String>,
    last_active: Option<String>,
    statuses: HashMap<String, PaneStatus>,
    pill: Entity<VoicePill>,
    pill_visible: bool,
}

/// Registers the voice listener on the main window's deck, and the "any key silences speech" hook.
pub fn init(cx: &mut App) {
    cx.set_global(VoiceActivity { state: "idle".into(), ..Default::default() });
    cx.observe_new::<DeckView>(|view, _, cx| {
        if cx.has_global::<VoiceGlobal>() {
            return;
        }
        let deck = view.deck().clone();
        let voice = cx.new(|cx| Voice::new(deck, cx));
        cx.set_global(VoiceGlobal(voice));
    })
    .detach();
    cx.observe_keystrokes(|_, _, _| tts::stop()).detach();
}

impl Voice {
    fn new(deck: Entity<Deck>, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&deck, Self::on_event).detach();
        cx.observe(&deck, Self::on_change).detach();
        let data_dir = deck.read(cx).data_dir.clone();
        let recent = JsonStore::load(&data_dir, LOCAL_FILE)
            .map(|s| s.get(RECENT_KEY))
            .ok()
            .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
            .unwrap_or_default();
        let state = &deck.read(cx).state;
        let statuses = state.panes.iter().map(|(id, p)| (id.clone(), p.status)).collect();
        let last_active = state.active_pane_id.clone();
        let pill = cx.new(|cx| VoicePill::new(deck.clone(), cx));
        Self { deck, recent, last_active, statuses, pill, pill_visible: false }
    }

    /// Moves `id` to the front of the recent list and saves it.
    fn touch(&mut self, id: &str, cx: &App) {
        self.recent.retain(|r| r != id);
        self.recent.insert(0, id.to_owned());
        self.recent.truncate(RECENT_MAX);
        let data_dir = self.deck.read(cx).data_dir.clone();
        let saved = JsonStore::load(&data_dir, LOCAL_FILE).and_then(|mut store| {
            store.set(RECENT_KEY, serde_json::json!(self.recent));
            store.save()
        });
        if let Err(e) = saved {
            // Storage unavailable: the order just won't survive a restart.
            eprintln!("voice recent not saved: {e}");
        }
    }

    /// Active pane changes feed the recent list; finished tasks are spoken when unseen.
    fn on_change(&mut self, deck: Entity<Deck>, cx: &mut Context<Self>) {
        let active = deck.read(cx).state.active_pane_id.clone();
        if active != self.last_active {
            self.last_active = active.clone();
            if let Some(id) = active {
                self.touch(&id, cx);
            }
        }
        let focused = crate::system::is_main_focused(cx);
        let d = deck.read(cx);
        let mut speak = Vec::new();
        for pane in d.state.panes.values() {
            let was = self.statuses.insert(pane.id.clone(), pane.status);
            if pane.status != PaneStatus::Done || was == Some(PaneStatus::Done) || !d.settings.speech.enabled {
                continue;
            }
            // Same condition as notifications: window in the background or another pane active.
            if focused && d.state.active_pane_id.as_deref() == Some(pane.id.as_str()) {
                continue;
            }
            let summary = d.tasks.get(&pane.id).and_then(|t| t.history.first()).and_then(|t| t.summary.clone());
            if let Some(summary) = summary {
                speak.push(speech_text(agent_name(pane.agent), Some(&summary)));
            }
        }
        self.statuses.retain(|id, _| d.state.panes.contains_key(id));
        if speak.is_empty() {
            return;
        }
        let speech = d.settings.speech.clone();
        let backend = d.backend.clone();
        for text in speak {
            tts::speak(&backend, &speech, text);
        }
    }

    fn on_event(&mut self, deck: Entity<Deck>, event: &DeckEvent, cx: &mut Context<Self>) {
        let DeckEvent::Backend(event) = event;
        let text = event.payload.as_str().unwrap_or("").to_owned();
        match event.name.as_str() {
            "voice-pill" => self.set_pill(text == "show", cx),
            // The pill truncates long errors: show the full message here too.
            "voice-error" => deck.update(cx, |d, cx| d.toast(format!("Voice: {text}"), cx)),
            "voice-state" => cx.global_mut::<VoiceActivity>().state = text,
            "voice-transcript" => self.deliver(&text, cx),
            _ => {}
        }
    }

    /// "Codex, ..." goes to that pane and makes it the voice target; anything else goes to the last
    /// pane used in the active workspace, never to a hidden pane of another workspace.
    fn deliver(&mut self, transcript: &str, cx: &mut Context<Self>) {
        let (backend, routed, pane_id) = {
            let d = self.deck.read(cx);
            let candidates = candidates(&d.state, &self.recent);
            let routed = route_transcript(transcript, &candidates);
            let pane_id = routed.pane_id.clone().or_else(|| target(&self.recent, &candidates));
            (d.backend.clone(), routed, pane_id)
        };
        let Some(pane_id) = pane_id else {
            self.deck.update(cx, |d, cx| d.toast("Dictation not sent: no running agent in this project", cx));
            return;
        };
        self.touch(&pane_id, cx);
        let submit = match agentdeck_core::voice::voice_get_config(&backend) {
            Ok(view) => view.config.submit,
            Err(e) => {
                self.deck.update(cx, |d, cx| d.toast(format!("Dictation not sent: {e}"), cx));
                return;
            }
        };
        let send = send_prompt(&backend, pane_id.clone(), &routed.text, submit, cx);
        let deck = self.deck.clone();
        cx.spawn(async move |_, cx| {
            let result = send.await;
            cx.update(|cx| match result {
                Ok(()) => {
                    let activity = cx.global_mut::<VoiceActivity>();
                    activity.sent_to = Some(pane_id);
                    activity.sent_at = now_ms();
                    deck.update(cx, |_, cx| cx.notify());
                }
                Err(e) => deck.update(cx, |d, cx| d.toast(format!("Dictation not sent: {e}"), cx)),
            })
        })
        .detach();
    }

    /// Shows or hides the pill on the target pane; the deck repaint redraws the panes.
    fn set_pill(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.pill_visible != visible {
            self.pill_visible = visible;
            self.deck.update(cx, |_, cx| cx.notify());
        }
    }
}

/// Live panes of the active workspace: current session first, then most recently focused.
fn candidates(state: &DeckState, recent: &[String]) -> Vec<RouteCandidate> {
    let workspace_id = state.active_workspace().id.clone();
    let session: Vec<&String> = state.active_tab().map(|t| t.pane_ids.iter().collect()).unwrap_or_default();
    let rank = |id: &str| recent.iter().position(|r| r == id).unwrap_or(recent.len());
    let mut list: Vec<RouteCandidate> = state
        .tabs
        .iter()
        .filter(|t| t.workspace_id == workspace_id)
        .flat_map(|t| t.pane_ids.iter().filter_map(|id| state.panes.get(id)))
        .filter(|p| !p.exited)
        .map(|p| RouteCandidate { id: p.id.clone(), agent: p.agent, title: p.title.clone() })
        .collect();
    list.sort_by_key(|c| (!session.contains(&&c.id), rank(&c.id)));
    list
}

/// Where an unnamed dictation goes: the last pane used in the active workspace, else its first live pane.
fn target(recent: &[String], candidates: &[RouteCandidate]) -> Option<String> {
    recent
        .iter()
        .find(|id| candidates.iter().any(|c| &c.id == *id))
        .cloned()
        .or_else(|| candidates.first().map(|c| c.id.clone()))
}

/// Types the prompt, then presses Enter in a separate write when `submit` (agent TUIs read text and
/// Enter in one write as a paste).
/// shortcut: local copy of Broadcast/send.ts sendPrompt; switch to the palette module's sender once it exists.
pub(crate) fn send_prompt(
    backend: &AppHandle,
    pane_id: String,
    text: &str,
    submit: bool,
    cx: &App,
) -> Task<Result<(), String>> {
    let backend = backend.clone();
    let data = prompt_text(text);
    let executor = cx.background_executor().clone();
    cx.background_spawn(async move {
        let state = backend.try_state::<AppState>().ok_or("the backend is not ready")?;
        agentdeck_core::pty::write_to_pane(&state, pane_id.clone(), data)?;
        if submit {
            executor.timer(Duration::from_millis(SUBMIT_DELAY_MS)).await;
            agentdeck_core::pty::write_to_pane(&state, pane_id, "\r".into())?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::{RouteCandidate, target};
    use agentdeck_core::agents::AgentKind;

    fn cand(id: &str) -> RouteCandidate {
        RouteCandidate { id: id.into(), agent: AgentKind::Shell, title: id.into() }
    }

    #[test]
    fn target_prefers_recent_then_first() {
        let list = [cand("a"), cand("b")];
        assert_eq!(target(&["x".into(), "b".into()], &list), Some("b".into()));
        assert_eq!(target(&[], &list), Some("a".into()));
        assert_eq!(target(&["a".into()], &[]), None);
    }
}
