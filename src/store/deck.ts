// UI state: workspaces, panes, session tabs, archived sessions, toasts. Side effects (PTY, IPC) live in runtime.ts.
import type { SerializedDockview } from "dockview-react";
import { create } from "zustand";
import type { AgentKind, PaneStatus } from "../lib/types";

export const FONT_SIZE = { min: 8, max: 32, default: 13 } as const;

/** What is saved for a pane and enough to respawn it. */
export interface PaneSpec {
  id: string;
  agent: AgentKind;
  title: string;
  cwd: string;
  args: string[];
  worktree: boolean;
  completedCount: number;
  fontSize: number;
  /** "Chef d'orchestre": a Claude pane wired to AgentDeck's MCP server (src/lib/orchestrator.ts). */
  orchestrator?: boolean;
}

export interface Pane extends PaneSpec {
  status: PaneStatus;
  lastAction: string | null;
  taskStartedAt: number | null;
  /** When `status` (or `exited`) last changed; drives "time in status" in the activity panel. Not persisted. */
  statusSince: number;
  exited: boolean;
  /** True once a hooks event arrived; otherwise status is derived from output. */
  hooked: boolean;
  /** Bumped on restart so the terminal view remounts on a fresh buffer. */
  generation: number;
}

/** A linked project folder. Owns its sessions (open tabs + archived). Unlinking never touches files. */
export interface Workspace {
  id: string;
  name: string;
  /** Folder path; "" for the default workspace (agents then start in the last used folder). */
  path: string;
  /** Set when the folder was a Git repository when linked (branch as of that moment). */
  git?: { branch: string | null };
}

export interface Tab {
  id: string;
  name: string;
  workspaceId: string;
  paneIds: string[];
  layout: SerializedDockview | null;
}

/** A closed session tab, kept so it can be reopened. */
export interface SavedSession {
  id: string;
  name: string;
  workspaceId: string;
  savedAt: number;
  layout: SerializedDockview | null;
  panes: PaneSpec[];
}

export interface Toast {
  id: number;
  message: string;
}

interface DeckState {
  workspaces: Workspace[];
  panes: Record<string, Pane>;
  tabs: Tab[];
  activeTabId: string;
  activePaneId: string | null;
  archived: SavedSession[];
  lastCwd: string;
  toasts: Toast[];
}

/** Pure status transition: starts the task timer, counts completed tasks. */
export function applyStatus(pane: Pane, status: PaneStatus, lastAction: string | null, now: number): Pane {
  const keepsTimer = status === "working" || status === "waiting";
  return {
    ...pane,
    status,
    lastAction: lastAction ?? pane.lastAction,
    statusSince: status === pane.status ? pane.statusSince : now,
    taskStartedAt: keepsTimer ? (pane.taskStartedAt ?? now) : null,
    completedCount: pane.completedCount + (status === "done" && pane.status !== "done" ? 1 : 0),
  };
}

const URGENCY: PaneStatus[] = ["waiting", "error", "working", "done", "idle"];

/** The status a session shows in lists: its most urgent pane's, or null when it has no pane. */
export function sessionStatus(paneIds: string[], panes: Record<string, Pane>): PaneStatus | null {
  const statuses = new Set(paneIds.flatMap((id) => (panes[id] ? [panes[id].status] : [])));
  return URGENCY.find((s) => statuses.has(s)) ?? null;
}

/** Activity panel grouping: a stopped process shows as "exited" unless it failed. */
export type ActivityKey = PaneStatus | "exited";
export const ACTIVITY_ORDER: ActivityKey[] = ["waiting", "error", "done", "working", "idle", "exited"];
export const activityKey = (pane: Pane): ActivityKey =>
  pane.exited && pane.status !== "error" ? "exited" : pane.status;

export interface ActivityItem {
  pane: Pane;
  tab: Tab;
  workspace: Workspace;
}

/** Every live pane of every workspace, grouped by ACTIVITY_ORDER (empty groups dropped), longest in status first. */
export function activityGroups(
  tabs: Tab[],
  workspaces: Workspace[],
  panes: Record<string, Pane>,
): { key: ActivityKey; items: ActivityItem[] }[] {
  const items = tabs.flatMap((tab) => {
    const workspace = workspaces.find((w) => w.id === tab.workspaceId);
    return workspace ? tab.paneIds.flatMap((id) => (panes[id] ? [{ pane: panes[id], tab, workspace }] : [])) : [];
  });
  return ACTIVITY_ORDER.map((key) => ({
    key,
    items: items.filter((i) => activityKey(i.pane) === key).sort((a, b) => a.pane.statusSince - b.pane.statusSince),
  })).filter((g) => g.items.length > 0);
}

export const clampFontSize = (size: number) =>
  Number.isFinite(size) ? Math.min(FONT_SIZE.max, Math.max(FONT_SIZE.min, Math.round(size))) : FONT_SIZE.default;

