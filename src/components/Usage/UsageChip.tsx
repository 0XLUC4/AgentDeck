import { useEffect, useState } from "react";
import { paneUsage } from "../../lib/ipc";
import type { PaneUsage } from "../../lib/types";
import type { Pane } from "../../store/deck";
import { paneStartedAt } from "../../store/runtime";
import { usageDetails, usageLabel } from "./format";

const REFRESH_MS = 30_000;

/** Tokens and cost of the pane's current process (Claude Code and Codex transcripts only). */
export function UsageChip({ pane }: { pane: Pane }) {
  const [usage, setUsage] = useState<PaneUsage | null>(null);
  const tracked = pane.agent === "claude" || pane.agent === "codex";
  const { id, cwd, agent, generation, hooked, exited, status } = pane;

  // Reloads on every status change (a turn just ended) and every 30 s while a hooked agent runs.
  // biome-ignore lint/correctness/useExhaustiveDependencies: generation/status are refresh triggers
  useEffect(() => {
    if (!tracked) return;
    let alive = true;
    const load = () => {
      const startedAt = paneStartedAt(id);
      if (startedAt === null) return;
      paneUsage(id, cwd, startedAt, agent).then(
        (u) => alive && setUsage(u),
        (err) => console.warn("pane_usage failed", err),
      );
    };
    load();
    const timer = hooked && !exited ? setInterval(load, REFRESH_MS) : undefined;
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [tracked, id, cwd, agent, generation, hooked, exited, status]);

  if (!tracked || !usage || usage.tokens === 0) return null;
  const models = usage.models.length ? `\nModels: ${usage.models.join(", ")}` : "";
  return (
    <span
      className="shrink-0 font-mono text-xs tabular-nums text-subtle"
      title={`Since the agent started\n${usageDetails(usage)}${models}`}
    >
      {usageLabel(usage)}
    </span>
  );
}
