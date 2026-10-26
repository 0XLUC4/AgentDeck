import { getVersion } from "@tauri-apps/api/app";
import { appDataDir } from "@tauri-apps/api/path";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  Bell,
  Bot,
  Check,
  ExternalLink,
  FolderOpen,
  Info,
  type LucideIcon,
  Mic,
  SlidersHorizontal,
  X,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { AGENT_KINDS, AGENTS } from "../../lib/agents";
import { FONT_SIZE } from "../../store/deck";
import { DEFAULT_FONT_FAMILY, type Settings, updateSettings, useSettings } from "../../store/settings";
import { AgentLogo } from "../AgentLogo/AgentLogo";
import { IconButton } from "../IconButton/IconButton";
import { button, ErrorText, errorMessage, field, Row, Section, Toggle } from "./controls";
import { VoiceSection } from "./VoiceSection";

/** Runs a save and reports it in the dialog header ("Enregistré" or the error). */
export type Save = (task: Promise<unknown>) => void;

type SectionId = "general" | "notifications" | "voice" | "agents" | "about";

const SECTIONS: { id: SectionId; label: string; icon: LucideIcon }[] = [
  { id: "general", label: "Général", icon: SlidersHorizontal },
  { id: "notifications", label: "Notifications", icon: Bell },
  { id: "voice", label: "Voix", icon: Mic },
  { id: "agents", label: "Agents", icon: Bot },
  { id: "about", label: "À propos", icon: Info },
];

function General({ s, save }: { s: Settings; save: Save }) {
  return (
    <Section title="Général">
      <Row label="Taille de police du terminal" hint="Pour les nouveaux agents" htmlFor="set-font-size">
        <input
          id="set-font-size"
          type="range"
          min={FONT_SIZE.min}
          max={FONT_SIZE.max}
          value={s.fontSize}
          onChange={(e) => save(updateSettings({ fontSize: e.target.valueAsNumber }))}
          className="w-36 accent-[var(--color-accent)]"
        />
        <span className="w-10 text-right font-mono text-xs text-muted tabular-nums">{s.fontSize} px</span>
      </Row>
      <Row label="Police du terminal" hint="Pour les nouveaux agents" htmlFor="set-font-family">
        <input
          id="set-font-family"
          value={s.fontFamily}
          placeholder={DEFAULT_FONT_FAMILY}
          onChange={(e) => save(updateSettings({ fontFamily: e.target.value }))}
          className={`${field} w-64 font-mono text-xs`}
        />
      </Row>
      <Row label="Confirmer « Tout arrêter » et « Tout relancer »" htmlFor="set-confirm">
        <Toggle id="set-confirm" checked={s.confirmBulk} onChange={(v) => save(updateSettings({ confirmBulk: v }))} />
      </Row>
      <Row label="Workspaces affichés au démarrage" htmlFor="set-sidebar">
        <Toggle id="set-sidebar" checked={s.sidebarOpen} onChange={(v) => save(updateSettings({ sidebarOpen: v }))} />
      </Row>
      <Row label="Panneau d'activité affiché au démarrage" htmlFor="set-activity">
        <Toggle
          id="set-activity"
          checked={s.activityOpen}
          onChange={(v) => save(updateSettings({ activityOpen: v }))}
        />
      </Row>
    </Section>
  );
}

const EVENTS: { key: "done" | "waiting" | "error"; label: string }[] = [
  { key: "done", label: "Terminé" },
  { key: "waiting", label: "En attente" },
  { key: "error", label: "Erreur" },
];

function Notifications({ s, save }: { s: Settings; save: Save }) {
  const set = (patch: Partial<Settings["notify"]>) => save(updateSettings({ notify: { ...s.notify, ...patch } }));
  return (
    <Section title="Notifications">
      <Row label="Notifications système" htmlFor="set-notify">
        <Toggle id="set-notify" checked={s.notify.enabled} onChange={(v) => set({ enabled: v })} />
      </Row>
      <Row label="Son" htmlFor="set-sound">
        <Toggle id="set-sound" checked={s.notify.sound} onChange={(v) => set({ sound: v })} />
      </Row>
      <fieldset className="flex items-center justify-between gap-6 py-3">
        <legend className="sr-only">Événements notifiés</legend>
        <span className="text-fg" aria-hidden="true">
          Événements
        </span>
        <div className="flex gap-4">
          {EVENTS.map(({ key, label }) => (
            <label key={key} className="flex cursor-pointer items-center gap-2 text-muted hover:text-fg">
              <input
                type="checkbox"
                checked={s.notify[key]}
                onChange={(e) => set({ [key]: e.target.checked })}
                className="size-4 accent-[var(--color-accent)]"
              />
              {label}
            </label>
          ))}
        </div>
      </fieldset>
      <Row
        label="Seulement si je ne regarde pas"
        hint="Fenêtre en arrière-plan ou agent qui n'est pas le panneau actif"
        htmlFor="set-unseen"
      >
        <Toggle id="set-unseen" checked={s.notify.onlyUnseen} onChange={(v) => set({ onlyUnseen: v })} />
      </Row>
    </Section>
  );
}

