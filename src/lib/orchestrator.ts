// "Chef d'orchestre": a Claude Code pane driving the other agents of its workspace through AgentDeck's MCP server
// (src-tauri/src/mcp). The backend validates arguments; this side checks scope and acts on panes and stores.
import { create } from "zustand";
import { promptPayload } from "../components/Broadcast/send";
import { activeTab, activeWorkspace, activityKey, deck, type Pane, type Tab, useDeck } from "../store/deck";
import { createPane, interruptPane, paneOutput } from "../store/runtime";
import { useSettings } from "../store/settings";
import { useTasks } from "../store/tasks";
import { mcpRespond, mcpSetPaused, onMcpActivity, onMcpRequest, writeToPane } from "./ipc";
import type { AgentKind, McpActivity, McpRequest } from "./types";

export const ORCHESTRATOR_TITLE = "Chef d'orchestre";
const MAX_LOG = 100;
/** Raw terminal text handed to the backend for read_output (it strips ANSI and keeps the last lines). */
const OUTPUT_TAIL_CHARS = 64 * 1024;
const RESULT_ACTIONS = 30;

export interface ActivityEntry {
  at: number;
  tool: string;
  /** "Tests (codex)" or the args summary. */
  summary: string;
  error: string | null;
}

interface OrchestratorState {
  paused: boolean;
  log: Record<string, ActivityEntry[]>;
}

export const useOrchestrator = create<OrchestratorState>(() => ({ paused: false, log: {} }));

export async function setOrchestratorPaused(paused: boolean): Promise<void> {
  try {
    await mcpSetPaused(paused);
    useOrchestrator.setState({ paused });
  } catch (err) {
    deck.toast(`Impossible de changer la pause de l'orchestrateur : ${errorMessage(err)}`);
  }
}

const errorMessage = (err: unknown) => (err instanceof Error ? err.message : String(err));

/** Starts a "Chef d'orchestre" Claude pane in the active session. */
export function createOrchestrator(): void {
  const cwd = activeWorkspace().path || useDeck.getState().lastCwd;
  const tab = activeTab();
  if (!cwd || !tab) {
    deck.toast("Liez d'abord un workspace : le chef d'orchestre travaille dans son dossier.");
    return;
  }
  void createPane(
    {
      id: crypto.randomUUID(),
      agent: "claude",
      title: ORCHESTRATOR_TITLE,
      cwd,
      args: [],
      worktree: false,
      completedCount: 0,
      fontSize: useSettings.getState().fontSize,
      orchestrator: true,
    },
    tab.id,
  );
}

const paneName = (pane: Pane | undefined, id: string) => (pane ? `${pane.title} (${pane.agent})` : id.slice(0, 8));

/** One-line, human summary of a tool call for the activity log. */
export function summarizeCall(tool: string, args: Record<string, unknown>, panes: Record<string, Pane>): string {
  const target = typeof args.paneId === "string" ? paneName(panes[args.paneId], args.paneId) : "";
  switch (tool) {
    case "spawn_agent":
      return `${String(args.agent)} « ${String(args.title)} »${args.worktree ? " · worktree" : ""}`;
    case "send_prompt": {
      const text = String(args.text ?? "").replace(/\s+/g, " ");
      return `${target} : ${text.length > 60 ? `${text.slice(0, 60)}…` : text}`;
    }
    case "wait_for": {
      const ids = Array.isArray(args.paneIds) ? args.paneIds.map(String) : [];
      return `${ids.map((id) => paneName(panes[id], id)).join(", ")} → ${String(args.until)}`;
    }
    default:
      return target;
  }
}

interface Scope {
  orchestrator: Pane;
  tab: Tab;
  /** Folder new agents must stay in. */
  root: string;
  panes: { pane: Pane; tab: Tab }[];
}

/** The orchestrator's workspace: every pane of every session of it, except the orchestrator itself. */
function scopeOf(orchestratorId: string): Scope {
  const { panes, tabs, workspaces } = useDeck.getState();
  const orchestrator = panes[orchestratorId];
  const tab = tabs.find((t) => t.paneIds.includes(orchestratorId));
  if (!orchestrator?.orchestrator || !tab) throw new Error("Chef d'orchestre introuvable");
  const workspace = workspaces.find((w) => w.id === tab.workspaceId);
  return {
    orchestrator,
    tab,
    root: workspace?.path || orchestrator.cwd,
    panes: tabs
      .filter((t) => t.workspaceId === tab.workspaceId)
      .flatMap((t) =>
        t.paneIds.flatMap((id) => (panes[id] && id !== orchestratorId ? [{ pane: panes[id], tab: t }] : [])),
      ),
  };
}

function target(scope: Scope, paneId: string): Pane {
  if (paneId === scope.orchestrator.id) throw new Error("Tu ne peux pas te piloter toi-même");
  const found = scope.panes.find((p) => p.pane.id === paneId)?.pane;
  if (!found) throw new Error(`Agent ${paneId} introuvable dans ton workspace (voir list_agents)`);
  return found;
}

/** Permission prompts are the user's to answer: no keystrokes go to a waiting agent. */
function assertCanType(pane: Pane): void {
  if (pane.exited) throw new Error(`${pane.title} est arrêté`);
  if (pane.status === "waiting")
    throw new Error(`${pane.title} attend une réponse de l'utilisateur : réessaie plus tard`);
}

