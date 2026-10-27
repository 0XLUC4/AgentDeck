// Pane lifecycle: IPC subscriptions, spawn/restart/close, output buffering, status events.
import type { UnlistenFn } from "@tauri-apps/api/event";
import { AGENTS, interruptKey } from "../lib/agents";
import {
  browserMcpArgs,
  killPane,
  mcpRegister,
  onAgentStatus,
  onPaneExit,
  onPaneOutput,
  resizePane,
  setActivePane,
  spawnPane,
  writeToPane,
} from "../lib/ipc";
import { notify } from "../lib/notify";
import { RingBuffer } from "../lib/ringBuffer";
import { activeTab, deck, newPane, type PaneSpec, useDeck } from "./deck";
import { useSettings } from "./settings";
import { tasks } from "./tasks";

/** Output within this window counts as "working" for panes without hooks. */
const ACTIVITY_WINDOW_MS = 3000;

interface Runtime {
  buffer: RingBuffer;
  listeners: Set<(data: string) => void>;
  unlisten: UnlistenFn[];
  lastOutputAt: number;
  cols: number;
  rows: number;
  onExit: (() => void) | null;
  /** When the current process was spawned (ms); pane usage counts from here. */
  startedAt?: number;
  /** Adopted after a webview reload: the next size report forces the TUI to repaint. */
  redraw?: boolean;
  /** The terminal has reported its size at least once. */
  sized?: boolean;
}

const runtimes = new Map<string, Runtime>();

/** Spawn time of the pane's current process, or null before it starts. */
export const paneStartedAt = (id: string): number | null => runtimes.get(id)?.startedAt ?? null;
/** Buffered raw terminal output of a pane ("" when unknown). */
export const paneOutput = (id: string): string => runtimes.get(id)?.buffer.text() ?? "";
/** In-flight stop/restart per pane: a second click joins the first instead of racing a second spawn. */
const stopping = new Map<string, Promise<void>>();
const restarting = new Map<string, Promise<void>>();
/** Pane the backend routes voice input to. */
let backendActivePane: string | null = null;

/** Runs `job` once per id at a time; concurrent callers share the same promise. */
function once(inflight: Map<string, Promise<void>>, id: string, job: () => Promise<void>): Promise<void> {
  const running = inflight.get(id);
  if (running) return running;
  const promise = job().finally(() => inflight.delete(id));
  inflight.set(id, promise);
  return promise;
}

const errorMessage = (err: unknown) => (err instanceof Error ? err.message : String(err));

/**
 * Creates the pane in the store, subscribes to its output, then spawns the process.
 * `adopt`: the process is still alive in the backend (webview reload), only reattach to it.
 */
export async function createPane(spec: PaneSpec, tabId: string, adopt = false): Promise<void> {
  // Already running in this page (restore ran twice after a dev hot reload): keep the existing runtime.
  if (runtimes.has(spec.id)) {
    deck.addPane(useDeck.getState().panes[spec.id] ?? newPane(spec), tabId);
    return;
  }
  deck.addPane(newPane(spec), tabId);
  const rt: Runtime = {
    buffer: new RingBuffer(),
    listeners: new Set(),
    unlisten: [],
    lastOutputAt: 0,
    cols: 80,
    rows: 24,
    onExit: null,
  };
  runtimes.set(spec.id, rt);
  try {
    rt.unlisten = await Promise.all([
      onPaneOutput(spec.id, (data) => {
        rt.buffer.push(data);
        rt.lastOutputAt = Date.now();
        for (const fn of rt.listeners) fn(data);
      }),
      onPaneExit(spec.id, () => {
        deck.updatePane(spec.id, (p) => ({ ...p, exited: true, statusSince: Date.now() }));
        rt.onExit?.();
      }),
    ]);
  } catch (err) {
    failPane(spec.id, err);
    return;
  }
  // Closed while subscribing: drop the listeners instead of leaking them.
  if (runtimes.get(spec.id) !== rt) {
    for (const unlisten of rt.unlisten) unlisten();
    return;
  }
  if (adopt) adoptPane(spec.id);
  else await spawn(spec.id);
}

/** Reattaches to a live process: no spawn; the screen is repainted on the first size report. */
function adoptPane(id: string): void {
  const rt = runtimes.get(id);
  if (!rt) return;
  rt.startedAt = Date.now();
  if (rt.sized) nudgeSize(id, rt);
  else rt.redraw = true;
  deck.updatePane(id, (p) => ({ ...p, exited: false, statusSince: Date.now() }));
}

