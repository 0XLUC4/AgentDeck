// UI state: panes, session tabs, archived sessions, toasts. Side effects (PTY, IPC) live in runtime.ts.
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
}

export interface Pane extends PaneSpec {
  status: PaneStatus;
  lastAction: string | null;
  taskStartedAt: number | null;
  exited: boolean;
  /** True once a hooks event arrived; otherwise status is derived from output. */
  hooked: boolean;
  /** Bumped on restart so the terminal view remounts on a fresh buffer. */
  generation: number;
}

export interface Tab {
  id: string;
  name: string;
  paneIds: string[];
  layout: SerializedDockview | null;
}

/** A closed session tab, kept so it can be reopened. */
export interface SavedSession {
  id: string;
  name: string;
  savedAt: number;
  layout: SerializedDockview | null;
  panes: PaneSpec[];
}

export interface Toast {
  id: number;
  message: string;
}

interface DeckState {
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
    taskStartedAt: keepsTimer ? (pane.taskStartedAt ?? now) : null,
    completedCount: pane.completedCount + (status === "done" && pane.status !== "done" ? 1 : 0),
  };
}

export const clampFontSize = (size: number) =>
  Number.isFinite(size) ? Math.min(FONT_SIZE.max, Math.max(FONT_SIZE.min, Math.round(size))) : FONT_SIZE.default;

export const newPane = (spec: PaneSpec): Pane => ({
  ...spec,
  fontSize: clampFontSize(spec.fontSize),
  status: "idle",
  lastAction: null,
  taskStartedAt: null,
  exited: false,
  hooked: false,
  generation: 0,
});

export const toSpec = ({ id, agent, title, cwd, args, worktree, completedCount, fontSize }: Pane): PaneSpec => ({
  id,
  agent,
  title,
  cwd,
  args,
  worktree,
  completedCount,
  fontSize,
});

export const newTab = (name: string, layout: SerializedDockview | null = null): Tab => ({
  id: crypto.randomUUID(),
  name,
  paneIds: [],
  layout,
});

/** Saved form of a tab: its layout plus the spec of each live pane. */
export function tabToSession(tab: Tab, panes: Record<string, Pane>, now: number): SavedSession {
  return {
    id: tab.id,
    name: tab.name,
    savedAt: now,
    layout: tab.layout,
    panes: tab.paneIds.flatMap((id) => (panes[id] ? [toSpec(panes[id])] : [])),
  };
}

const firstTab = newTab("Session 1");

export const useDeck = create<DeckState>(() => ({
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

const updateTab = (id: string, fn: (tab: Tab) => Tab) =>
  set({ tabs: get().tabs.map((t) => (t.id === id ? fn(t) : t)) });

export const activeTab = () => get().tabs.find((t) => t.id === get().activeTabId);

export const deck = {
  updatePane,
  addPane(pane: Pane, tabId: string) {
    set({ panes: { ...get().panes, [pane.id]: pane } });
    updateTab(tabId, (t) => ({ ...t, paneIds: [...t.paneIds, pane.id] }));
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
  rename(id: string, title: string) {
    updatePane(id, (p) => ({ ...p, title }));
  },
  zoom(id: string, delta: number | "reset") {
    updatePane(id, (p) => ({
      ...p,
      fontSize: delta === "reset" ? FONT_SIZE.default : clampFontSize(p.fontSize + delta),
    }));
  },
  addTab(tab = newTab(`Session ${get().tabs.length + 1}`)) {
    set({ tabs: [...get().tabs, tab], activeTabId: tab.id });
    return tab.id;
  },
  /** Removes the tab only. Always keeps one tab. */
  removeTab(id: string) {
    const tabs = get().tabs.filter((t) => t.id !== id);
    if (tabs.length === 0) tabs.push(newTab("Session 1"));
    const activeTabId = tabs.some((t) => t.id === get().activeTabId) ? get().activeTabId : tabs[0].id;
    set({ tabs, activeTabId });
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
    const tabId = deck.addTab({ id: session.id, name: session.name, paneIds: [], layout: session.layout });
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
  toast(message: string) {
    const id = Date.now() + Math.random();
    set({ toasts: [...get().toasts, { id, message }] });
    setTimeout(() => set({ toasts: get().toasts.filter((t) => t.id !== id) }), 6000);
  },
};
