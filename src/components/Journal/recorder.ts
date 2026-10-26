// Persists every task that leaves a pane's "current" slot (finished or interrupted) to today's journal file,
// so the journal survives restarts. Tasks themselves stay in memory (store/tasks.ts).
import { journalAppend } from "../../lib/ipc";
import { useDeck } from "../../store/deck";
import { useTasks } from "../../store/tasks";
import { toJournalEntry } from "./journal";

let started = false;

/** Subscribes once (module-level call from App). */
export function startJournalRecorder(): void {
  if (started) return;
  started = true;
  useTasks.subscribe((state, prev) => {
    for (const [paneId, log] of Object.entries(state)) {
      const latest = log.history[0];
      if (!latest || latest === prev[paneId]?.history[0] || latest.endedAt === null) continue;
      const deck = useDeck.getState();
      const pane = deck.panes[paneId];
      if (!pane) continue;
      const tab = deck.tabs.find((t) => t.paneIds.includes(paneId));
      const workspace = deck.workspaces.find((w) => w.id === tab?.workspaceId);
      journalAppend(toJournalEntry(pane, workspace, latest)).catch((err) => console.warn("journal_append failed", err));
    }
  });
}
