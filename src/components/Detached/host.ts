// Main-window side of detached panes: opens/closes pane-* windows and serves their scrollback, input and resizes.
import { emitTo, listen } from "@tauri-apps/api/event";
import { getAllWebviewWindows, WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { create } from "zustand";
import { AGENTS } from "../../lib/agents";
import {
  forwardsInput,
  geometryKey,
  PANE_META,
  PANE_REMOTE,
  paneWindowLabel,
  REPLAY,
  REPLAY_REQUEST,
  type RemoteAction,
  type Replay,
  type ReplayRequest,
} from "../../lib/detach";
import { deck, useDeck } from "../../store/deck";
import { focusPane, paneOutputText, reportSize, sendInput } from "../../store/runtime";
import { DEFAULT_FONT_FAMILY, useSettings } from "../../store/settings";

/** Panes currently shown in their own window. Not persisted: panes reopen attached. */
export const useDetached = create<Record<string, true>>(() => ({}));
export const isDetached = (paneId: string) => useDetached.getState()[paneId] === true;

const warn = (what: string) => (err: unknown) => console.warn(`${what} failed`, err);

interface Geometry {
  x: number;
  y: number;
  width: number;
  height: number;
}

function readGeometry(paneId: string): Geometry | null {
  try {
    const raw = JSON.parse(localStorage.getItem(geometryKey(paneId)) ?? "null");
    return raw && [raw.x, raw.y, raw.width, raw.height].every(Number.isFinite) ? raw : null;
  } catch {
    return null;
  }
}

function markAttached(paneId: string): void {
  useDetached.setState((s) => {
    const { [paneId]: _, ...rest } = s;
    return rest;
  }, true);
}

/** Opens the pane in its own window (or focuses it if already detached). */
export async function detachPane(paneId: string): Promise<void> {
  const pane = useDeck.getState().panes[paneId];
  if (!pane) return;
  const label = paneWindowLabel(paneId);
  const existing = await WebviewWindow.getByLabel(label);
  if (existing) {
    await existing.setFocus().catch(warn("setFocus"));
    return;
  }
  const geometry = readGeometry(paneId);
  // The main window's terminal stops forwarding as soon as this is set; the new window takes over.
  useDetached.setState({ [paneId]: true });
  const win = new WebviewWindow(label, {
    url: `pane.html?pane=${encodeURIComponent(paneId)}`,
    title: pane.title || AGENTS[pane.agent].name,
    width: geometry?.width ?? 900,
    height: geometry?.height ?? 600,
    ...(geometry ? { x: geometry.x, y: geometry.y } : { center: true }),
    minWidth: 360,
    minHeight: 200,
    decorations: false,
    shadow: true,
  });
  void win.once("tauri://error", (e) => {
    markAttached(paneId);
    deck.toast(`Could not detach the pane: ${String(e.payload)}`);
  });
  void win.once("tauri://destroyed", () => markAttached(paneId));
}

/** Closes the pane's window; the destroyed event puts the terminal back in the grid. */
export async function reattachPane(paneId: string): Promise<void> {
  const win = await WebviewWindow.getByLabel(paneWindowLabel(paneId));
  if (win) await win.close().catch(warn("close"));
  else markAttached(paneId);
}

export const toggleDetached = (paneId: string) => void (isDetached(paneId) ? reattachPane : detachPane)(paneId);

function onRemote(action: RemoteAction): void {
  const { paneId } = action;
  if (!useDeck.getState().panes[paneId]) return;
  if (action.kind === "focus") focusPane(paneId);
  else if (action.kind === "zoom") {
    if (action.delta === "reset" || Number.isFinite(action.delta)) deck.zoom(paneId, action.delta);
  }
  // Input and size only from the owner: a stale window must not double-reply to terminal queries.
  else if (!forwardsInput(true, isDetached(paneId))) return;
  else if (action.kind === "input" && typeof action.data === "string") sendInput(paneId, action.data);
  else if (action.kind === "resize" && action.cols > 0 && action.rows > 0) reportSize(paneId, action.cols, action.rows);
}

/** Main window only, once. Returns nothing: listeners live as long as the app. */
export function startDetachHost(): void {
  // A reload of the main window forgets which panes were detached: close leftovers so there is one owner.
  getAllWebviewWindows()
    .then((wins) => wins.filter((w) => w.label.startsWith("pane-")).map((w) => w.close()))
    .catch(warn("close leftover pane windows"));

  listen<ReplayRequest>(REPLAY_REQUEST, ({ payload }) => {
    const pane = useDeck.getState().panes[payload.paneId];
    if (!pane || payload.windowLabel !== paneWindowLabel(pane.id)) return;
    const fontFamily = useSettings.getState().fontFamily.trim() || DEFAULT_FONT_FAMILY;
    const replay: Replay = { paneId: pane.id, text: paneOutputText(pane.id), pane, fontFamily };
    emitTo(payload.windowLabel, REPLAY, replay).catch(warn("pane-replay"));
  }).catch(warn("pane-replay-request listener"));

  listen<RemoteAction>(PANE_REMOTE, ({ payload }) => onRemote(payload)).catch(warn("pane-remote listener"));

  // Keeps each detached window's header in sync; closes it when its pane is closed.
  useDeck.subscribe((state, prev) => {
    for (const paneId of Object.keys(useDetached.getState())) {
      const pane = state.panes[paneId];
      if (!pane) void reattachPane(paneId);
      else if (pane !== prev.panes[paneId]) emitTo(paneWindowLabel(paneId), PANE_META, pane).catch(warn("pane-meta"));
    }
  });
}
