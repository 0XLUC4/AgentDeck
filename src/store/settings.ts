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

export interface Settings {
  fontSize: number;
  fontFamily: string;
  confirmBulk: boolean;
  sidebarOpen: boolean;
  activityOpen: boolean;
  notify: NotifySettings;
  agents: Record<AgentKind, AgentDefaults>;
}

export const DEFAULT_FONT_FAMILY = "Cascadia Code, Consolas, monospace";

export const DEFAULT_SETTINGS: Settings = {
  fontSize: FONT_SIZE.default,
  fontFamily: DEFAULT_FONT_FAMILY,
  confirmBulk: true,
  sidebarOpen: true,
  activityOpen: false,
  notify: { enabled: true, sound: true, done: true, waiting: true, error: false, onlyUnseen: true },
  agents: Object.fromEntries(AGENT_KINDS.map((k) => [k, { args: "", worktree: false }])) as Record<
    AgentKind,
    AgentDefaults
  >,
};

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;
const bool = (v: unknown, fallback: boolean) => (typeof v === "boolean" ? v : fallback);

/** Builds valid settings from anything read from disk: unknown or invalid fields fall back to defaults. */
export function mergeSettings(raw: unknown): Settings {
  const d = DEFAULT_SETTINGS;
  if (!isObject(raw)) return structuredClone(d);
  const n = isObject(raw.notify) ? raw.notify : {};
  const agents = isObject(raw.agents) ? raw.agents : {};
  const fontFamily = typeof raw.fontFamily === "string" ? raw.fontFamily.trim() : "";
  return {
    fontSize: typeof raw.fontSize === "number" ? clampFontSize(raw.fontSize) : d.fontSize,
    fontFamily: fontFamily || d.fontFamily,
    confirmBulk: bool(raw.confirmBulk, d.confirmBulk),
    sidebarOpen: bool(raw.sidebarOpen, d.sidebarOpen),
    activityOpen: bool(raw.activityOpen, d.activityOpen),
    notify: {
      enabled: bool(n.enabled, d.notify.enabled),
      sound: bool(n.sound, d.notify.sound),
      done: bool(n.done, d.notify.done),
      waiting: bool(n.waiting, d.notify.waiting),
      error: bool(n.error, d.notify.error),
      onlyUnseen: bool(n.onlyUnseen, d.notify.onlyUnseen),
    },
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
  if (!store) throw new Error("stockage des réglages indisponible");
  await store.set(KEY, useSettings.getState());
  await store.save();
}
