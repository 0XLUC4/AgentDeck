// Cost limits (Settings > Notifications): warns once per day / per agent run when the estimated cost
// crosses the limit and, if asked, interrupts the agents and pauses the orchestrator.
import { deck, type Pane, useDeck } from "../store/deck";
import { interruptPane, paneStartedAt } from "../store/runtime";
import { type BudgetSettings, useSettings } from "../store/settings";
import { paneUsage, usageSummary } from "./ipc";
import { notify } from "./notify";
import { setOrchestratorPaused } from "./orchestrator";

const CHECK_MS = 60_000;
const TRACKED = new Set(["claude", "codex"]);

/** Limits already reported: "day:2026-10-27", "pane:<id>:<generation>". */
const warned = new Set<string>();

export const overLimit = (cost: number, limit: number) => limit > 0 && cost >= limit;

const today = () => new Date().toLocaleDateString("sv"); // YYYY-MM-DD, local time

async function hit(message: string, budget: BudgetSettings, panes: Pane[]): Promise<void> {
  const { notify: prefs, phoneUrl } = useSettings.getState();
  deck.toast(message);
  void notify("AgentDeck budget", message, { system: prefs.enabled, sound: prefs.sound, phoneUrl });
  if (!budget.interrupt) return;
  for (const pane of panes) if (pane.status === "working") interruptPane(pane.id);
  await setOrchestratorPaused(true);
}

async function check(): Promise<void> {
  const { budget } = useSettings.getState();
  const panes = Object.values(useDeck.getState().panes).filter((p) => TRACKED.has(p.agent) && !p.exited);
  const dayKey = `day:${today()}`;
  if (budget.dailyUsd > 0 && !warned.has(dayKey)) {
    const { total } = await usageSummary("today");
    if (overLimit(total.cost, budget.dailyUsd)) {
      warned.add(dayKey);
      await hit(`Today's agents cost about $${total.cost.toFixed(2)} (limit $${budget.dailyUsd}).`, budget, panes);
    }
  }
  if (budget.paneUsd <= 0) return;
  for (const pane of panes) {
    const key = `pane:${pane.id}:${pane.generation}`;
    const startedAt = paneStartedAt(pane.id);
    if (warned.has(key) || startedAt === null) continue;
    const usage = await paneUsage(pane.id, pane.cwd, startedAt, pane.agent);
    if (!overLimit(usage.cost, budget.paneUsd)) continue;
    warned.add(key);
    await hit(`${pane.title} cost about $${usage.cost.toFixed(2)} (limit $${budget.paneUsd} per agent).`, budget, [
      pane,
    ]);
  }
}

let started = false;

/** Starts the periodic check once (module-level call from App). */
export function startBudgetWatch(): void {
  if (started) return;
  started = true;
  setInterval(() => {
    check().catch((err) => console.warn("budget check failed", err));
  }, CHECK_MS);
}
