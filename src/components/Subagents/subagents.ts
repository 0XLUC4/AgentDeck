// Claude's internal subagents, seen through hooks, shown as read-only panes next to their parent. In memory only.
import { create } from "zustand";
import { onAgentStatus, onAgentSubagent } from "../../lib/ipc";
import type { AgentSubagentEvent } from "../../lib/types";
import { useDeck } from "../../store/deck";

const MAX_ACTIONS = 200;

export interface Subagent {
  id: string;
  type: string;
  startedAt: number;
  endedAt: number | null;
  actions: { at: number; text: string }[];
  summary: string | null;
}

/** Subagents per parent pane id, in start order. */
export type Subagents = Record<string, Subagent[]>;

export const useSubagents = create<Subagents>(() => ({}));

/** Start creates the subagent; a tool call from an unknown one creates it too (hooks may arrive out of order). */
export function applySubagentEvent(state: Subagents, e: AgentSubagentEvent, now: number): Subagents {
  const list = state[e.paneId] ?? [];
  const found = list.find((a) => a.id === e.agentId);
  let agent: Subagent = found ?? {
    id: e.agentId,
    type: e.agentType || "subagent",
    startedAt: now,
    endedAt: null,
    actions: [],
    summary: null,
  };
  if (e.agentType) agent = { ...agent, type: e.agentType };
  if (e.event === "PreToolUse" && e.action) {
    agent = { ...agent, actions: [...agent.actions, { at: now, text: e.action }].slice(-MAX_ACTIONS) };
  }
  if (e.event === "SubagentStop") agent = { ...agent, endedAt: now, summary: e.summary };
  return {
    ...state,
    [e.paneId]: found ? list.map((a) => (a.id === e.agentId ? agent : a)) : [...list, agent],
  };
}

/** Keeps only the subagents of `paneId` matching `keep`; drops the entry when none are left. */
function keepOnly(state: Subagents, paneId: string, keep: (a: Subagent) => boolean): Subagents {
  const list = state[paneId];
  if (!list) return state;
  const kept = list.filter(keep);
  if (kept.length === list.length) return state;
  const { [paneId]: _old, ...rest } = state;
  return kept.length > 0 ? { ...rest, [paneId]: kept } : rest;
}

/** A new prompt to the parent: finished subagents of the previous turn go away. */
export const clearFinished = (state: Subagents, paneId: string) => keepOnly(state, paneId, (a) => a.endedAt === null);
export const dismissSubagent = (state: Subagents, paneId: string, id: string) =>
  keepOnly(state, paneId, (a) => a.id !== id);
export const dismiss = (paneId: string, id: string) =>
  useSubagents.setState((s) => dismissSubagent(s, paneId, id), true);

/** Dock panel id of a subagent viewer. Pane ids have no ":", agent ids may. */
export const subagentPanelId = (paneId: string, agentId: string) => `sub:${paneId}:${agentId}`;
export function parseSubagentPanel(panelId: string): { paneId: string; agentId: string } | null {
  if (!panelId.startsWith("sub:")) return null;
  const rest = panelId.slice("sub:".length);
  const at = rest.indexOf(":");
  return at > 0 ? { paneId: rest.slice(0, at), agentId: rest.slice(at + 1) } : null;
}

/** Subscribes once (main window, app start). */
export function listenSubagents(): Promise<unknown> {
  // A closed parent takes its subagents with it.
  useDeck.subscribe((state, prev) => {
    if (state.panes === prev.panes) return;
    const gone = Object.keys(useSubagents.getState()).filter((id) => !state.panes[id]);
    if (gone.length > 0)
      useSubagents.setState((s) => gone.reduce((acc, id) => keepOnly(acc, id, () => false), s), true);
  });
  return Promise.all([
    onAgentSubagent((e) => useSubagents.setState((s) => applySubagentEvent(s, e, Date.now()), true)),
    onAgentStatus((e) => {
      if (e.event === "UserPromptSubmit") useSubagents.setState((s) => clearFinished(s, e.paneId), true);
    }),
  ]);
}
