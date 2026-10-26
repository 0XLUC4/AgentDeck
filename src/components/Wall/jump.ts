import { revealPane } from "../../app/Deck";
import { useDeck } from "../../store/deck";
import { focusPane } from "../../store/runtime";
import { useWall } from "./wallState";

/** Leaves the wall, shows the pane's session, reveals the pane and focuses its terminal (same as the activity panel). */
export function jumpToPane(tabId: string, paneId: string): void {
  useWall.setState({ open: false });
  useDeck.setState({ activeTabId: tabId });
  revealPane(tabId, paneId);
  focusPane(paneId);
  // The session becomes visible on the next render; focus the terminal after it.
  requestAnimationFrame(() =>
    document.querySelector<HTMLElement>(`[data-pane-id="${CSS.escape(paneId)}"] .xterm-helper-textarea`)?.focus(),
  );
}

/** Pane the pane-level actions target: the focused one if it is in the visible session, else that session's first. */
export function currentPane(): { tabId: string; paneId: string } | null {
  const { tabs, activeTabId, activePaneId } = useDeck.getState();
  const tab = tabs.find((t) => t.id === activeTabId);
  if (!tab) return null;
  const paneId = activePaneId && tab.paneIds.includes(activePaneId) ? activePaneId : tab.paneIds[0];
  return paneId ? { tabId: tab.id, paneId } : null;
}
