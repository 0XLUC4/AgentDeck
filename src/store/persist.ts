// Saves open tabs (layout + pane specs) and archived sessions to store.json; restores them on startup.
import { load, type Store } from "@tauri-apps/plugin-store";
import { deck, type SavedSession, type Tab, tabToSession, useDeck } from "./deck";
import { createPane } from "./runtime";

const KEY = "deck";

interface Saved {
  version: 1;
  tabs: SavedSession[];
  activeTabId: string;
  lastCwd: string;
  archived?: SavedSession[];
}

let store: Store | undefined;

function snapshot(): Saved {
  const { tabs, panes, activeTabId, lastCwd, archived } = useDeck.getState();
  const now = Date.now();
  return { version: 1, activeTabId, lastCwd, archived, tabs: tabs.map((t) => tabToSession(t, panes, now)) };
}

/** Loads the saved session (respawning each pane fresh), then autosaves on every change. */
export async function restoreAndAutosave(): Promise<void> {
  try {
    store = await load("store.json", { autoSave: 500 });
    const saved = await store.get<Saved>(KEY);
    if (saved?.version === 1 && saved.tabs.length > 0) {
      const tabs: Tab[] = saved.tabs.map((t) => ({ id: t.id, name: t.name, layout: t.layout, paneIds: [] }));
      useDeck.setState({
        tabs,
        activeTabId: saved.activeTabId,
        lastCwd: saved.lastCwd,
        archived: saved.archived ?? [],
      });
      for (const tab of saved.tabs) {
        for (const pane of tab.panes) void createPane(pane, tab.id);
      }
    }
  } catch (err) {
    deck.toast(`Impossible de restaurer la session : ${err instanceof Error ? err.message : String(err)}`);
  }
  useDeck.subscribe((state, prev) => {
    const changed =
      state.tabs !== prev.tabs ||
      state.panes !== prev.panes ||
      state.archived !== prev.archived ||
      state.lastCwd !== prev.lastCwd ||
      state.activeTabId !== prev.activeTabId;
    if (changed) store?.set(KEY, snapshot()).catch((err) => console.warn("save failed", err));
  });
}