export const newPane = (spec: PaneSpec): Pane => ({
  ...spec,
  fontSize: clampFontSize(spec.fontSize),
  status: "idle",
  lastAction: null,
  taskStartedAt: null,
  statusSince: Date.now(),
  exited: false,
  hooked: false,
  generation: 0,
});

export const toSpec = ({
  id,
  agent,
  title,
  cwd,
  args,
  worktree,
  completedCount,
  fontSize,
  orchestrator,
}: Pane): PaneSpec => ({
  id,
  agent,
  title,
  cwd,
  args,
  worktree,
  completedCount,
  fontSize,
  ...(orchestrator ? { orchestrator } : {}),
});

export const newTab = (name: string, workspaceId: string, layout: SerializedDockview | null = null): Tab => ({
  id: crypto.randomUUID(),
  name,
  workspaceId,
  paneIds: [],
  layout,
});

/** Saved form of a tab: its layout plus the spec of each live pane. */
export function tabToSession(tab: Tab, panes: Record<string, Pane>, now: number): SavedSession {
  return {
    id: tab.id,
    name: tab.name,
    workspaceId: tab.workspaceId,
    savedAt: now,
    layout: tab.layout,
    panes: tab.paneIds.flatMap((id) => (panes[id] ? [toSpec(panes[id])] : [])),
  };
}

export const DEFAULT_WORKSPACE_NAME = "Default";

export const newWorkspace = (path: string, name = folderName(path) || DEFAULT_WORKSPACE_NAME): Workspace => ({
  id: crypto.randomUUID(),
  name,
  path,
});

/** Last segment of a Windows or POSIX path. */
export const folderName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? "";

