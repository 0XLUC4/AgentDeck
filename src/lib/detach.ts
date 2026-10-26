// Detached pane windows: event protocol shared by the main window (host) and pane.html. Pure: no store, no IPC.
import type { Pane } from "../store/deck";

export const paneWindowLabel = (paneId: string) => `pane-${paneId}`;
/** localStorage key of a pane window's size/position, saved when it closes (logical pixels). */
export const geometryKey = (paneId: string) => `agentdeck:pane-window:${paneId}`;

/** pane window -> main: send me the scrollback. Payload: ReplayRequest. */
export const REPLAY_REQUEST = "pane-replay-request";
/** main -> pane window: scrollback + current pane state. Payload: Replay. */
export const REPLAY = "pane-replay";
/** main -> pane window: the pane changed in the store. Payload: Pane. */
export const PANE_META = "pane-meta";
/** pane window -> main: input, resize, focus, zoom. Payload: RemoteAction. */
export const PANE_REMOTE = "pane-remote";

export interface ReplayRequest {
  paneId: string;
  windowLabel: string;
}

export interface Replay {
  paneId: string;
  text: string;
  pane: Pane;
  fontFamily: string;
}

export type RemoteBody =
  | { kind: "input"; data: string }
  | { kind: "resize"; cols: number; rows: number }
  | { kind: "focus" }
  | { kind: "zoom"; delta: number | "reset" };
export type RemoteAction = { paneId: string } & RemoteBody;

/**
 * Exactly one xterm per pane forwards input and resizes, otherwise the agent gets duplicate
 * replies to terminal queries (ESC[6n, device attributes) and the two windows fight over the size.
 */
export const forwardsInput = (inDetachedWindow: boolean, paneDetached: boolean) => inDetachedWindow === paneDetached;

/**
 * Orders scrollback replay before live output: live chunks received before the replay are held,
 * then written after it, minus the leading chunks the replay already contains.
 */
export function replayGate(write: (data: string) => void) {
  let pending: string[] | null = [];
  return {
    output(data: string) {
      if (pending) pending.push(data);
      else write(data);
    },
    replay(text: string) {
      if (!pending) return;
      // shortcut: overlap is detected by suffix match, so a repeated chunk (spinner frame) may be dropped once.
      let skip = pending.length;
      while (skip > 0 && !text.endsWith(pending.slice(0, skip).join(""))) skip--;
      if (text) write(text);
      for (const data of pending.slice(skip)) write(data);
      pending = null;
    },
  };
}
