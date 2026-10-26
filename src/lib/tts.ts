// Spoken task summaries (Web Speech API): "<Agent> a terminé : <summary>" when a task ends unseen.
import { useEffect, useState } from "react";
import { useDeck } from "../store/deck";
import { useSettings } from "../store/settings";
import { useTasks } from "../store/tasks";
import { AGENTS } from "./agents";
import { getVoiceConfig } from "./ipc";

export const SPEECH_MAX_CHARS = 200;

/** Markdown/code to plain speakable text, cut at a word boundary after `max` chars. */
export function cleanForSpeech(text: string, max = SPEECH_MAX_CHARS): string {
  const plain = text
    .replace(/```[\s\S]*?(```|$)/g, " ") // code blocks are not worth hearing
    .replace(/`([^`]*)`/g, "$1")
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/https?:\/\/\S+/g, " ")
    .replace(/^\s*(#{1,6}|>|[-*+]|\d+[.)])\s+/gm, "")
    .replace(/[*~#|]+/g, "")
    .replace(/\s+/g, " ")
    .trim();
  if (plain.length <= max) return plain;
  const cut = plain.slice(0, max);
  const space = cut.lastIndexOf(" ");
  return `${(space > max * 0.6 ? cut.slice(0, space) : cut).replace(/[\s,;:.-]+$/, "")}…`;
}

export function speechText(agentName: string, summary: string | null): string {
  const clean = summary ? cleanForSpeech(summary) : "";
  return clean ? `${agentName} a terminé : ${clean}` : `${agentName} a terminé`;
}

const supported = () => typeof window !== "undefined" && "speechSynthesis" in window;

export function stopSpeaking(): void {
  if (supported() && (speechSynthesis.speaking || speechSynthesis.pending)) speechSynthesis.cancel();
}

/** Speaks with the chosen voice, else the first voice of the dictation language (fr-FR by default). */
export async function speak(text: string): Promise<void> {
  if (!supported()) return;
  const { voice: voiceUri, rate } = useSettings.getState().speech;
  const language = await getVoiceConfig().then(
    (c) => (c.language && c.language !== "auto" ? c.language : "fr"),
    () => "fr",
  );
  const voices = speechSynthesis.getVoices();
  const voice =
    voices.find((v) => v.voiceURI === voiceUri) ??
    voices.find((v) => v.lang.toLowerCase().startsWith(language.toLowerCase()));
  const utterance = new SpeechSynthesisUtterance(text);
  utterance.lang = voice?.lang ?? (language === "fr" ? "fr-FR" : language);
  if (voice) utterance.voice = voice;
  utterance.rate = rate;
  speechSynthesis.cancel();
  speechSynthesis.speak(utterance);
}

/** Installed voices; the list fills asynchronously in most engines. */
export function useSpeechVoices(): SpeechSynthesisVoice[] {
  const [voices, setVoices] = useState<SpeechSynthesisVoice[]>(() => (supported() ? speechSynthesis.getVoices() : []));
  useEffect(() => {
    if (!supported()) return;
    const sync = () => setVoices(speechSynthesis.getVoices());
    speechSynthesis.addEventListener("voiceschanged", sync);
    return () => speechSynthesis.removeEventListener("voiceschanged", sync);
  }, []);
  return voices;
}

/** Speaks finished tasks the user isn't looking at; any key press silences it. Call once (main window). */
export function startTaskSpeech(): () => void {
  const unsubscribe = useDeck.subscribe((state, prev) => {
    if (state.panes === prev.panes || !useSettings.getState().speech.enabled) return;
    for (const pane of Object.values(state.panes)) {
      if (pane.status !== "done" || prev.panes[pane.id]?.status === "done") continue;
      // Same condition as notifications: window in the background or another pane active.
      if (document.hasFocus() && state.activePaneId === pane.id) continue;
      // tasks.apply runs before the status update, so the finished task is already archived.
      const summary = useTasks.getState()[pane.id]?.history[0]?.summary ?? null;
      if (summary) void speak(speechText(AGENTS[pane.agent].name, summary)).catch((err) => console.warn("tts", err));
    }
  });
  window.addEventListener("keydown", stopSpeaking, true);
  return () => {
    unsubscribe();
    window.removeEventListener("keydown", stopSpeaking, true);
  };
}
