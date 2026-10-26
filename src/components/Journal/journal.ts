// Daily journal: finished tasks (persisted per day by the backend) grouped by workspace > agent,
// plus today's commits and usage, rendered as Markdown. Pure functions, no IPC.

import { AGENT_KINDS, AGENTS } from "../../lib/agents";
import { formatSince } from "../../lib/clock";
import type { AgentKind, GitCommitInfo, PaneStatus, UsageTotals } from "../../lib/types";
import type { Task } from "../../store/tasks";
import { formatCost, formatTokens, isAgentKind } from "../Usage/format";

const MAX_ACTIONS = 30;

/** What is saved for one finished (or interrupted) task. */
export interface JournalEntry {
  paneId: string;
  title: string;
  agent: AgentKind;
  cwd: string;
  workspaceName: string;
  /** "" when the pane belongs to the default workspace. */
  workspacePath: string;
  prompt: string | null;
  summary: string | null;
  actions: string[];
  startedAt: number;
  endedAt: number;
  status: PaneStatus;
}

export function toJournalEntry(
  pane: { id: string; title: string; agent: AgentKind; cwd: string },
  workspace: { name: string; path: string } | undefined,
  task: Task,
): JournalEntry {
  return {
    paneId: pane.id,
    title: pane.title,
    agent: pane.agent,
    cwd: pane.cwd,
    workspaceName: workspace?.name ?? "",
    workspacePath: workspace?.path ?? "",
    prompt: task.prompt,
    summary: task.summary,
    actions: task.actions.slice(-MAX_ACTIONS).map((a) => a.text),
    startedAt: task.startedAt,
    endedAt: task.endedAt ?? Date.now(),
    status: task.status,
  };
}

/** Keeps only well-formed entries from a day file (it is plain JSON on disk). */
export function parseEntries(raw: unknown[]): JournalEntry[] {
  return raw.filter(
    (e): e is JournalEntry =>
      typeof e === "object" &&
      e !== null &&
      typeof (e as JournalEntry).startedAt === "number" &&
      typeof (e as JournalEntry).endedAt === "number" &&
      isAgentKind(String((e as JournalEntry).agent)) &&
      Array.isArray((e as JournalEntry).actions),
  );
}

export interface WorkspaceCommits {
  name: string;
  path: string;
  commits: GitCommitInfo[];
}

export interface JournalGroup {
  /** Workspace path, or the pane cwd when the workspace has none. */
  key: string;
  name: string;
  agents: { agent: AgentKind; tasks: JournalEntry[] }[];
  commits: GitCommitInfo[];
}

const norm = (p: string) =>
  p
    .replace(/[\\/]+$/, "")
    .replace(/\//g, "\\")
    .toLowerCase();
const lastSegment = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;

/** Workspace > agent (fixed agent order) > tasks by start time. Workspaces with only commits are kept. */
export function groupJournal(entries: JournalEntry[], commits: WorkspaceCommits[] = []): JournalGroup[] {
  const groups = new Map<string, JournalGroup>();
  const group = (path: string, name: string) => {
    const key = norm(path);
    let g = groups.get(key);
    if (!g) {
      g = { key: path, name: name || lastSegment(path) || "Sans workspace", agents: [], commits: [] };
      groups.set(key, g);
    }
    return g;
  };
  for (const e of [...entries].sort((a, b) => a.startedAt - b.startedAt)) {
    const g = group(e.workspacePath || e.cwd, e.workspaceName);
    let slot = g.agents.find((a) => a.agent === e.agent);
    if (!slot) {
      slot = { agent: e.agent, tasks: [] };
      g.agents.push(slot);
    }
    slot.tasks.push(e);
  }
  for (const c of commits) if (c.commits.length) group(c.path, c.name).commits.push(...c.commits);
  for (const g of groups.values()) g.agents.sort((a, b) => AGENT_KINDS.indexOf(a.agent) - AGENT_KINDS.indexOf(b.agent));
  return [...groups.values()];
}

const oneLine = (text: string, max: number) => {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat;
};

export const clockTime = (ms: number) =>
  new Date(ms).toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit" });

export const STATUS_LABEL: Partial<Record<PaneStatus, string>> = {
  error: "erreur",
  working: "interrompue",
  waiting: "interrompue",
};

export function taskLine(e: JournalEntry): string {
  const status = STATUS_LABEL[e.status];
  const head = `${clockTime(e.startedAt)} → ${clockTime(e.endedAt)} (${formatSince(e.endedAt - e.startedAt)})`;
  return `${head}${status ? ` [${status}]` : ""} — ${e.prompt ? oneLine(e.prompt, 200) : "(tâche sans demande)"}`;
}

/** Local YYYY-MM-DD, the backend's day-file name. */
export const dayKey = (d = new Date()) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;

export function journalMarkdown(
  date: string,
  groups: JournalGroup[],
  usage: { total: UsageTotals; byAgent: { key: string; tokens: number; cost: number }[] } | null,
): string {
  const [y, m, d] = date.split("-");
  const out = [`# Journal du ${d}/${m}/${y}`, ""];
  if (usage && usage.total.tokens > 0) {
    const agents = usage.byAgent
      .map((a) => `${isAgentKind(a.key) ? AGENTS[a.key].name : a.key} ${formatCost(a.cost)}`)
      .join(", ");
    out.push(
      `**Consommation** : ${formatTokens(usage.total.tokens)} tokens · ${formatCost(usage.total.cost)} (${agents})`,
      "",
    );
  }
  if (groups.length === 0) out.push("_Aucune tâche ni commit aujourd'hui._", "");
  for (const g of groups) {
    out.push(`## ${g.name}`, "");
    if (g.key) out.push(`\`${g.key}\``, "");
    for (const { agent, tasks } of g.agents) {
      out.push(`### ${AGENTS[agent].name} — ${tasks.length} tâche(s)`, "");
      for (const t of tasks) {
        out.push(`- ${taskLine(t)}`);
        if (t.summary) out.push(`  - Résultat : ${oneLine(t.summary, 400)}`);
        if (t.actions.length)
          out.push(
            `  - ${t.actions.length} action(s) : ${t.actions
              .slice(-3)
              .map((a) => oneLine(a, 80))
              .join(" · ")}`,
          );
      }
      out.push("");
    }
    if (g.commits.length) {
      out.push(`### Commits — ${g.commits.length}`, "");
      for (const c of g.commits) out.push(`- \`${c.hash}\` ${clockTime(Date.parse(c.time))} ${c.subject}`);
      out.push("");
    }
  }
  return `${out.join("\n").trimEnd()}\n`;
}
