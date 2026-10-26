// Per-pane prompt queue: when a hooked agent finishes (status "done"), the next prompt is sent automatically.
// In memory only (KISS): queues vanish with the app or the pane.
import { create } from "zustand";
import { sendPrompt } from "../components/Broadcast/send";
import type { AgentKind, PaneStatus } from "../lib/types";
import { deck, type Pane, useDeck } from "./deck";

export interface QueueItem {
  id: string;
  text: string;
}

export interface PaneQueue {
  items: QueueItem[];
  paused: boolean;
}

export const ADVANCE_DELAY_MS = 1500;
/** Only agents with hooks report "done"; the others never advance a queue. */
export const QUEUE_AGENTS: AgentKind[] = ["claude", "codex"];

export const EMPTY_QUEUE: PaneQueue = { items: [], paused: false };

export const useQueues = create<Record<string, PaneQueue>>(() => ({}));

/** True when a pane just finished and its queue has something to send. */
export const shouldAdvance = (prev: PaneStatus | undefined, next: PaneStatus, queue: PaneQueue | undefined) =>
  prev !== "done" && next === "done" && !!queue && !queue.paused && queue.items.length > 0;

/** After the delay: still alive and not busy again (looking at a done pane turns it idle). */
export const readyForNext = (pane: Pane | undefined) =>
  !!pane && !pane.exited && (pane.status === "done" || pane.status === "idle");

/** Moves the item at `index` by `delta` places (clamped). */
export function moveItem<T>(items: T[], index: number, delta: number): T[] {
  const to = Math.max(0, Math.min(items.length - 1, index + delta));
  if (index < 0 || index >= items.length || to === index) return items;
  const next = [...items];
  const [item] = next.splice(index, 1);
  next.splice(to, 0, item);
  return next;
}

const get = (paneId: string) => useQueues.getState()[paneId] ?? EMPTY_QUEUE;
const update = (paneId: string, fn: (q: PaneQueue) => PaneQueue) => useQueues.setState({ [paneId]: fn(get(paneId)) });

export const queues = {
  add(paneId: string, text: string) {
    const trimmed = text.trim();
    if (trimmed) update(paneId, (q) => ({ ...q, items: [...q.items, { id: crypto.randomUUID(), text: trimmed }] }));
  },
  remove(paneId: string, itemId: string) {
    update(paneId, (q) => ({ ...q, items: q.items.filter((i) => i.id !== itemId) }));
  },
  move(paneId: string, itemId: string, delta: number) {
    update(paneId, (q) => ({
      ...q,
      items: moveItem(
        q.items,
        q.items.findIndex((i) => i.id === itemId),
        delta,
      ),
    }));
  },
  setPaused(paneId: string, paused: boolean) {
    update(paneId, (q) => ({ ...q, paused }));
  },
  forget(paneId: string) {
    const { [paneId]: _removed, ...rest } = useQueues.getState();
    useQueues.setState(rest, true);
  },
};

/** Sends the next queued prompt now. Puts it back and toasts if the write fails. */
export async function sendNext(paneId: string): Promise<void> {
  const item = get(paneId).items[0];
  if (!item) return;
  queues.remove(paneId, item.id);
  try {
    await sendPrompt(paneId, item.text);
  } catch (err) {
    update(paneId, (q) => ({ ...q, items: [item, ...q.items] }));
    deck.toast(`File d'attente : envoi impossible (${err instanceof Error ? err.message : String(err)})`);
  }
}

/** Watches pane statuses; call once from the main window. Returns the unsubscribe function. */
export function startQueueRunner(): () => void {
  return useDeck.subscribe((state, prev) => {
    if (state.panes === prev.panes) return;
    for (const id of Object.keys(useQueues.getState())) if (!state.panes[id]) queues.forget(id);
    for (const pane of Object.values(state.panes)) {
      if (!shouldAdvance(prev.panes[pane.id]?.status, pane.status, useQueues.getState()[pane.id])) continue;
      setTimeout(() => {
        const queue = useQueues.getState()[pane.id];
        if (queue && !queue.paused && readyForNext(useDeck.getState().panes[pane.id])) void sendNext(pane.id);
      }, ADVANCE_DELAY_MS);
    }
  });
}