const normalize = (path: string) =>
  path
    .replace(/[\\/]+/g, "/")
    .replace(/\/$/, "")
    .toLowerCase();

/** Absolute folder for `cwd` (relative to `root` when not absolute); throws when it leaves `root`. */
export function resolveInside(root: string, cwd: string | null | undefined): string {
  if (!cwd) return root;
  if (cwd.split(/[\\/]/).includes("..")) throw new Error("cwd ne peut pas contenir ..");
  const absolute = /^([a-zA-Z]:)?[\\/]/.test(cwd);
  const sep = root.includes("\\") ? "\\" : "/";
  const full = absolute ? cwd : `${root.replace(/[\\/]+$/, "")}${sep}${cwd}`;
  const base = normalize(root);
  const path = normalize(full);
  if (path !== base && !path.startsWith(`${base}/`)) throw new Error(`cwd doit rester dans ${root}`);
  return full;
}

const str = (v: unknown) => (typeof v === "string" ? v : "");

/** Runs one MCP tool for `orchestratorId`. Throws an Error whose message goes back to the orchestrator. */
export async function handleMcpRequest(req: Pick<McpRequest, "orchestratorId" | "tool" | "args">): Promise<unknown> {
  const scope = scopeOf(req.orchestratorId);
  const args = req.args;
  const now = Date.now();
  switch (req.tool) {
    case "list_agents":
      return scope.panes.map(({ pane, tab }) => ({
        paneId: pane.id,
        agent: pane.agent,
        title: pane.title,
        status: activityKey(pane),
        lastAction: pane.lastAction,
        session: tab.name,
        cwd: pane.cwd,
      }));
    case "spawn_agent": {
      const id = crypto.randomUUID();
      await createPane(
        {
          id,
          agent: str(args.agent) as AgentKind,
          title: str(args.title),
          cwd: resolveInside(scope.root, str(args.cwd)),
          args: Array.isArray(args.args) ? args.args.map(String) : [],
          worktree: args.worktree === true,
          completedCount: 0,
          fontSize: useSettings.getState().fontSize,
        },
        scope.tab.id,
      );
      const pane = useDeck.getState().panes[id];
      if (!pane) throw new Error("L'agent a été fermé pendant son lancement");
      if (pane.status === "error") throw new Error(`Échec du lancement : ${pane.lastAction ?? "erreur inconnue"}`);
      return { paneId: id };
    }
    case "send_prompt": {
      const pane = target(scope, str(args.paneId));
      assertCanType(pane);
      const payload = promptPayload(str(args.text));
      // promptPayload always ends with the submitting \r.
      await writeToPane(pane.id, args.submit === false ? payload.slice(0, -1) : payload);
      return { ok: true };
    }
    case "get_status": {
      const pane = target(scope, str(args.paneId));
      return {
        paneId: pane.id,
        status: activityKey(pane),
        lastAction: pane.lastAction,
        timeInStatusSec: Math.round((now - pane.statusSince) / 1000),
      };
    }
    case "get_result": {
      const pane = target(scope, str(args.paneId));
      const log = useTasks.getState()[pane.id];
      const task = log?.history[0];
      return {
        paneId: pane.id,
        running: Boolean(log?.current),
        result: task
          ? {
              prompt: task.prompt,
              summary: task.summary,
              status: task.status,
              actions: task.actions.slice(-RESULT_ACTIONS).map((a) => a.text),
              durationSec: task.endedAt ? Math.round((task.endedAt - task.startedAt) / 1000) : null,
            }
          : null,
      };
    }
    case "wait_for": {
      const ids = Array.isArray(args.paneIds) ? args.paneIds.map(String) : [];
      return { statuses: Object.fromEntries(ids.map((id) => [id, target(scope, id).status])) };
    }
    case "read_output":
      return { raw: paneOutput(target(scope, str(args.paneId)).id).slice(-OUTPUT_TAIL_CHARS) };
    case "interrupt": {
      const pane = target(scope, str(args.paneId));
      assertCanType(pane);
      interruptPane(pane.id);
      return { ok: true };
    }
    default:
      throw new Error(`Outil inconnu : ${req.tool}`);
  }
}

function record(e: McpActivity): void {
  const entry: ActivityEntry = {
    at: Date.now(),
    tool: e.tool,
    summary: summarizeCall(e.tool, e.args, useDeck.getState().panes),
    error: e.error,
  };
  const { log } = useOrchestrator.getState();
  const list = [entry, ...(log[e.orchestratorId] ?? [])].slice(0, MAX_LOG);
  useOrchestrator.setState({ log: { ...log, [e.orchestratorId]: list } });
}

/** Subscribes once (app start) to MCP requests and activity from the backend. */
export function listenOrchestrator(): Promise<unknown> {
  return Promise.all([
    onMcpRequest((req) => {
      handleMcpRequest(req)
        .then((result) => mcpRespond(req.requestId, result ?? null, null))
        .catch((err) => mcpRespond(req.requestId, null, errorMessage(err)))
        .catch((err) => console.warn("mcp_respond failed", err));
    }),
    onMcpActivity(record),
  ]);
}
