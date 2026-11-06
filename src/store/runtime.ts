// Pane lifecycle: IPC subscriptions, spawn/restart/close, output buffering, status events.
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, UserAttentionType } from "@tauri-apps/api/window";
import { sendPrompt } from "../components/Broadcast/send";
import { AGENTS, interruptKey } from "../lib/agents";
import {
  browserClose,
  browserOpen,
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
import { rememberPrompt } from "../lib/recentPrompts";
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
  /** Spawned with --resume: an exit right after means the conversation is gone, so retry fresh. */
  resumedAt?: number;
  /** PORT given to this agent (settings.basePort + n), kept across restarts. */
  port?: number;
  /** Last hook event: a hooked agent "working" with no event for long is likely stuck. */
  lastHookAt?: number;
  /** Output before this time is a repaint (focus, resize), not activity. */
  repaintUntil?: number;
}

/** A TUI repaints right after a click, a resize or a keystroke: that output must not light the pane up as busy. */
const REPAINT_MS = 600;

const runtimes = new Map<string, Runtime>();

/** Spawn time of the pane's current process, or null before it starts. */
export const paneStartedAt = (id: string): number | null => runtimes.get(id)?.startedAt ?? null;
/** Buffered raw terminal output of a pane ("" when unknown). */
/** Last `chars` characters of a pane's buffered output. */
export const paneOutputTail = (id: string, chars: number): string => runtimes.get(id)?.buffer.tail(chars) ?? "";
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
    port: spec.port, // an adopted process still runs with it
  };
  runtimes.set(spec.id, rt);
  try {
    rt.unlisten = await Promise.all([
      onPaneOutput(spec.id, (data) => {
        rt.buffer.push(data);
        const now = Date.now();
        if (!rt.repaintUntil || now > rt.repaintUntil) rt.lastOutputAt = now;
        for (const fn of rt.listeners) fn(data);
      }),
      onPaneExit(spec.id, () => {
        const pane = useDeck.getState().panes[spec.id];
        if (!rt.onExit && pane && rt.resumedAt && Date.now() - rt.resumedAt < RESUME_GRACE_MS) {
          retryFresh(spec.id, rt);
          return;
        }
        // The agent quit by itself (Ctrl+C, /exit): close its pane instead of leaving a dead one in the layout.
        if (!rt.onExit && pane && !pane.exited) {
          deck.toast(`${pane.title} exited`);
          void closePane(spec.id);
          return;
        }
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
/** Conversation ids are UUIDs; anything else (a value starting with "-" would read as a flag) is ignored. */
const RESUME_ID = /^[A-Za-z0-9][A-Za-z0-9-]{0,63}$/;
/** Claude exits within this window when the conversation to resume no longer exists. */
const RESUME_GRACE_MS = 8000;

/** The resumed conversation is gone: start a fresh one instead of losing the pane. */
function retryFresh(id: string, rt: Runtime): void {
  rt.resumedAt = undefined;
  rt.buffer.clear();
  deck.updatePane(id, (p) => ({ ...p, resumeId: undefined, generation: p.generation + 1 }));
  void spawn(id);
}

/** At most this many agents start at once: a restore with many panes would otherwise spawn them all together. */
const SPAWN_SLOTS = 3;
let freeSlots = SPAWN_SLOTS;
const slotQueue: (() => void)[] = [];

async function withSpawnSlot<T>(job: () => Promise<T>): Promise<T> {
  if (freeSlots > 0) freeSlots--;
  else await new Promise<void>((resolve) => slotQueue.push(resolve));
  try {
    return await job();
  } finally {
    const next = slotQueue.shift();
    if (next) next();
    else freeSlots++;
  }
}

/** Smallest free PORT offset among running agents. */
function freePort(base: number): number {
  const used = new Set([...runtimes.values()].map((r) => r.port));
  let port = base;
  while (used.has(port)) port++;
  return port;
}

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
    const browser = pane.browser && pane.agent === "claude" ? await browserOpen(id) : [];
    const settings = useSettings.getState();
    const resume =
      pane.agent === "claude" && pane.resumeId && settings.resumeSessions && RESUME_ID.test(pane.resumeId)
        ? pane.resumeId
        : null;
    rt.resumedAt = undefined;
    const portTaken = [...runtimes.values()].some((r) => r !== rt && r.port === rt.port);
    if (settings.basePort && (rt.port === undefined || portTaken)) {
      const port = freePort(settings.basePort);
      rt.port = port;
      deck.updatePane(id, (p) => ({ ...p, port }));
    }
    const env: Record<string, string> = {
      ...(mcp ? { AGENTDECK_MCP_SECRET: mcp.secret } : {}),
      ...(settings.basePort && rt.port ? { PORT: String(rt.port) } : {}),
    };
    const spawned = await withSpawnSlot(async () => {
      if (runtimes.get(id) !== rt) return false; // closed while queued
      await spawnPane({
        paneId: id,
        agent: pane.agent,
        cwd: pane.cwd,
        args: [...browser, ...(mcp?.args ?? []), ...(resume ? ["--resume", resume] : []), ...pane.args],
        worktree: pane.worktree,
        cols,
        rows,
        title: pane.title,
        env: Object.keys(env).length > 0 ? env : undefined,
        trust: settings.trustAgentFolders,
      });
      // Measured from the real start: time spent queued for a slot must not eat the grace window.
      if (resume) rt.resumedAt = Date.now();
      return true;
    });
    if (!spawned) return;
    // Closed while spawning: closePane's kill ran before the backend registered the pane.
    if (runtimes.get(id) !== rt) {
      await killPane(id).catch((err) => console.warn("kill_pane failed", err));
      return;
    }
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
    resumeId: undefined, // a restart starts a new conversation
    generation: p.generation + 1,
  }));
  await spawn(id);
}

export async function closePane(id: string): Promise<void> {
  const rt = runtimes.get(id);
  const hadBrowser = useDeck.getState().panes[id]?.browser;
  runtimes.delete(id);
  deck.removePane(id);
  tasks.remove(id);
  stallAlerts.delete(id);
  clearTimeout(rateLimitRetries.get(id));
  rateLimitRetries.delete(id);
  for (const [file, editor] of editors) if (editor.paneId === id) editors.delete(file);
  if (backendActivePane === id) {
    backendActivePane = null;
    setActivePane(null).catch((err) => console.warn("set_active_pane failed", err));
  }
  for (const unlisten of rt?.unlisten ?? []) unlisten();
  if (hadBrowser) browserClose(id).catch((err) => console.warn("browser_close failed", err));
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
  // The echo of what the user types is not the agent working.
  const rt = runtimes.get(id);
  if (rt) rt.repaintUntil = Date.now() + REPAINT_MS;
  writeToPane(id, data).catch((err) => console.warn("write_to_pane failed", err));
}

export function reportSize(id: string, cols: number, rows: number): void {
  const rt = runtimes.get(id);
  if (!rt) return;
  rt.repaintUntil = Date.now() + REPAINT_MS;
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
  const rt = runtimes.get(id);
  if (rt) rt.repaintUntil = Date.now() + REPAINT_MS;
  deck.acknowledge(id); // looking at a finished pane counts as having seen it
  if (useDeck.getState().activePaneId === id) return;
  useDeck.setState({ activePaneId: id });
  backendActivePane = id;
  setActivePane(id).catch((err) => console.warn("set_active_pane failed", err));
}

/**
 * Worth a confirmation before closing: marked working and still printing. An agent interrupted with Esc gets no
 * Stop hook and stays "working" while silent; a busy one keeps animating its spinner.
 */
export function isBusy(id: string, now = Date.now()): boolean {
  const rt = runtimes.get(id);
  return useDeck.getState().panes[id]?.status === "working" && !!rt && now - rt.lastOutputAt < ACTIVITY_WINDOW_MS;
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
    const rt = runtimes.get(e.paneId);
    if (rt) rt.lastHookAt = Date.now();
    const resumeId = e.sessionId ?? undefined;
    deck.updatePane(e.paneId, (p) =>
      p.hooked && (!resumeId || p.resumeId === resumeId)
        ? p
        : { ...p, hooked: true, ...(resumeId ? { resumeId } : {}) },
    );
    if (e.editedFile) checkConflict(e.paneId, e.editedFile);
    if (e.event === "UserPromptSubmit" && e.prompt) rememberPrompt(e.prompt);
    tasks.apply(e.paneId, e, prev.taskStartedAt ?? undefined);
    deck.setStatus(e.paneId, e.status, e.lastAction);
    if (e.status === "error" && e.event === "StopFailure" && /rate.?limit/i.test(e.lastAction ?? "")) {
      scheduleRateLimitRetry(e.paneId);
    }
    // Flash the taskbar button while the window is in the background and an agent needs the user.
    if (e.status === "waiting" && prev.status !== "waiting" && !document.hasFocus()) {
      getCurrentWindow()
        .requestUserAttention(UserAttentionType.Informational)
        .catch(() => undefined);
    }
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

/** Last pane that edited each file (normalized path), to spot two agents editing the same file. */
const editors = new Map<string, { paneId: string; at: number; warnedAt?: number }>();
const CONFLICT_WINDOW_MS = 15 * 60_000;

/** Toasts once when another live agent edited the same file recently (no worktree between them). */
export function checkConflict(paneId: string, file: string, now = Date.now()): void {
  const key = file.replaceAll("\\", "/").toLowerCase();
  const prev = editors.get(key);
  const panes = useDeck.getState().panes;
  let warnedAt = prev?.warnedAt;
  if (
    prev &&
    prev.paneId !== paneId &&
    panes[prev.paneId] &&
    now - prev.at < CONFLICT_WINDOW_MS &&
    (!warnedAt || now - warnedAt > CONFLICT_WINDOW_MS)
  ) {
    const name = key.slice(key.lastIndexOf("/") + 1);
    deck.toast(`${panes[prev.paneId].title} and ${panes[paneId]?.title ?? "another agent"} both edited ${name}`);
    warnedAt = now;
  }
  editors.set(key, { paneId, at: now, warnedAt });
}

/** Alerts already sent, keyed by pane id: the statusSince they were sent for, and how many reminders. */
const stallAlerts = new Map<string, { since: number; count: number }>();

/**
 * Called every second: notifies when a hooked agent has been working with no hook event for
 * `stuckMinutes` (hung command, foreground dev server), and reminds every `remindMinutes` while
 * an agent keeps waiting for an answer.
 */
export function watchAgents(now = Date.now()): void {
  const { stuckMinutes, remindMinutes, notify: prefs, phoneUrl } = useSettings.getState();
  for (const pane of Object.values(useDeck.getState().panes)) {
    const rt = runtimes.get(pane.id);
    if (!rt || pane.exited || !pane.hooked) continue;
    const sent = stallAlerts.get(pane.id);
    const fresh = !sent || sent.since !== pane.statusSince;
    let due: string | null = null;
    let count = fresh ? 0 : sent.count;
    if (pane.status === "working" && stuckMinutes > 0 && count === 0) {
      const quietMs = now - Math.max(rt.lastHookAt ?? 0, pane.statusSince);
      if (quietMs > stuckMinutes * 60_000)
        due = `${AGENTS[pane.agent].name} looks stuck (${stuckMinutes} min without progress)`;
    } else if (pane.status === "waiting" && remindMinutes > 0 && prefs.waiting) {
      const reminders = Math.floor((now - pane.statusSince) / (remindMinutes * 60_000));
      if (reminders > count) {
        due = `${AGENTS[pane.agent].name} is still waiting for you`;
        count = reminders - 1;
      }
    }
    if (!due) continue;
    stallAlerts.set(pane.id, { since: pane.statusSince, count: count + 1 });
    void notify(due, pane.title, { system: prefs.enabled, sound: prefs.sound, phoneUrl });
  }
}

const rateLimitRetries = new Map<string, ReturnType<typeof setTimeout>>();

/** Sends "continue" once the wait is over, if the agent is still stopped on that error. */
function scheduleRateLimitRetry(id: string): void {
  const minutes = useSettings.getState().retryRateLimitMinutes;
  if (minutes <= 0 || rateLimitRetries.has(id)) return;
  const since = useDeck.getState().panes[id]?.statusSince;
  const timer = setTimeout(() => {
    rateLimitRetries.delete(id);
    const pane = useDeck.getState().panes[id];
    if (!pane || pane.exited || pane.status !== "error" || pane.statusSince !== since) return;
    deck.toast(`${pane.title}: continuing after the rate limit`);
    sendPrompt(id, "continue").catch((err) => console.warn("rate limit retry failed", err));
  }, minutes * 60_000);
  rateLimitRetries.set(id, timer);
}