/** TUIs repaint on resize: nudge the width so even an unchanged size redraws the adopted screen. */
function nudgeSize(id: string, rt: Runtime): void {
  resizePane(id, Math.max(rt.cols - 1, 1), rt.rows)
    .then(() => resizePane(id, rt.cols, rt.rows))
    .catch(() => undefined);
}

const ALREADY_RUNNING = "pane id already in use";

async function spawn(id: string): Promise<void> {
  const pane = useDeck.getState().panes[id];
  const rt = runtimes.get(id);
  if (!pane || !rt) return;
  const { cols, rows } = rt;
  rt.startedAt = Date.now();
  try {
    // Orchestrator wiring (MCP config, role prompt, secret) is regenerated on each spawn; the secret only
    // goes to this process's env.
    const mcp = pane.orchestrator ? await mcpRegister(id) : null;
    const browser = pane.browser && pane.agent === "claude" ? await browserMcpArgs(id) : [];
    await spawnPane({
      paneId: id,
      agent: pane.agent,
      cwd: pane.cwd,
      args: [...browser, ...(mcp?.args ?? []), ...pane.args],
      worktree: pane.worktree,
      cols,
      rows,
      title: pane.title,
      env: mcp ? { AGENTDECK_MCP_SECRET: mcp.secret } : undefined,
      trust: useSettings.getState().trustAgentFolders,
    });
    // The terminal may have been fitted while spawning.
    if (rt.cols !== cols || rt.rows !== rows) await resizePane(id, rt.cols, rt.rows);
  } catch (err) {
    // Race with a webview reload: the process is already running under this id.
    if (errorMessage(err) === ALREADY_RUNNING) adoptPane(id);
    else failPane(id, err);
  }
}

function failPane(id: string, err: unknown): void {
  const pane = useDeck.getState().panes[id];
  deck.updatePane(id, (p) => ({
    ...p,
    status: "error",
    exited: true,
    lastAction: errorMessage(err),
    statusSince: Date.now(),
  }));
  deck.toast(`Failed to start ${pane ? AGENTS[pane.agent].name : "the agent"}: ${errorMessage(err)}`);
}

/** Kills the process and waits for its exit event (3 s max), leaving the pane in the exited state. */
export const stopPane = (id: string): Promise<void> => once(stopping, id, () => doStop(id));

async function doStop(id: string): Promise<void> {
  const rt = runtimes.get(id);
  const pane = useDeck.getState().panes[id];
  if (!rt || !pane || pane.exited) return;
  const exited = new Promise<void>((resolve) => {
    rt.onExit = resolve;
    setTimeout(resolve, 3000);
  });
  await killPane(id).catch((err) => console.warn("kill_pane failed", err));
  await exited;
  rt.onExit = null;
  deck.updatePane(id, (p) => ({ ...p, exited: true }));
}

export const restartPane = (id: string): Promise<void> => once(restarting, id, () => doRestart(id));

async function doRestart(id: string): Promise<void> {
  const rt = runtimes.get(id);
  if (!rt) return;
  await stopPane(id);
  if (runtimes.get(id) !== rt) return; // closed meanwhile
  rt.buffer.clear();
  deck.updatePane(id, (p) => ({
    ...p,
    status: "idle",
    lastAction: null,
    taskStartedAt: null,
    statusSince: Date.now(),
    exited: false,
    hooked: false,
    generation: p.generation + 1,
  }));
  await spawn(id);
}

export async function closePane(id: string): Promise<void> {
  const rt = runtimes.get(id);
  runtimes.delete(id);
  deck.removePane(id);
  tasks.remove(id);
  if (backendActivePane === id) {
    backendActivePane = null;
    setActivePane(null).catch((err) => console.warn("set_active_pane failed", err));
  }
  for (const unlisten of rt?.unlisten ?? []) unlisten();
  await killPane(id).catch((err) => console.warn("kill_pane failed", err));
}

/** Buffered scrollback of a pane (for detached windows). */
export const paneOutputText = (id: string): string => runtimes.get(id)?.buffer.text() ?? "";

/** Replays buffered output then streams new output. Returns a detach function. */
export function attachOutput(id: string, onData: (data: string) => void): () => void {
  const rt = runtimes.get(id);
  if (!rt) return () => undefined;
  const replay = rt.buffer.text();
  if (replay) onData(replay);
  rt.listeners.add(onData);
  return () => rt.listeners.delete(onData);
}

