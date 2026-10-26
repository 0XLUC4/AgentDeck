// Saves workspaces, open tabs (layout + pane specs) and archived sessions to store.json; restores them on startup.
import { load, type Store } from "@tauri-apps/plugin-store";
import {
  DEFAULT_WORKSPACE_NAME,
  deck,
  newTab,
  type SavedSession,
  type Tab,
  tabToSession,
  useDeck,
  type Workspace,
} from "./deck";
import { createPane } from "./runtime";

const KEY = "deck";

export interface Saved {
  version: 2;
  workspaces: Workspace[];
  tabs: SavedSession[];
  activeTabId: string;
  lastCwd: string;
  archived: SavedSession[];
}

/** v1 had no workspaces: sessions had no workspaceId. */
interface SavedV1 {
  version: 1;
  tabs: Omit<SavedSession, "workspaceId">[];
  activeTabId: string;
  lastCwd: string;
  archived?: Omit<SavedSession, "workspaceId">[];
}

let store: Store | undefined;

/** Upgrades any saved shape to v2. Returns null when there is nothing usable. */
export function migrate(saved: unknown): Saved | null {
  const data = saved as Saved | SavedV1 | undefined;
  if (data?.version === 2) return data.workspaces.length > 0 && data.tabs.length > 0 ? data : null;
  if (data?.version !== 1 || data.tabs.length === 0) return null;
  // Old sessions go into a "Par défaut" workspace; agents there keep starting in the last used folder.
  const ws: Workspace = { id: crypto.randomUUID(), name: DEFAULT_WORKSPACE_NAME, path: "" };
  return {
    version: 2,
    workspaces: [ws],
    tabs: data.tabs.map((t) => ({ ...t, workspaceId: ws.id })),
    activeTabId: data.activeTabId,
    lastCwd: data.lastCwd,
    archived: (data.archived ?? []).map((s) => ({ ...s, workspaceId: ws.id })),
  };
}

function snapshot(): Saved {
  const { workspaces, tabs, panes, activeTabId, lastCwd, archived } = useDeck.getState();
  const now = Date.now();
  return { version: 2, workspaces, activeTabId, lastCwd, archived, tabs: tabs.map((t) => tabToSession(t, panes, now)) };
}

/** Applies a saved state to the store (without spawning). Returns the tabs to respawn. */
export function hydrate(saved: Saved): SavedSession[] {
  const known = new Set(saved.workspaces.map((w) => w.id));
  // A tab whose workspace vanished (hand-edited file) falls back to the first workspace.
  const fix = (s: SavedSession) => (known.has(s.workspaceId) ? s : { ...s, workspaceId: saved.workspaces[0].id });
  const sessions = saved.tabs.map(fix);
  const tabs: Tab[] = sessions.map(({ id, name, workspaceId, layout }) => ({
    id,
    name,
    workspaceId,
    layout,
    paneIds: [],
  }));
  for (const ws of saved.workspaces) {
    if (!tabs.some((t) => t.workspaceId === ws.id)) tabs.push(newTab("Session 1", ws.id));
  }
  useDeck.setState({
    workspaces: saved.workspaces,
    tabs,
    activeTabId: tabs.some((t) => t.id === saved.activeTabId) ? saved.activeTabId : tabs[0].id,
    lastCwd: saved.lastCwd,
    archived: saved.archived.map(fix),
  });
  return sessions;
}

/** Loads the saved state (respawning each pane fresh), then autosaves on every change. */
export async function restoreAndAutosave(): Promise<void> {
  try {
    store = await load("store.json", { autoSave: 500 });
    const saved = migrate(await store.get(KEY));
    if (saved) {
      for (const tab of hydrate(saved)) {
        for (const pane of tab.panes) void createPane(pane, tab.id);
      }
    }
  } catch (err) {
    deck.toast(`Impossible de restaurer la session : ${err instanceof Error ? err.message : String(err)}`);
  }
  useDeck.subscribe((state, prev) => {
    const changed =
      state.workspaces !== prev.workspaces ||
      state.tabs !== prev.tabs ||
      state.panes !== prev.panes ||
      state.archived !== prev.archived ||
      state.lastCwd !== prev.lastCwd ||
      state.activeTabId !== prev.activeTabId;
    if (changed) store?.set(KEY, snapshot()).catch((err) => console.warn("save failed", err));
  });
}
