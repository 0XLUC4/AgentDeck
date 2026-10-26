// Session templates: agents, titles, args, layout and font sizes of a session, reusable in any workspace.
// Folders are stored relative to the workspace; a folder outside it is dropped (the new pane uses the workspace).

import { load, type Store } from "@tauri-apps/plugin-store";
import type { SerializedDockview } from "dockview-react";
import { create } from "zustand";
import { splitArgs } from "../lib/splitArgs";
import type { AgentKind } from "../lib/types";
import { activeWorkspace, deck, newTab, type Pane, type PaneSpec, type Tab, useDeck } from "./deck";
import { createPane } from "./runtime";
import { useSettings } from "./settings";

export interface TemplatePane {
  /** Pane id inside `layout`; replaced by a fresh id on each use. */
  key: string;
  agent: AgentKind;
  title: string;
  /** Relative to the workspace folder ("" = the folder itself); null = the workspace folder / last used folder. */
  cwd: string | null;
  /** Undefined: the agent's defaults from Settings > Agents. */
  args?: string[];
  worktree?: boolean;
  fontSize?: number;
}

export interface SessionTemplate {
  id: string;
  name: string;
  builtin?: boolean;
  /** Null: panes side by side. */
  layout: SerializedDockview | null;
  panes: TemplatePane[];
}

const pane = (key: string, agent: AgentKind, title: string): TemplatePane => ({ key, agent, title, cwd: null });

export const BUILTIN_TEMPLATES: SessionTemplate[] = [
  {
    id: "builtin:front-back-tests",
    name: "Front + Back + Tests",
    builtin: true,
    layout: null,
    panes: [pane("front", "claude", "Front"), pane("back", "claude", "Back"), pane("tests", "claude", "Tests")],
  },
  {
    id: "builtin:claude-vs-codex",
    name: "Claude vs Codex",
    builtin: true,
    layout: null,
    panes: [pane("claude", "claude", "Claude"), pane("codex", "codex", "Codex")],
  },
  {
    id: "builtin:solo-claude",
    name: "Solo Claude",
    builtin: true,
    layout: null,
    panes: [pane("solo", "claude", "Claude")],
  },
];

const comparable = (path: string) =>
  path
    .replace(/[\\/]+/g, "/")
    .replace(/\/$/, "")
    .toLowerCase();

/** `path` relative to `root` with "/" separators ("" for root itself), or null when outside (or no root). */
export function relativePath(path: string, root: string): string | null {
  if (!root || !path) return null;
  const p = comparable(path);
  const r = comparable(root);
  if (p === r) return "";
  if (!p.startsWith(`${r}/`)) return null;
  // Keep the original case of the relative part.
  return path
    .replace(/[\\/]+/g, "/")
    .replace(/\/$/, "")
    .slice(r.length + 1);
}

/** Joins a relative "/" path to `root` with root's own separator. */
export function joinPath(root: string, rel: string): string {
  if (!rel) return root;
  const sep = root.includes("\\") ? "\\" : "/";
  return `${root.replace(/[\\/]+$/, "")}${sep}${rel.split("/").join(sep)}`;
}

/** Template from a live session. Pane ids stay as layout keys. */
export function sessionToTemplate(
  name: string,
  tab: Pick<Tab, "paneIds" | "layout">,
  panes: Record<string, Pane>,
  workspacePath: string,
): SessionTemplate {
  return {
    id: crypto.randomUUID(),
    name,
    layout: tab.layout,
    panes: tab.paneIds.flatMap((id) => {
      const p = panes[id];
      if (!p) return [];
      const cwd = relativePath(p.cwd, workspacePath);
      return [
        { key: id, agent: p.agent, title: p.title, cwd, args: p.args, worktree: p.worktree, fontSize: p.fontSize },
      ];
    }),
  };
}

