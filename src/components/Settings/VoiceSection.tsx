import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Check, Download, KeyRound, Trash2 } from "lucide-react";
import { type KeyboardEvent, useCallback, useEffect, useState } from "react";
import { downloadWhisperModel, getVoiceConfig, listWhisperModels, setVoiceApiKey, setVoiceConfig } from "../../lib/ipc";
import { formatShortcut, toTauriShortcut } from "../../lib/shortcut";
import type { ModelStatus, VoiceConfig, WhisperModel } from "../../lib/types";
import { button, ErrorText, errorMessage, field, Row, Section, Toggle } from "./controls";
import type { Save } from "./SettingsDialog";

const LANGUAGES = [
  { code: "fr", label: "Français" },
  { code: "en", label: "English" },
  { code: "es", label: "Español" },
  { code: "de", label: "Deutsch" },
  { code: "it", label: "Italiano" },
  { code: "auto", label: "Auto-détection" },
];

const PRESETS = [
  { label: "OpenAI", apiUrl: "https://api.openai.com/v1/audio/transcriptions", apiModel: "whisper-1" },
  { label: "Groq", apiUrl: "https://api.groq.com/openai/v1/audio/transcriptions", apiModel: "whisper-large-v3-turbo" },
];

const formatSize = (bytes: number | null) =>
  bytes === null ? "" : bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} Go` : `${Math.round(bytes / 1e6)} Mo`;

interface Progress {
  model: WhisperModel;
  received: number;
  total: number | null;
}

/** Click, then press a combo. Esc cancels. Modifier-only presses keep listening. */
function ShortcutRecorder({ value, onChange }: { value: string; onChange: (shortcut: string) => void }) {
  const [recording, setRecording] = useState(false);

  function onKeyDown(e: KeyboardEvent<HTMLButtonElement>) {
    if (!recording) return;
    e.preventDefault(); // also keeps Esc from closing the dialog
    e.stopPropagation();
    if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) return setRecording(false);
    const shortcut = toTauriShortcut(e);
    if (!shortcut) return;
    setRecording(false);
    onChange(shortcut);
  }

  return (
    <button
      id="set-voice-shortcut"
      type="button"
      data-shortcut-recorder
      onClick={() => setRecording(true)}
      onBlur={() => setRecording(false)}
      onKeyDown={onKeyDown}
      className={`${field} inline-flex w-56 items-center justify-center font-mono text-xs ${recording ? "border-accent text-muted" : ""}`}
    >
      {recording ? "Appuyez sur une combinaison… (Échap)" : formatShortcut(value)}
    </button>
  );
}

function ModelList({ config, onSelect }: { config: VoiceConfig; onSelect: (model: WhisperModel) => void }) {
  const [models, setModels] = useState<ModelStatus[] | null>(null);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    listWhisperModels().then(setModels, (err) => setError(`Liste des modèles indisponible : ${errorMessage(err)}`));
  }, []);

  useEffect(() => refresh(), [refresh]);

  async function download(model: WhisperModel) {
    setError(null);
    setProgress({ model, received: 0, total: null });
    let unlisten: UnlistenFn | undefined;
    try {
      unlisten = await listen<{ received: number; total: number | null }>("voice-download", (e) =>
        setProgress({ model, ...e.payload }),
      );
      await downloadWhisperModel(model);
    } catch (err) {
      setError(`Téléchargement impossible : ${errorMessage(err)}`);
    } finally {
      unlisten?.();
      setProgress(null);
      refresh();
    }
  }

  if (!models) return error ? <ErrorText>{error}</ErrorText> : <p className="py-3 text-xs text-subtle">Chargement…</p>;
  return (
    <fieldset className="flex flex-col gap-1.5 py-3">
      <legend className="mb-2 text-fg">Modèle Whisper</legend>
      {models.map((m) => {
        const busy = progress?.model === m.model ? progress : null;
        const pct = busy?.total ? Math.round((busy.received / busy.total) * 100) : null;
        return (
          <div
            key={m.model}
            className={`flex items-center gap-3 rounded-lg border px-3 py-2 ${
              config.model === m.model ? "border-line-strong bg-raised" : "border-line"
            }`}
          >
            <label className={`flex flex-1 items-center gap-3 ${m.downloaded ? "cursor-pointer" : "opacity-70"}`}>
              <input
                type="radio"
                name="whisper-model"
                checked={config.model === m.model}
                disabled={!m.downloaded}
                onChange={() => onSelect(m.model)}
                className="size-4 accent-[var(--color-accent)]"
              />
              <span className="font-mono text-xs text-fg">{m.model}</span>
              <span className="text-xs text-subtle">{formatSize(m.sizeBytes)}</span>
            </label>
            {m.downloaded ? (
              <span className="inline-flex items-center gap-1 text-xs text-ok">
                <Check size={12} strokeWidth={1.5} aria-hidden="true" />
                Téléchargé
              </span>
            ) : busy ? (
              <div className="flex w-40 items-center gap-2">
                <progress
                  value={pct ?? undefined}
                  max={100}
                  aria-label={`Téléchargement de ${m.model}`}
                  className="h-1.5 flex-1 overflow-hidden rounded-full [&::-webkit-progress-bar]:bg-line [&::-webkit-progress-value]:bg-accent"
                />
                <span className="w-9 text-right text-xs text-muted tabular-nums">{pct === null ? "…" : `${pct}%`}</span>
              </div>
            ) : (
              <button
                type="button"
                disabled={progress !== null}
                onClick={() => void download(m.model)}
                className={button}
              >
                <Download size={14} strokeWidth={1.5} aria-hidden="true" />
                Télécharger
              </button>
            )}
          </div>
        );
      })}
      {error && <ErrorText>{error}</ErrorText>}
    </fieldset>
  );
}

function ApiSettings({
  config,
  update,
  reload,
  save,
}: {
  config: VoiceConfig;
  update: (patch: Partial<VoiceConfig>) => void;
  reload: () => void;
  save: Save;
}) {
  const [apiUrl, setApiUrl] = useState(config.apiUrl);
  const [apiModel, setApiModel] = useState(config.apiModel);
  const [key, setKey] = useState("");

  // Text fields commit on blur/Enter: the backend validates the URL, so half-typed values must not be sent.
  const commit = () => {
    if (apiUrl !== config.apiUrl || apiModel !== config.apiModel) update({ apiUrl, apiModel });
  };
  const onEnter = (e: KeyboardEvent<HTMLInputElement>) => e.key === "Enter" && commit();
  const storeKey = (value: string | null) => {
    const task = setVoiceApiKey(value);
    save(task);
    task.then(
      () => {
        setKey("");
        reload();
      },
      () => undefined,
    );
  };

  return (
    <>
      <Row label="Préréglages">
        {PRESETS.map((p) => (
          <button
            key={p.label}
            type="button"
            onClick={() => {
              setApiUrl(p.apiUrl);
              setApiModel(p.apiModel);
              update({ apiUrl: p.apiUrl, apiModel: p.apiModel });
            }}
            className={button}
          >
            {p.label}
          </button>
        ))}
      </Row>
      <Row label="URL" htmlFor="set-api-url">
        <input
          id="set-api-url"
          value={apiUrl}
          onChange={(e) => setApiUrl(e.target.value)}
          onBlur={commit}
          onKeyDown={onEnter}
          className={`${field} w-80 font-mono text-xs`}
        />
      </Row>
      <Row label="Modèle" htmlFor="set-api-model">
        <input
          id="set-api-model"
          value={apiModel}
          onChange={(e) => setApiModel(e.target.value)}
          onBlur={commit}
          onKeyDown={onEnter}
          className={`${field} w-80 font-mono text-xs`}
        />
      </Row>
      <Row label="Clé API" hint={config.hasApiKey ? "Clé enregistrée" : "Aucune clé enregistrée"} htmlFor="set-api-key">
        <form
          className="flex items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (key.trim()) storeKey(key.trim());
          }}
        >
          <input
            id="set-api-key"
            type="password"
            autoComplete="off"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            placeholder={config.hasApiKey ? "••••••••" : "sk-…"}
            className={`${field} w-48 font-mono text-xs`}
          />
          <button type="submit" disabled={!key.trim()} className={button}>
            <KeyRound size={14} strokeWidth={1.5} aria-hidden="true" />
            Enregistrer
          </button>
          <button type="button" disabled={!config.hasApiKey} onClick={() => storeKey(null)} className={button}>
            <Trash2 size={14} strokeWidth={1.5} aria-hidden="true" />
            Supprimer
          </button>
        </form>
      </Row>
    </>
  );
}

/** Voice settings live in the Rust backend: every change goes through voice_set_config. */
export function VoiceSection({ save }: { save: Save }) {
  const [config, setConfig] = useState<VoiceConfig | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    getVoiceConfig().then(setConfig, (err) => setError(`Réglages voix indisponibles : ${errorMessage(err)}`));
  }, []);

  useEffect(() => reload(), [reload]);

  if (!config) return error ? <ErrorText>{error}</ErrorText> : <p className="text-xs text-subtle">Chargement…</p>;

  // Optimistic: apply now; on failure the backend's real config is reloaded and the error shown by `save`.
  const update = (patch: Partial<VoiceConfig>) => {
    const { hasApiKey: _, ...next } = { ...config, ...patch };
    setConfig({ ...config, ...patch });
    const task = setVoiceConfig(next);
    save(task);
    task.then(setConfig, reload);
  };

  return (
    <Section title="Voix">
      <Row label="Raccourci" hint="Global, actif même hors de la fenêtre" htmlFor="set-voice-shortcut">
        <ShortcutRecorder value={config.shortcut} onChange={(shortcut) => update({ shortcut })} />
      </Row>
      <Row label="Mode" htmlFor="set-voice-mode">
        <select
          id="set-voice-mode"
          value={config.pushToTalk ? "hold" : "toggle"}
          onChange={(e) => update({ pushToTalk: e.target.value === "hold" })}
          className={`${field} w-56`}
        >
          <option value="hold">Maintenir pour parler</option>
          <option value="toggle">Appuyer pour démarrer/arrêter</option>
        </select>
      </Row>
      <Row label="Envoyer automatiquement" hint="Appuie sur Entrée après la transcription" htmlFor="set-voice-submit">
        <Toggle id="set-voice-submit" checked={config.submit} onChange={(submit) => update({ submit })} />
      </Row>
      <Row label="Langue" htmlFor="set-voice-lang">
        <select
          id="set-voice-lang"
          value={config.language}
          onChange={(e) => update({ language: e.target.value })}
          className={`${field} w-56`}
        >
          {LANGUAGES.map((l) => (
            <option key={l.code} value={l.code}>
              {l.label}
            </option>
          ))}
        </select>
      </Row>
      <Row label="Moteur" htmlFor="set-voice-engine">
        <select
          id="set-voice-engine"
          value={config.engine}
          onChange={(e) => update({ engine: e.target.value as VoiceConfig["engine"] })}
          className={`${field} w-56`}
        >
          <option value="local">Local — Whisper hors ligne</option>
          <option value="api">API</option>
        </select>
      </Row>
      {config.engine === "local" ? (
        <ModelList config={config} onSelect={(model) => update({ model })} />
      ) : (
        <ApiSettings config={config} update={update} reload={reload} save={save} />
      )}
      {error && <ErrorText>{error}</ErrorText>}
    </Section>
  );
}
