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
  LayoutTemplate,
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
import { HistorySettings } from "../History/HistorySettings";
import { IconButton } from "../IconButton/IconButton";
import { TemplatesSection } from "../Templates/TemplatesSection";
import { AutostartRow, UpdateRow } from "./AppControls";
import { button, ErrorText, errorMessage, field, Row, Section, Toggle } from "./controls";
import { SpeechRows } from "./SpeechRows";
import { VoiceSection } from "./VoiceSection";

/** Runs a save and reports it in the dialog header ("Saved" or the error). */
export type Save = (task: Promise<unknown>) => void;

type SectionId = "general" | "notifications" | "voice" | "agents" | "templates" | "about";

const SECTIONS: { id: SectionId; label: string; icon: LucideIcon }[] = [
  { id: "general", label: "General", icon: SlidersHorizontal },
  { id: "notifications", label: "Notifications", icon: Bell },
  { id: "voice", label: "Voice", icon: Mic },
  { id: "agents", label: "Agents", icon: Bot },
  { id: "templates", label: "Templates", icon: LayoutTemplate },
  { id: "about", label: "About", icon: Info },
];

function General({ s, save }: { s: Settings; save: Save }) {
  return (
    <Section title="General">
      <Row label="Terminal font size" hint="For new agents" htmlFor="set-font-size">
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
      <Row label="Terminal font" hint="For new agents" htmlFor="set-font-family">
        <input
          id="set-font-family"
          value={s.fontFamily}
          placeholder={DEFAULT_FONT_FAMILY}
          onChange={(e) => save(updateSettings({ fontFamily: e.target.value }))}
          className={`${field} w-64 font-mono text-xs`}
        />
      </Row>
      <Row label='Confirm "Stop all" and "Restart all"' htmlFor="set-confirm">
        <Toggle id="set-confirm" checked={s.confirmBulk} onChange={(v) => save(updateSettings({ confirmBulk: v }))} />
      </Row>
      <Row label="Show workspaces on startup" htmlFor="set-sidebar">
        <Toggle id="set-sidebar" checked={s.sidebarOpen} onChange={(v) => save(updateSettings({ sidebarOpen: v }))} />
      </Row>
      <Row label="Show activity panel on startup" htmlFor="set-activity">
        <Toggle
          id="set-activity"
          checked={s.activityOpen}
          onChange={(v) => save(updateSettings({ activityOpen: v }))}
        />
      </Row>
      <Row label="Auto-arrange panes in a grid" hint="When a pane is added or removed" htmlFor="set-auto-grid">
        <Toggle id="set-auto-grid" checked={s.autoGrid} onChange={(v) => save(updateSettings({ autoGrid: v }))} />
      </Row>
      <AutostartRow save={save} />
      <HistorySettings save={save} />
      <Row
        label="Close to system tray"
        hint="Closing the window hides it; Quit from the tray icon stops everything"
        htmlFor="set-close-tray"
      >
        <Toggle
          id="set-close-tray"
          checked={s.closeToTray}
          onChange={(v) => save(updateSettings({ closeToTray: v }))}
        />
      </Row>
    </Section>
  );
}

const EVENTS: { key: "done" | "waiting" | "error"; label: string }[] = [
  { key: "done", label: "Done" },
  { key: "waiting", label: "Waiting" },
  { key: "error", label: "Error" },
];

function Notifications({ s, save }: { s: Settings; save: Save }) {
  const set = (patch: Partial<Settings["notify"]>) => save(updateSettings({ notify: { ...s.notify, ...patch } }));
  return (
    <Section title="Notifications">
      <Row label="System notifications" htmlFor="set-notify">
        <Toggle id="set-notify" checked={s.notify.enabled} onChange={(v) => set({ enabled: v })} />
      </Row>
      <Row label="Sound" htmlFor="set-sound">
        <Toggle id="set-sound" checked={s.notify.sound} onChange={(v) => set({ sound: v })} />
      </Row>
      <fieldset className="flex items-center justify-between gap-6 py-3">
        <legend className="sr-only">Notified events</legend>
        <span className="text-fg" aria-hidden="true">
          Events
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
        label="Only when I'm not looking"
        hint="Window in the background, or the agent is not the active pane"
        htmlFor="set-unseen"
      >
        <Toggle id="set-unseen" checked={s.notify.onlyUnseen} onChange={(v) => set({ onlyUnseen: v })} />
      </Row>
      <SpeechRows s={s} save={save} />
    </Section>
  );
}

function Agents({ s, save }: { s: Settings; save: Save }) {
  return (
    <Section title="Agents">
      <Row
        label="Trust agent folders automatically"
        hint="Skips each agent's folder trust prompt for folders you open in AgentDeck. Only enable for code you trust: project hooks and MCP servers will run without asking."
        htmlFor="set-trust-folders"
      >
        <Toggle
          id="set-trust-folders"
          checked={s.trustAgentFolders}
          onChange={(v) => save(updateSettings({ trustAgentFolders: v }))}
        />
      </Row>
      <p className="pb-2 text-xs text-subtle">Values prefilled in "Add agent".</p>
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
              aria-label={`Default arguments for ${AGENTS[kind].name}`}
              value={defaults.args}
              placeholder="Default arguments"
              onChange={(e) => set({ args: e.target.value })}
              className={`${field} flex-1 font-mono text-xs`}
            />
            <span className="flex shrink-0 items-center gap-2 text-xs text-muted">
              <Toggle
                checked={defaults.worktree}
                onChange={(v) => set({ worktree: v })}
                label={`Default Git worktree for ${AGENTS[kind].name}`}
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
    <Section title="About">
      <Row label="Version">
        <span className="font-mono text-xs text-muted">{version ?? "…"}</span>
      </Row>
      <UpdateRow />
      <Row label="Data folder" hint={dataDir ?? "…"}>
        <button
          type="button"
          disabled={!dataDir}
          onClick={() => dataDir && revealItemInDir(dataDir).catch(fail)}
          className={button}
        >
          <FolderOpen size={14} strokeWidth={1.5} aria-hidden="true" />
          Open
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

/** "Settings": full-height modal, sections on the left. Every change applies and saves immediately. */
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
        setError(`Could not save: ${errorMessage(err)}`);
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
        <nav aria-label="Settings sections" className="flex w-48 shrink-0 flex-col gap-0.5 border-r border-line p-3">
          <h2 id="settings-title" className="px-2 pt-1 pb-3 text-[15px] font-medium">
            Settings
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
              {saved ? "Saved" : ""}
            </span>
            <IconButton icon={X} label="Close" hint="Close (Esc)" onClick={onClose} />
          </div>
          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-6 pb-6">
            {error && <ErrorText>{error}</ErrorText>}
            {section === "general" && <General s={settings} save={save} />}
            {section === "notifications" && <Notifications s={settings} save={save} />}
            {section === "voice" && <VoiceSection save={save} />}
            {section === "agents" && <Agents s={settings} save={save} />}
            {section === "templates" && <TemplatesSection save={save} />}
            {section === "about" && <About />}
          </div>
        </div>
      </div>
    </dialog>
  );
}