const normalizePath = (path: string) =>
  path
    .replace(/[\\/]+$/, "")
    .replace(/\//g, "\\")
    .toLowerCase();

/** Workspace already linked to this folder (case- and slash-insensitive, Windows style). */
export const workspaceForPath = (path: string, workspaces: Workspace[]) =>
  path ? workspaces.find((w) => w.path && normalizePath(w.path) === normalizePath(path)) : undefined;

const firstWorkspace = newWorkspace("");
const firstTab = newTab("Session 1", firstWorkspace.id);

export const useDeck = create<DeckState>(() => ({
  workspaces: [firstWorkspace],
  panes: {},
  tabs: [firstTab],
  activeTabId: firstTab.id,
  activePaneId: null,
  archived: [],
  lastCwd: "",
  toasts: [],
}));

const set = useDeck.setState;
const get = useDeck.getState;

const updatePane = (id: string, fn: (pane: Pane) => Pane) => {
  const pane = get().panes[id];
  if (pane) set({ panes: { ...get().panes, [id]: fn(pane) } });
};

const sessionsOf = (workspaceId: string) => get().archived.filter((s) => s.workspaceId === workspaceId);

const updateTab = (id: string, fn: (tab: Tab) => Tab) =>
  set({ tabs: get().tabs.map((t) => (t.id === id ? fn(t) : t)) });

export const activeTab = () => get().tabs.find((t) => t.id === get().activeTabId);

/** The active workspace is the one owning the active tab. */
export const activeWorkspace = (state: DeckState = get()) => {
  const wsId = state.tabs.find((t) => t.id === state.activeTabId)?.workspaceId;
  return state.workspaces.find((w) => w.id === wsId) ?? state.workspaces[0];
};

/** Tab to show after `tabs` changed: keep the active one, else one of the same workspace, else the first. */
function pickActive(tabs: Tab[], preferWorkspace: string | undefined): string {
  const { activeTabId } = get();
  if (tabs.some((t) => t.id === activeTabId)) return activeTabId;
  return (tabs.find((t) => t.workspaceId === preferWorkspace) ?? tabs[0]).id;
}

export const deck = {
  updatePane,
  addPane(pane: Pane, tabId: string) {
    set({ panes: { ...get().panes, [pane.id]: pane } });
    // Idempotent: a restore that runs twice (dev hot reload) must not list a pane twice.
    updateTab(tabId, (t) => (t.paneIds.includes(pane.id) ? t : { ...t, paneIds: [...t.paneIds, pane.id] }));
  },
  removePane(id: string) {
    const { [id]: _removed, ...panes } = get().panes;
    set({
      panes,
      tabs: get().tabs.map((t) => ({ ...t, paneIds: t.paneIds.filter((p) => p !== id) })),
      activePaneId: get().activePaneId === id ? null : get().activePaneId,
    });
  },
  setStatus(id: string, status: PaneStatus, lastAction: string | null, now = Date.now()) {
    updatePane(id, (p) => applyStatus(p, status, lastAction, now));
  },
  /** "Seen": a finished pane goes back to idle so the activity list only shows what still needs attention. */
  acknowledge(id: string, now = Date.now()) {
    if (get().panes[id]?.status === "done") deck.setStatus(id, "idle", null, now);
  },
  rename(id: string, title: string) {
    updatePane(id, (p) => ({ ...p, title }));
  },
  zoom(id: string, delta: number | "reset") {
    updatePane(id, (p) => ({
      ...p,
      fontSize: delta === "reset" ? FONT_SIZE.default : clampFontSize(p.fontSize + delta),
    }));
  },
  /** Opens a new session tab in a workspace (the active one by default) and shows it. */
  addTab(tab?: Tab) {
    if (!tab) {
      const ws = activeWorkspace().id;
      const count = get().tabs.filter((t) => t.workspaceId === ws).length + sessionsOf(ws).length;
      tab = newTab(`Session ${count + 1}`, ws);
    }
    set({ tabs: [...get().tabs, tab], activeTabId: tab.id });
    return tab.id;
  },
  /** Removes the tab only. Every workspace keeps at least one open tab. */
  removeTab(id: string) {
    const removed = get().tabs.find((t) => t.id === id);
    if (!removed) return;
    const tabs = get().tabs.filter((t) => t.id !== id);
    if (!tabs.some((t) => t.workspaceId === removed.workspaceId)) tabs.push(newTab("Session 1", removed.workspaceId));
    set({ tabs, activeTabId: pickActive(tabs, removed.workspaceId) });
  },
  /** Moves a tab (layout + pane specs) to the archive. The caller kills its PTYs. Returns the pane ids. */
  archiveTab(id: string, now = Date.now()): string[] {
    const tab = get().tabs.find((t) => t.id === id);
    if (!tab) return [];
    set({ archived: [tabToSession(tab, get().panes, now), ...get().archived.filter((s) => s.id !== id)] });
    for (const paneId of tab.paneIds) deck.removePane(paneId);
    deck.removeTab(id);
    return tab.paneIds;
  },
  /** Removes a session from the archive and opens an empty tab with its layout. The caller respawns its panes. */
  unarchive(id: string): { tabId: string; panes: PaneSpec[] } | null {
    const session = get().archived.find((s) => s.id === id);
    if (!session) return null;
    set({ archived: get().archived.filter((s) => s.id !== id) });
    const workspaceId = get().workspaces.some((w) => w.id === session.workspaceId)
      ? session.workspaceId
      : activeWorkspace().id;
    const tabId = deck.addTab({ id: session.id, name: session.name, workspaceId, paneIds: [], layout: session.layout });
    return { tabId, panes: session.panes };
  },
  deleteArchived(id: string) {
    set({ archived: get().archived.filter((s) => s.id !== id) });
  },
  renameTab(id: string, name: string) {
    updateTab(id, (t) => ({ ...t, name }));
  },
  setTabLayout(id: string, layout: SerializedDockview) {
    updateTab(id, (t) => ({ ...t, layout }));
  },
  /** Links a folder as a new workspace with one empty session, and shows it. */
  linkWorkspace(path: string, name?: string, git?: Workspace["git"]): Workspace {
    const ws: Workspace = git ? { ...newWorkspace(path, name), git } : newWorkspace(path, name);
    set({ workspaces: [...get().workspaces, ws] });
    deck.addTab(newTab("Session 1", ws.id));
    return ws;
  },
  renameWorkspace(id: string, name: string) {
    set({ workspaces: get().workspaces.map((w) => (w.id === id ? { ...w, name } : w)) });
  },
  selectWorkspace(id: string) {
    if (activeWorkspace().id === id) return;
    const tab = get().tabs.find((t) => t.workspaceId === id);
    if (tab) set({ activeTabId: tab.id });
  },
  /**
   * Forgets a workspace with its open and archived sessions (files on disk are untouched).
   * Always keeps one workspace. The caller kills the returned panes.
   */
  unlinkWorkspace(id: string): string[] {
    const state = get();
    if (!state.workspaces.some((w) => w.id === id)) return [];
    const doomed = state.tabs.filter((t) => t.workspaceId === id);
    const paneIds = doomed.flatMap((t) => t.paneIds);
    const { panes } = state;
    const workspaces = state.workspaces.filter((w) => w.id !== id);
    if (workspaces.length === 0) workspaces.push(newWorkspace(""));
    const tabs = state.tabs.filter((t) => t.workspaceId !== id);
    if (tabs.length === 0) tabs.push(newTab("Session 1", workspaces[0].id));
    set({
      workspaces,
      tabs,
      panes: Object.fromEntries(Object.entries(panes).filter(([paneId]) => !paneIds.includes(paneId))),
      archived: state.archived.filter((s) => s.workspaceId !== id),
      activeTabId: pickActive(tabs, undefined),
      activePaneId: paneIds.includes(state.activePaneId ?? "") ? null : state.activePaneId,
    });
    return paneIds;
  },
  toast(message: string) {
    const id = Date.now() + Math.random();
    set({ toasts: [...get().toasts, { id, message }] });
    setTimeout(() => set({ toasts: get().toasts.filter((t) => t.id !== id) }), 6000);
  },
};
