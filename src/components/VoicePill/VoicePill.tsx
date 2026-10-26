// Floating dictation pill: live mic bars, model download progress, transcribing dots, errors, last transcript.
import { useEffect, useRef, useState } from "react";
import { onVoiceDownload, onVoiceError, onVoiceLevel, onVoiceState, onVoiceTranscript } from "../../lib/ipc";
import type { VoiceState } from "../../lib/types";

const BAR_COUNT = 14;
const DECAY = 0.88;
const MIN_HEIGHT = 0.12;
// Center bars are taller, like a waveform envelope.
const BAR_WEIGHTS = Array.from({ length: BAR_COUNT }, (_, i) => {
  const distance = Math.abs(i - (BAR_COUNT - 1) / 2) / (BAR_COUNT / 2);
  return 1 - distance * 0.6;
});

const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

function Bars() {
  const barsRef = useRef<(HTMLSpanElement | null)[]>([]);
  const targetRef = useRef(0);

  useEffect(() => {
    let shown = 0;
    let frame = 0;
    const still = reducedMotion();
    const tick = (time: number) => {
      // Rise instantly, fall smoothly.
      shown = Math.max(targetRef.current, shown * DECAY);
      barsRef.current.forEach((bar, i) => {
        if (!bar) return;
        const wobble = still ? 1 : 0.75 + 0.25 * Math.sin(time / 90 + i * 1.7);
        const height = Math.max(MIN_HEIGHT, shown * BAR_WEIGHTS[i] * wobble);
        bar.style.transform = `scaleY(${height})`;
      });
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    const unlisten = onVoiceLevel((level) => {
      targetRef.current = level;
    });
    return () => {
      cancelAnimationFrame(frame);
      unlisten.then((off) => off()).catch(() => {});
    };
  }, []);

  return (
    <div className="flex h-6 items-center gap-[3px]" aria-hidden="true">
      {BAR_WEIGHTS.map((_, i) => (
        <span
          // biome-ignore lint/suspicious/noArrayIndexKey: static, fixed-length list
          key={i}
          ref={(el) => {
            barsRef.current[i] = el;
          }}
          className="h-full w-[3px] origin-center rounded-full bg-white"
          style={{ transform: `scaleY(${MIN_HEIGHT})` }}
        />
      ))}
    </div>
  );
}

function Dots() {
  return (
    <div className="flex h-6 items-center gap-1.5" aria-hidden="true">
      {[0, 150, 300].map((delay) => (
        <span
          key={delay}
          className="size-1.5 animate-pulse rounded-full bg-white/80 motion-reduce:animate-none"
          style={{ animationDelay: `${delay}ms` }}
        />
      ))}
    </div>
  );
}

export function VoicePill() {
  const [state, setState] = useState<VoiceState>("idle");
  const [error, setError] = useState("");
  const [transcript, setTranscript] = useState("");
  const [percent, setPercent] = useState<number | null>(null);

  useEffect(() => {
    const unlisteners = [
      onVoiceState((next) => {
        setState(next);
        if (next === "listening") setTranscript("");
      }),
      onVoiceError(setError),
      onVoiceTranscript(setTranscript),
      onVoiceDownload(({ received, total }) => setPercent(total ? Math.floor((received * 100) / total) : null)),
    ];
    return () => {
      for (const unlisten of unlisteners) unlisten.then((off) => off()).catch(() => {});
    };
  }, []);

  const label = {
    idle: transcript || "Prêt",
    listening: "Écoute en cours",
    downloading: `Téléchargement du modèle${percent === null ? "…" : ` ${percent} %`}`,
    transcribing: "Transcription…",
    error: error || "Erreur vocale",
  }[state];

  return (
    <div className="flex h-screen items-center justify-center bg-transparent">
      <div
        data-tauri-drag-region
        role="status"
        aria-live="polite"
        aria-label={label}
        className="flex h-12 max-w-[300px] cursor-default select-none items-center gap-3 rounded-full bg-black/90 px-5 shadow-lg ring-1 ring-white/10"
      >
        {state === "listening" && <Bars />}
        {state === "transcribing" && <Dots />}
        {state === "error" && (
          <span data-tauri-drag-region className="truncate text-sm text-red-400">
            {label}
          </span>
        )}
        {state === "downloading" && (
          <span data-tauri-drag-region className="truncate text-sm text-white/70 tabular-nums">
            {label}
          </span>
        )}
        {state === "idle" && (
          <span data-tauri-drag-region className="truncate text-sm text-white/70">
            {label}
          </span>
        )}
      </div>
    </div>
  );
}
