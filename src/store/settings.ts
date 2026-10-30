// Frontend preferences (general, notifications, agent defaults), saved to settings.json. Voice config lives in Rust.
import { load, type Store } from "@tauri-apps/plugin-store";
import { create } from "zustand";
import { AGENT_KINDS } from "../lib/agents";
import type { AgentKind } from "../lib/types";
import { clampFontSize, FONT_SIZE } from "./deck";

export interface AgentDefaults {
  args: string;
  worktree: boolean;
}

export interface NotifySettings {
  enabled: boolean;
  sound: boolean;
  done: boolean;
  waiting: boolean;
  error: boolean;
  /** Only when the window is unfocused or the pane isn't the active one. */
  onlyUnseen: boolean;
}

/** Spoken summary when a task finishes (Web Speech API). `voice` is a voiceURI, "" = first voice of the language. */
export interface SpeechSettings {
  enabled: boolean;
  voice: string;
  rate: number;
}

/** Permission auto-approved by AgentDeck. `rule`: "Read", "Bash(pnpm test)" or "Bash(pnpm test *)". */
export interface AutoAllowRule {
  rule: string;
  /** Folder the rule applies to (and its subfolders); "" = every folder. */
  folder: string;
}

/** Cost limits in USD; 0 = off. Claude Code and Codex only (the agents with transcripts). */
export interface BudgetSettings {
  dailyUsd: number;
  paneUsd: number;
  /** Interrupt the working agents and pause the orchestrator once a limit is hit. */
  interrupt: boolean;
}

export interface Settings {
  fontSize: number;
  fontFamily: string;
  confirmBulk: boolean;
  sidebarOpen: boolean;
  activityOpen: boolean;
  /** Closing the main window hides it in the system tray instead of quitting. */
  closeToTray: boolean;
  /** Re-arranges a session's panes in a grid whenever one is added or removed. */
  autoGrid: boolean;
  notify: NotifySettings;
  agents: Record<AgentKind, AgentDefaults>;
  /** Pre-accept each agent's "trust this folder" prompt for folders opened in AgentDeck. */
  trustAgentFolders: boolean;
  /** Claude's internal subagents shown as read-only panes next to their parent (components/Subagents). */
  showSubagents: boolean;
  speech: SpeechSettings;
  autoAllow: AutoAllowRule[];
  budget: BudgetSettings;
  /** Webhook (ntfy topic URL or similar) also receiving the notifications; "" = off. */
  phoneUrl: string;
}

export const DEFAULT_FONT_FAMILY = "Cascadia Code, Consolas, monospace";

export const DEFAULT_SETTINGS: Settings = {
  fontSize: FONT_SIZE.default,
  fontFamily: DEFAULT_FONT_FAMILY,
  confirmBulk: true,
  sidebarOpen: true,
  activityOpen: false,
  closeToTray: false,
  autoGrid: true,
  notify: { enabled: true, sound: true, done: true, waiting: true, error: false, onlyUnseen: true },
  speech: { enabled: false, voice: "", rate: 1 },
  trustAgentFolders: true,
  showSubagents: true,
  autoAllow: [],
  budget: { dailyUsd: 0, paneUsd: 0, interrupt: false },
  phoneUrl: "",
  agents: Object.fromEntries(AGENT_KINDS.map((k) => [k, { args: "", worktree: false }])) as Record<
    AgentKind,
    AgentDefaults
  >,
};

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;
const bool = (v: unknown, fallback: boolean) => (typeof v === "boolean" ? v : fallback);
const usd = (v: unknown) => (typeof v === "number" && Number.isFinite(v) && v > 0 ? v : 0);

/** Builds valid settings from anything read from disk: unknown or invalid fields fall back to defaults. */
export function mergeSettings(raw: unknown): Settings {
  const d = DEFAULT_SETTINGS;
  if (!isObject(raw)) return structuredClone(d);
  const n = isObject(raw.notify) ? raw.notify : {};
  const agents = isObject(raw.agents) ? raw.agents : {};
  const sp = isObject(raw.speech) ? raw.speech : {};
  const b = isObject(raw.budget) ? raw.budget : {};
  const fontFamily = typeof raw.fontFamily === "string" ? raw.fontFamily.trim() : "";
  return {
    fontSize: typeof raw.fontSize === "number" ? clampFontSize(raw.fontSize) : d.fontSize,
    fontFamily: fontFamily || d.fontFamily,
    confirmBulk: bool(raw.confirmBulk, d.confirmBulk),
    sidebarOpen: bool(raw.sidebarOpen, d.sidebarOpen),
    activityOpen: bool(raw.activityOpen, d.activityOpen),
    closeToTray: bool(raw.closeToTray, d.closeToTray),
    autoGrid: bool(raw.autoGrid, d.autoGrid),
    trustAgentFolders: bool(raw.trustAgentFolders, d.trustAgentFolders),
    showSubagents: bool(raw.showSubagents, d.showSubagents),
    notify: {
      enabled: bool(n.enabled, d.notify.enabled),
      sound: bool(n.sound, d.notify.sound),
      done: bool(n.done, d.notify.done),
      waiting: bool(n.waiting, d.notify.waiting),
      error: bool(n.error, d.notify.error),
      onlyUnseen: bool(n.onlyUnseen, d.notify.onlyUnseen),
    },
    speech: {
      enabled: bool(sp.enabled, d.speech.enabled),
      voice: typeof sp.voice === "string" ? sp.voice : d.speech.voice,
      rate: typeof sp.rate === "number" && sp.rate >= 0.5 && sp.rate <= 2 ? sp.rate : d.speech.rate,
    },
    autoAllow: (Array.isArray(raw.autoAllow) ? raw.autoAllow : []).flatMap((r) =>
      isObject(r) && typeof r.rule === "string" && r.rule.trim()
        ? [{ rule: r.rule.trim(), folder: typeof r.folder === "string" ? r.folder : "" }]
        : [],
    ),
    budget: { dailyUsd: usd(b.dailyUsd), paneUsd: usd(b.paneUsd), interrupt: bool(b.interrupt, d.budget.interrupt) },
    phoneUrl: typeof raw.phoneUrl === "string" ? raw.phoneUrl.trim() : "",
    agents: Object.fromEntries(
      AGENT_KINDS.map((k) => {
        const a = isObject(agents[k]) ? agents[k] : {};
        return [k, { args: typeof a.args === "string" ? a.args : "", worktree: bool(a.worktree, false) }];
      }),
    ) as Record<AgentKind, AgentDefaults>,
  };
}

const KEY = "settings";
let store: Store | undefined;

export const useSettings = create<Settings>(() => structuredClone(DEFAULT_SETTINGS));

/** Loads settings.json once at startup. On failure the defaults stay in effect. */
export const settingsReady: Promise<void> = (async () => {
  try {
    store = await load("settings.json", { autoSave: false });
    useSettings.setState(mergeSettings(await store.get(KEY)));
  } catch (err) {
    console.warn("settings load failed", err);
  }
})();

/** Applies a change immediately and writes it to disk. Rejects if the write fails (state stays applied). */
export async function updateSettings(patch: Partial<Settings>): Promise<void> {
  useSettings.setState(patch);
  if (!store) throw new Error("settings storage unavailable");
  await store.set(KEY, useSettings.getState());
  await store.save();
}
