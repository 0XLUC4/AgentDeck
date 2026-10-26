// Per-pane task log built from hook events: what was asked, what the agent did, what it answered. In memory only.
import { create } from "zustand";
import { onTaskStart } from "../lib/checkpoints";
import type { AgentStatusEvent, PaneStatus } from "../lib/types";

export const MAX_ACTIONS = 200;
export const MAX_HISTORY = 20;

export interface TaskAction {
  at: number;
  text: string;
}

export interface Task {
  prompt: string | null;
  startedAt: number;
  actions: TaskAction[];
  summary: string | null;
  endedAt: number | null;
  status: PaneStatus;
}

/** `current` is the running task; finished ones move to `history` (newest first). */
export interface TaskLog {
  current: Task | null;
  history: Task[];
}

const EMPTY: TaskLog = { current: null, history: [] };

const newTask = (startedAt: number, prompt: string | null = null): Task => ({
  prompt,
  startedAt,
  actions: [],
  summary: null,
  endedAt: null,
  status: "working",
});

const archive = (history: Task[], task: Task) => [task, ...history].slice(0, MAX_HISTORY);

/**
 * Pure reducer. A task starts on UserPromptSubmit or on the first working event without a running task,
 * ends on done/error. `fallbackStart` dates tasks seen only at their end (Codex notify).
 */
export function applyTaskEvent(
  log: TaskLog = EMPTY,
  e: Pick<AgentStatusEvent, "status" | "event" | "lastAction" | "prompt" | "summary">,
  now: number,
  fallbackStart = now,
): TaskLog {
  let { current, history } = log;
  if (e.event === "UserPromptSubmit" || (e.status === "working" && !current)) {
    // A new prompt while a task runs: keep the interrupted one in history.
    if (current) history = archive(history, { ...current, endedAt: now });
    current = newTask(now, e.prompt ?? null);
  }
  if (!current && e.status !== "done" && e.status !== "error") return { current, history };
  const task = current ?? newTask(Math.min(fallbackStart, now));
  const last = task.actions[task.actions.length - 1];
  const actions =
    e.lastAction && e.status !== "error" && last?.text !== e.lastAction
      ? [...task.actions, { at: now, text: e.lastAction }].slice(-MAX_ACTIONS)
      : task.actions;
  const updated: Task = { ...task, actions, prompt: task.prompt ?? e.prompt ?? null };
  if (e.status !== "done" && e.status !== "error") return { current: updated, history };
  const summary = e.summary ?? (e.status === "error" ? e.lastAction : null) ?? null;
  return { current: null, history: archive(history, { ...updated, summary, endedAt: now, status: e.status }) };
}

export const useTasks = create<Record<string, TaskLog>>(() => ({}));

export const tasks = {
  apply(paneId: string, e: AgentStatusEvent, fallbackStart?: number, now = Date.now()) {
    const before = useTasks.getState()[paneId]?.current;
    const after = applyTaskEvent(useTasks.getState()[paneId], e, now, fallbackStart);
    useTasks.setState({ [paneId]: after });
    // shortcut: snapshot runs async after the hook answered, so a very fast first edit can land before it.
    if (after.current && after.current.startedAt !== before?.startedAt) onTaskStart(paneId);
  },
  remove(paneId: string) {
    const { [paneId]: _removed, ...rest } = useTasks.getState();
    useTasks.setState(rest, true);
  },
};
