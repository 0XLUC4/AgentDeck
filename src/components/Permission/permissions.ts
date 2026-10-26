// Pending agent permission prompts (Claude PermissionRequest hook), answered from the UI. In memory only.
import type { UnlistenFn } from "@tauri-apps/api/event";
import { create } from "zustand";
import { AGENTS } from "../../lib/agents";
import { answerPermission, onAgentPermission, onAgentPermissionResolved } from "../../lib/ipc";
import { notify } from "../../lib/notify";
import type { PermissionDecision, PermissionRequest } from "../../lib/types";
import { deck, useDeck } from "../../store/deck";
import { useSettings } from "../../store/settings";

/** Keyed by requestId; insertion order = arrival order. */
export type PendingPermissions = Record<string, PermissionRequest>;

export const addPermission = (state: PendingPermissions, req: PermissionRequest): PendingPermissions => ({
  ...state,
  [req.requestId]: req,
});

export function removePermission(state: PendingPermissions, requestId: string): PendingPermissions {
  if (!(requestId in state)) return state;
  const { [requestId]: _removed, ...rest } = state;
  return rest;
}

export const permissionsForPane = (state: PendingPermissions, paneId: string) =>
  Object.values(state).filter((r) => r.paneId === paneId);

export const usePermissions = create<PendingPermissions>(() => ({}));

const remove = (requestId: string) => usePermissions.setState((s) => removePermission(s, requestId), true);

/** Sends the user's answer; the request leaves the UI at once (the backend confirms with a resolved event). */
export async function answer(req: PermissionRequest, decision: PermissionDecision, message?: string): Promise<void> {
  remove(req.requestId);
  try {
    await answerPermission(req.requestId, decision, message);
  } catch (err) {
    deck.toast(`Answer not sent (${err instanceof Error ? err.message : String(err)}): reply in the terminal.`);
  }
}

/** Subscribes once to permission events; notifies like a "waiting" status would. */
export async function listenPermissions(): Promise<UnlistenFn> {
  const unlisten = await Promise.all([
    onAgentPermission((req) => {
      const state = useDeck.getState();
      const pane = state.panes[req.paneId];
      if (!pane) return;
      usePermissions.setState((s) => addPermission(s, req), true);
      const prefs = useSettings.getState().notify;
      const unseen = !document.hasFocus() || state.activePaneId !== req.paneId;
      if (prefs.waiting && (unseen || !prefs.onlyUnseen) && (prefs.enabled || prefs.sound)) {
        const title = `${AGENTS[pane.agent].name} is asking for permission: ${req.summary}`;
        void notify(title, pane.title, { system: prefs.enabled, sound: prefs.sound });
      }
    }),
    onAgentPermissionResolved(remove),
  ]);
  return () => {
    for (const fn of unlisten) fn();
  };
}
