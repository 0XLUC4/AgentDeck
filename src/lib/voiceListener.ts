// Main window side of voice input: Rust emits the transcript, we pick the target pane and type it.

import { sendPrompt } from "../components/Broadcast/send";
import { activeWorkspace, deck, useDeck } from "../store/deck";
import { getVoiceConfig, onVoiceTranscript, voiceSend } from "./ipc";
import { type RouteCandidate, routeTranscript } from "./voiceRoute";

/** Pane ids, most recently focused first. */
const recent: string[] = [];
useDeck.subscribe((state, prev) => {
  const id = state.activePaneId;
  if (!id || id === prev.activePaneId) return;
  const at = recent.indexOf(id);
  if (at >= 0) recent.splice(at, 1);
  recent.unshift(id);
});

/** Live panes of the active workspace: current session first, then most recently focused. */
export function voiceCandidates(state = useDeck.getState()): RouteCandidate[] {
  const workspaceId = activeWorkspace(state).id;
  const session = new Set(state.tabs.find((t) => t.id === state.activeTabId)?.paneIds);
  const rank = (id: string) => (recent.includes(id) ? recent.indexOf(id) : recent.length);
  return state.tabs
    .filter((t) => t.workspaceId === workspaceId)
    .flatMap((t) => t.paneIds.map((id) => state.panes[id]))
    .filter((p) => p && !p.exited)
    .sort((a, b) => Number(session.has(b.id)) - Number(session.has(a.id)) || rank(a.id) - rank(b.id));
}

/** Subscribes once (main window): "Codex, …" goes to that pane, anything else to the active pane as before. */
export function listenVoiceTranscripts() {
  return onVoiceTranscript(async (transcript) => {
    try {
      const { submit } = await getVoiceConfig();
      const { paneId, text } = routeTranscript(transcript, voiceCandidates());
      if (paneId) await sendPrompt(paneId, text, submit);
      else await voiceSend(text, submit);
    } catch (err) {
      deck.toast(`Dictation not sent: ${err instanceof Error ? err.message : String(err)}`);
    }
  });
}
