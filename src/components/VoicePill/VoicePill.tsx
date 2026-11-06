// Floating dictation pill: live mic bars, model download progress, transcribing dots, errors, last transcript.
import { useEffect, useRef, useState } from "react";
import {
  onVoiceDownload,
  onVoiceError,
  onVoiceLevel,
  onVoicePartial,
  onVoiceState,
  onVoiceTranscript,
} from "../../lib/ipc";
import type { VoiceState } from "../../lib/types";

// ~30 levels/s from Rust: 28 bars show the last ~0.9 s of real mic signal, newest on the right.
const BAR_COUNT = 28;
const MIN_HEIGHT = 0.1;
/** Below this the mic is just room noise: draw flat so silence reads as silence. */
const NOISE_GATE = 0.2;
/** Per-frame easing toward the target height, so bars glide instead of jumping. */
const EASE = 0.35;
const BAR_KEYS = Array.from({ length: BAR_COUNT }, (_, i) => i);

const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

function Waveform() {
  const barsRef = useRef<(HTMLSpanElement | null)[]>([]);

  useEffect(() => {
    const history = new Float32Array(BAR_COUNT);
    const shown = new Float32Array(BAR_COUNT);
    const ease = reducedMotion() ? 1 : EASE;
    let frame = 0;
    const tick = () => {
      barsRef.current.forEach((bar, i) => {
        shown[i] += (history[i] - shown[i]) * ease;
        if (bar) bar.style.transform = `scaleY(${MIN_HEIGHT + shown[i] * (1 - MIN_HEIGHT)})`;
      });
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    const unlisten = onVoiceLevel((level) => {
      // Scroll left: values and their eased heights move together, so motion stays smooth.
      history.copyWithin(0, 1);
      shown.copyWithin(0, 1);
      history[BAR_COUNT - 1] = level < NOISE_GATE ? 0 : (level - NOISE_GATE) / (1 - NOISE_GATE);
    });
    return () => {
      cancelAnimationFrame(frame);
      unlisten.then((off) => off()).catch(() => {});
    };
  }, []);

  return (
    <div
      className="flex h-7 items-center gap-[2px] [mask-image:linear-gradient(to_right,transparent,black_30%)]"
      aria-hidden="true"
    >
      {BAR_KEYS.map((i) => (
        <span
          key={i}
          ref={(el) => {
            barsRef.current[i] = el;
          }}
          className="h-full w-[3px] origin-center rounded-full bg-white will-change-transform"
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

/** Shown once the transcript went out: a green disc whose tick draws itself (CSS in index.css). */
function CheckMark() {
  return (
    <svg className="voice-check size-6 shrink-0" viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="11" fill="#22c55e" />
      <path
        d="M7 12.5l3.2 3.2L17 9"
        fill="none"
        stroke="white"
        strokeWidth="2.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function VoicePill() {
  const [state, setState] = useState<VoiceState>("idle");
  const [error, setError] = useState("");
  const [transcript, setTranscript] = useState("");
  const [partial, setPartial] = useState("");
  const [percent, setPercent] = useState<number | null>(null);

  useEffect(() => {
    const unlisteners = [
      onVoiceState((next) => {
        setState(next);
        if (next === "listening") {
          setTranscript("");
          setPartial("");
        }
      }),
      onVoiceError(setError),
      onVoiceTranscript(setTranscript),
      onVoicePartial(setPartial),
      onVoiceDownload(({ received, total }) => setPercent(total ? Math.floor((received * 100) / total) : null)),
    ];
    return () => {
      for (const unlisten of unlisteners) unlisten.then((off) => off()).catch(() => {});
    };
  }, []);

  const label = {
    idle: transcript || "Ready",
    listening: partial || "Listening",
    downloading: `Downloading model${percent === null ? "…" : ` ${percent}%`}`,
    transcribing: "Transcribing…",
    error: error || "Voice error",
  }[state];

  return (
    <div className="flex h-screen items-center justify-center bg-transparent">
      <div
        data-tauri-drag-region
        role="status"
        aria-live="polite"
        aria-label={label}
        className={`flex max-w-[440px] cursor-default select-none items-center gap-3 bg-black/90 px-5 shadow-lg ring-1 ring-white/10 ${
          state === "listening" && partial ? "min-h-12 rounded-3xl py-1.5" : "h-12 rounded-full"
        }`}
      >
        {state === "listening" && (
          <div data-tauri-drag-region className="flex min-w-0 flex-col items-center">
            {partial && (
              // Live words above the waveform: rtl box + ltr text keeps the last words, "…" replaces the start.
              <span data-tauri-drag-region dir="rtl" className="max-w-[400px] truncate text-sm leading-5 text-white/85">
                <bdi dir="ltr">{partial}</bdi>
              </span>
            )}
            <Waveform />
          </div>
        )}
        {state === "transcribing" && partial && (
          <span data-tauri-drag-region dir="rtl" className="min-w-0 truncate text-sm text-white/60">
            <bdi dir="ltr">{partial}</bdi>
          </span>
        )}
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
        {state === "idle" && transcript && <CheckMark />}
        {state === "idle" && (
          <span data-tauri-drag-region className="truncate text-sm text-white/70">
            {label}
          </span>
        )}
      </div>
    </div>
  );
}
