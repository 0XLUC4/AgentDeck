import { useEffect, useRef, useState } from "react";
import { onVoiceLevel } from "../../lib/ipc";
import { useVoiceActivity, useVoiceTarget } from "../../lib/voiceListener";

/** How long the "delivered" flash plays on the pane that received a dictation. */
const SENT_FLASH_MS = 900;

/**
 * Neon outline on the pane you are talking to: a light runs around the border while you speak, its glow
 * follows your voice, and the pane flashes once the words land in it. Styles in index.css (.voice-glow).
 */
export function VoiceGlow({ paneId }: { paneId: string }) {
  const target = useVoiceTarget() === paneId;
  const state = useVoiceActivity((s) => s.state);
  const sentAt = useVoiceActivity((s) => (s.sentTo === paneId ? s.sentAt : 0));
  const [flash, setFlash] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const live = target && (state === "listening" || state === "transcribing");

  useEffect(() => {
    if (!sentAt) return;
    setFlash(true);
    const timer = setTimeout(() => setFlash(false), SENT_FLASH_MS);
    return () => clearTimeout(timer);
  }, [sentAt]);

  // Mic level straight into a CSS variable: no React render per level event.
  useEffect(() => {
    if (!live || state !== "listening") return;
    let smooth = 0;
    const unlisten = onVoiceLevel((level) => {
      smooth += (level - smooth) * 0.4;
      ref.current?.style.setProperty("--voice-level", smooth.toFixed(3));
    });
    return () => {
      unlisten.then((off) => off()).catch(() => {});
      ref.current?.style.setProperty("--voice-level", "0");
    };
  }, [live, state]);

  if (!live && !flash) return null;
  return (
    <div
      ref={ref}
      aria-hidden="true"
      className={`voice-glow ${live ? `voice-glow-${state}` : ""} ${flash ? "voice-glow-sent" : ""}`}
    />
  );
}
