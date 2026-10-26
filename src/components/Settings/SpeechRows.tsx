// Réglages > Notifications: spoken summary of finished tasks (Web Speech API).
import { Volume2 } from "lucide-react";
import { speak, useSpeechVoices } from "../../lib/tts";
import { type Settings, updateSettings } from "../../store/settings";
import { button, field, Row, Toggle } from "./controls";

export function SpeechRows({ s, save }: { s: Settings; save: (task: Promise<unknown>) => void }) {
  const voices = useSpeechVoices();
  const set = (patch: Partial<Settings["speech"]>) => save(updateSettings({ speech: { ...s.speech, ...patch } }));
  return (
    <>
      <Row
        label="Lire le résumé à voix haute"
        hint="Quand une tâche se termine et que vous ne regardez pas ; une touche coupe la lecture"
        htmlFor="set-tts"
      >
        <Toggle id="set-tts" checked={s.speech.enabled} onChange={(v) => set({ enabled: v })} />
      </Row>
      {s.speech.enabled && (
        <>
          <Row label="Voix" hint="Par défaut : la langue de la dictée, sinon français" htmlFor="set-tts-voice">
            <select
              id="set-tts-voice"
              value={s.speech.voice}
              onChange={(e) => set({ voice: e.target.value })}
              className={`${field} w-64`}
            >
              <option value="">Automatique</option>
              {voices.map((v) => (
                <option key={v.voiceURI} value={v.voiceURI}>
                  {v.name} ({v.lang})
                </option>
              ))}
            </select>
            <button
              type="button"
              className={button}
              onClick={() => void speak("Claude Code a terminé : test de voix.")}
            >
              <Volume2 size={14} strokeWidth={1.5} aria-hidden="true" />
              Tester
            </button>
          </Row>
          <Row label="Débit" htmlFor="set-tts-rate">
            <input
              id="set-tts-rate"
              type="range"
              min={0.5}
              max={2}
              step={0.1}
              value={s.speech.rate}
              onChange={(e) => set({ rate: e.target.valueAsNumber })}
              className="w-36 accent-[var(--color-accent)]"
            />
            <span className="w-10 text-right font-mono text-xs text-muted tabular-nums">
              {s.speech.rate.toFixed(1)}×
            </span>
          </Row>
        </>
      )}
    </>
  );
}