function Agents({ s, save }: { s: Settings; save: Save }) {
  return (
    <Section title="Agents">
      <p className="pb-2 text-xs text-subtle">Valeurs pré-remplies dans « Ajouter un agent ».</p>
      {AGENT_KINDS.map((kind) => {
        const defaults = s.agents[kind];
        const set = (patch: Partial<typeof defaults>) =>
          save(updateSettings({ agents: { ...s.agents, [kind]: { ...defaults, ...patch } } }));
        return (
          <div key={kind} className="flex items-center gap-3 py-3">
            <span className="flex w-36 shrink-0 items-center gap-2.5">
              <AgentLogo kind={kind} size={16} />
              <span className="truncate text-fg">{AGENTS[kind].name}</span>
            </span>
            <input
              aria-label={`Arguments par défaut pour ${AGENTS[kind].name}`}
              value={defaults.args}
              placeholder="Arguments par défaut"
              onChange={(e) => set({ args: e.target.value })}
              className={`${field} flex-1 font-mono text-xs`}
            />
            <span className="flex shrink-0 items-center gap-2 text-xs text-muted">
              <Toggle
                checked={defaults.worktree}
                onChange={(v) => set({ worktree: v })}
                label={`Worktree Git par défaut pour ${AGENTS[kind].name}`}
              />
              Worktree
            </span>
          </div>
        );
      })}
    </Section>
  );
}

const LINKS = [
  { label: "Claude Code", url: "https://docs.anthropic.com/en/docs/claude-code" },
  { label: "Codex CLI", url: "https://github.com/openai/codex" },
  { label: "Gemini CLI", url: "https://github.com/google-gemini/gemini-cli" },
  { label: "Tauri", url: "https://v2.tauri.app" },
];

function About() {
  const [version, setVersion] = useState<string | null>(null);
  const [dataDir, setDataDir] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getVersion().then(setVersion, (err) => setError(errorMessage(err)));
    appDataDir().then(setDataDir, (err) => setError(errorMessage(err)));
  }, []);

  const fail = (err: unknown) => setError(errorMessage(err));
  return (
    <Section title="À propos">
      <Row label="Version">
        <span className="font-mono text-xs text-muted">{version ?? "…"}</span>
      </Row>
      <Row label="Dossier des données" hint={dataDir ?? "…"}>
        <button
          type="button"
          disabled={!dataDir}
          onClick={() => dataDir && revealItemInDir(dataDir).catch(fail)}
          className={button}
        >
          <FolderOpen size={14} strokeWidth={1.5} aria-hidden="true" />
          Ouvrir
        </button>
      </Row>
      <div className="flex flex-wrap gap-2 py-3">
        {LINKS.map(({ label, url }) => (
          <button key={url} type="button" onClick={() => openUrl(url).catch(fail)} className={button}>
            {label}
            <ExternalLink size={13} strokeWidth={1.5} aria-hidden="true" />
          </button>
        ))}
      </div>
      {error && <ErrorText>{error}</ErrorText>}
    </Section>
  );
}

/** "Réglages": full-height modal, sections on the left. Every change applies and saves immediately. */
export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const settings = useSettings();
  const [section, setSection] = useState<SectionId>("general");
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => {
    dialogRef.current?.showModal();
    return () => window.clearTimeout(timer.current);
  }, []);

  const save: Save = useCallback((task) => {
    task.then(
      () => {
        setError(null);
        setSaved(true);
        window.clearTimeout(timer.current);
        timer.current = window.setTimeout(() => setSaved(false), 1500);
      },
      (err) => {
        setSaved(false);
        setError(`Enregistrement impossible : ${errorMessage(err)}`);
      },
    );
  }, []);

  return (
    <dialog
      ref={dialogRef}
      onClose={onClose}
      aria-labelledby="settings-title"
      className="m-auto h-[calc(100vh-3rem)] max-h-[48rem] w-[min(56rem,94vw)] overflow-hidden rounded-xl border border-line bg-panel p-0 text-fg backdrop:bg-gutter/70"
    >
      <div className="flex h-full">
        <nav
          aria-label="Sections des réglages"
          className="flex w-48 shrink-0 flex-col gap-0.5 border-r border-line p-3"
        >
          <h2 id="settings-title" className="px-2 pt-1 pb-3 text-[15px] font-medium">
            Réglages
          </h2>
          {SECTIONS.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              type="button"
              aria-current={section === id ? "page" : undefined}
              onClick={() => setSection(id)}
              className="flex h-8 items-center gap-2.5 rounded-md px-2 text-left text-muted transition-colors duration-150 hover:bg-hover hover:text-fg aria-[current=page]:bg-raised aria-[current=page]:text-fg"
            >
              <Icon size={15} strokeWidth={1.5} aria-hidden="true" />
              {label}
            </button>
          ))}
        </nav>

        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex h-12 shrink-0 items-center justify-end gap-3 pr-3 pl-6">
            <span
              role="status"
              className={`inline-flex items-center gap-1 text-xs text-subtle transition-opacity duration-300 ${
                saved ? "opacity-100" : "opacity-0"
              }`}
            >
              <Check size={12} strokeWidth={1.5} aria-hidden="true" />
              {saved ? "Enregistré" : ""}
            </span>
            <IconButton icon={X} label="Fermer" hint="Fermer (Échap)" onClick={onClose} />
          </div>
          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-6 pb-6">
            {error && <ErrorText>{error}</ErrorText>}
            {section === "general" && <General s={settings} save={save} />}
            {section === "notifications" && <Notifications s={settings} save={save} />}
            {section === "voice" && <VoiceSection save={save} />}
            {section === "agents" && <Agents s={settings} save={save} />}
            {section === "about" && <About />}
          </div>
        </div>
      </div>
    </dialog>
  );
}