/** Interrupts the current task (Esc / Ctrl+C); the agent session stays alive. */
export function interruptPane(id: string): void {
  const pane = useDeck.getState().panes[id];
  if (pane && !pane.exited) sendInput(id, interruptKey(pane.agent));
}

const activePaneIds = () => activeTab()?.paneIds ?? [];
export const stopAll = () => Promise.all(activePaneIds().map(stopPane));
export const restartAll = () => Promise.all(activePaneIds().map(restartPane));
export const interruptAll = () => activePaneIds().forEach(interruptPane);

/** Closes a session tab but keeps it (layout + panes) in the archive. */
export function archiveSession(tabId: string): void {
  for (const id of deck.archiveTab(tabId)) void closePane(id);
}

/** Forgets a workspace and kills its agents. Files on disk are never touched. */
export function unlinkWorkspace(workspaceId: string): void {
  for (const id of deck.unlinkWorkspace(workspaceId)) void closePane(id);
}

export function reopenSession(sessionId: string): void {
  const reopened = deck.unarchive(sessionId);
  if (!reopened) return;
  for (const spec of reopened.panes) void createPane(spec, reopened.tabId);
}

export function sendInput(id: string, data: string): void {
  const pane = useDeck.getState().panes[id];
  if (!pane) return;
  if (pane.exited) {
    if (data === "r" || data === "R") void restartPane(id);
    return;
  }
  writeToPane(id, data).catch((err) => console.warn("write_to_pane failed", err));
}

export function reportSize(id: string, cols: number, rows: number): void {
  const rt = runtimes.get(id);
  if (!rt) return;
  rt.sized = true;
  if (rt.redraw) {
    rt.redraw = false;
    rt.cols = cols;
    rt.rows = rows;
    nudgeSize(id, rt);
    return;
  }
  if (rt.cols === cols && rt.rows === rows) return;
  rt.cols = cols;
  rt.rows = rows;
  resizePane(id, cols, rows).catch(() => undefined); // fails harmlessly before spawn completes
}

export function focusPane(id: string): void {
  deck.acknowledge(id); // looking at a finished pane counts as having seen it
  if (useDeck.getState().activePaneId === id) return;
  useDeck.setState({ activePaneId: id });
  backendActivePane = id;
  setActivePane(id).catch((err) => console.warn("set_active_pane failed", err));
}

/** Panes without hooks: working if output arrived recently, idle otherwise. Called every second. */
export function deriveStatuses(now = Date.now()): void {
  for (const pane of Object.values(useDeck.getState().panes)) {
    const rt = runtimes.get(pane.id);
    if (!rt || pane.hooked || pane.exited || (pane.status !== "working" && pane.status !== "idle")) continue;
    const status = now - rt.lastOutputAt < ACTIVITY_WINDOW_MS ? "working" : "idle";
    if (status !== pane.status) deck.setStatus(pane.id, status, null, now);
  }
}

const NOTIFY_TEXT = { done: "finished", waiting: "is waiting for your answer", error: "hit an error" } as const;

/** Subscribes once to hook status events; notifies per the user's notification settings. */
export function listenAgentStatus(): Promise<UnlistenFn> {
  return onAgentStatus((e) => {
    const state = useDeck.getState();
    const prev = state.panes[e.paneId];
    if (!prev) return;
    deck.updatePane(e.paneId, (p) => ({ ...p, hooked: true }));
    tasks.apply(e.paneId, e, prev.taskStartedAt ?? undefined);
    deck.setStatus(e.paneId, e.status, e.lastAction);
    const event = e.status as keyof typeof NOTIFY_TEXT;
    // PermissionRequest has its own, more precise notification (components/Permission/permissions.ts).
    if (e.status === prev.status || !(event in NOTIFY_TEXT) || e.event === "PermissionRequest") return;
    const { notify: prefs, phoneUrl } = useSettings.getState();
    const unseen = !document.hasFocus() || state.activePaneId !== e.paneId;
    if (prefs[event] && (unseen || !prefs.onlyUnseen)) {
      const title = `${AGENTS[prev.agent].name} ${NOTIFY_TEXT[event]}`;
      void notify(title, prev.title, { system: prefs.enabled, sound: prefs.sound, phoneUrl });
    }
  });
}
