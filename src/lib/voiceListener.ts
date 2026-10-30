// Main window side of voice input: Rust emits the transcript, we pick the target pane and type it.

import { create } from "zustand";
import { sendPrompt } from "../components/Broadcast/send";
import { activeWorkspace, deck, useDeck } from "../store/deck";
import { getVoiceConfig, onVoiceTranscript } from "./ipc";
import { type RouteCandidate, routeTranscript } from "./voiceRoute";

const RECENT_KEY = "voice-recent";
const RECENT_MAX = 50;

/** Pane ids, most recently focused or addressed by voice first. Saved: pane ids survive a restart. */
const useRecent = create<string[]>(() => {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(saved) ? saved.filter((id): id is string => typeof id === "string") : [];
  } catch {
    return [];
  }
});

const touch = (id: string) => {
  const recent = [id, ...useRecent.getState().filter((r) => r !== id)].slice(0, RECENT_MAX);
  useRecent.setState(recent, true);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
  } catch {
    // Storage unavailable: the order just won't survive a restart.
  }
};
useDeck.subscribe((state, prev) => {
  const id = state.activePaneId;
  if (id && id !== prev.activePaneId) touch(id);
});

/** Live panes of the active workspace: current session first, then most recently focused. */
export function voiceCandidates(state = useDeck.getState(), recent = useRecent.getState()): RouteCandidate[] {
  const workspaceId = activeWorkspace(state).id;
  const session = new Set(state.tabs.find((t) => t.id === state.activeTabId)?.paneIds);
  const rank = (id: string) => (recent.includes(id) ? recent.indexOf(id) : recent.length);
  return state.tabs
    .filter((t) => t.workspaceId === workspaceId)
    .flatMap((t) => t.paneIds.map((id) => state.panes[id]))
    .filter((p) => p && !p.exited)
    .sort((a, b) => Number(session.has(b.id)) - Number(session.has(a.id)) || rank(a.id) - rank(b.id));
}

/** Where an unnamed dictation goes: the last pane used in the active workspace, else its first live pane. */
function voiceTarget(
  state: ReturnType<typeof useDeck.getState>,
  recent: string[],
  candidates = voiceCandidates(state, recent),
) {
  return recent.find((id) => candidates.some((c) => c.id === id)) ?? candidates[0]?.id ?? null;
}

/** Id of the pane the next unnamed dictation goes to, kept in sync for the header indicator. */
export function useVoiceTarget(): string | null {
  const recent = useRecent();
  return useDeck((s) => voiceTarget(s, recent));
}

/**
 * Subscribes once (main window): "Codex, …" goes to that pane and makes it the workspace's voice target; anything
 * else goes to the last pane used in the active workspace, never to a hidden pane of another workspace.
 */
export function listenVoiceTranscripts() {
  return onVoiceTranscript(async (transcript) => {
    try {
      const { submit } = await getVoiceConfig();
      const state = useDeck.getState();
      const recent = useRecent.getState();
      const candidates = voiceCandidates(state, recent);
      const routed = routeTranscript(transcript, candidates);
      const paneId = routed.paneId ?? voiceTarget(state, recent, candidates);
      if (!paneId) throw new Error("no running agent in this workspace");
      touch(paneId);
      await sendPrompt(paneId, routed.text, submit);
    } catch (err) {
      deck.toast(`Dictation not sent: ${err instanceof Error ? err.message : String(err)}`);
    }
  });
}