/** Fresh pane specs + layout for a new session. `fallbackCwd` is used when the workspace has no folder. */
export function instantiate(
  template: SessionTemplate,
  workspacePath: string,
  fallbackCwd: string,
  defaults: { fontSize: number; agents: Record<AgentKind, { args: string; worktree: boolean }> },
): { layout: SerializedDockview | null; panes: PaneSpec[] } {
  let layoutJson = template.layout ? JSON.stringify(template.layout) : null;
  const panes = template.panes.map((p): PaneSpec => {
    const id = crypto.randomUUID();
    // Keys are pane UUIDs (or short builtin keys with no layout): a plain replace remaps the layout.
    if (layoutJson) layoutJson = layoutJson.split(JSON.stringify(p.key)).join(JSON.stringify(id));
    const base = workspacePath || fallbackCwd;
    return {
      id,
      agent: p.agent,
      title: p.title,
      cwd: workspacePath && p.cwd !== null ? joinPath(workspacePath, p.cwd) : base,
      args: p.args ?? splitArgs(defaults.agents[p.agent].args),
      worktree: p.worktree ?? defaults.agents[p.agent].worktree,
      completedCount: 0,
      fontSize: p.fontSize ?? defaults.fontSize,
    };
  });
  return { layout: layoutJson ? JSON.parse(layoutJson) : null, panes };
}

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;

/** Keeps only well-formed user templates read from disk. */
export function parseTemplates(raw: unknown): SessionTemplate[] {
  if (!Array.isArray(raw)) return [];
  return raw.filter(
    (t): t is SessionTemplate =>
      isObject(t) &&
      typeof t.id === "string" &&
      typeof t.name === "string" &&
      Array.isArray(t.panes) &&
      t.panes.every((p) => isObject(p) && typeof p.key === "string" && typeof p.agent === "string"),
  );
}

const KEY = "templates";
let store: Store | undefined;

/** User templates only; builtins are added by `allTemplates`. */
export const useTemplates = create<{ list: SessionTemplate[] }>(() => ({ list: [] }));

export const templatesReady: Promise<void> = (async () => {
  try {
    store = await load("templates.json", { autoSave: false });
    useTemplates.setState({ list: parseTemplates(await store.get(KEY)) });
  } catch (err) {
    console.warn("templates load failed", err);
  }
})();

async function persist(list: SessionTemplate[]): Promise<void> {
  useTemplates.setState({ list });
  if (!store) throw new Error("template storage unavailable");
  await store.set(KEY, list);
  await store.save();
}

export const allTemplates = (list = useTemplates.getState().list) => [...BUILTIN_TEMPLATES, ...list];

/** Saves the active session (or `tabId`) as a template. Rejects if the session is empty or the write fails. */
export async function saveSessionAsTemplate(name: string, tabId = useDeck.getState().activeTabId): Promise<void> {
  const state = useDeck.getState();
  const tab = state.tabs.find((t) => t.id === tabId);
  if (!tab || tab.paneIds.length === 0) throw new Error("the session has no agents");
  const workspace = state.workspaces.find((w) => w.id === tab.workspaceId);
  const template = sessionToTemplate(name.trim() || tab.name, tab, state.panes, workspace?.path ?? "");
  await persist([...useTemplates.getState().list, template]);
}

export const renameTemplate = (id: string, name: string) =>
  persist(useTemplates.getState().list.map((t) => (t.id === id ? { ...t, name } : t)));

export const deleteTemplate = (id: string) => persist(useTemplates.getState().list.filter((t) => t.id !== id));

/** Opens a new session in the active workspace from a template and spawns its agents. */
export function newSessionFromTemplate(templateId: string): void {
  const template = allTemplates().find((t) => t.id === templateId);
  if (!template) return;
  const workspace = activeWorkspace();
  const { lastCwd } = useDeck.getState();
  if (!workspace.path && !lastCwd) {
    deck.toast("No working folder: link a workspace or add an agent first.");
    return;
  }
  const { layout, panes } = instantiate(template, workspace.path, lastCwd, useSettings.getState());
  const tabId = deck.addTab(newTab(template.name, workspace.id, layout));
  for (const spec of panes) void createPane(spec, tabId);
}
