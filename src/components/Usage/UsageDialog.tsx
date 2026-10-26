import { Gauge, TriangleAlert } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { AGENTS } from "../../lib/agents";
import { closeDialog, openDialog, useDialogs } from "../../lib/dialogs";
import { usageSummary } from "../../lib/ipc";
import type { UsageKeyed, UsageRange, UsageSummary, UsageTotals } from "../../lib/types";
import { folderName, useDeck, type Workspace } from "../../store/deck";
import { AgentLogo } from "../AgentLogo/AgentLogo";
import { IconButton } from "../IconButton/IconButton";
import { formatCost, formatTokens, isAgentKind, usageDetails } from "./format";
import { Modal, Segmented } from "./Modal";

const RANGES: { id: UsageRange; label: string }[] = [
  { id: "today", label: "Today" },
  { id: "7d", label: "7 days" },
  { id: "30d", label: "30 days" },
];

const norm = (p: string) =>
  p
    .replace(/[\\/]+$/, "")
    .replace(/\//g, "\\")
    .toLowerCase();
const emptyTotals = (): UsageTotals => ({
  input: 0,
  output: 0,
  cacheWrite: 0,
  cacheRead: 0,
  tokens: 0,
  cost: 0,
  unpricedTokens: 0,
});

/** Folds per-cwd rows into linked workspaces (cwd inside the workspace folder), else the folder name. */
export function byWorkspace(rows: UsageKeyed[], workspaces: Workspace[]): UsageKeyed[] {
  const merged = new Map<string, UsageKeyed>();
  for (const row of rows) {
    const cwd = norm(row.key);
    const ws = workspaces.find((w) => w.path && (cwd === norm(w.path) || cwd.startsWith(`${norm(w.path)}\\`)));
    const key = ws?.name ?? (folderName(row.key) || "Unknown");
    const acc = merged.get(key) ?? { key, ...emptyTotals() };
    for (const f of ["input", "output", "cacheWrite", "cacheRead", "tokens", "cost", "unpricedTokens"] as const) {
      acc[f] += row[f];
    }
    merged.set(key, acc);
  }
  return [...merged.values()].sort((a, b) => b.cost - a.cost || b.tokens - a.tokens);
}

/** Bar length: cost when anything is priced, else tokens. */
const measure = (rows: UsageTotals[]) =>
  rows.some((r) => r.cost > 0) ? (r: UsageTotals) => r.cost : (r: UsageTotals) => r.tokens;

function Figure({ totals }: { totals: UsageTotals }) {
  return (
    <span className="shrink-0 text-right font-mono text-xs tabular-nums text-muted" title={usageDetails(totals)}>
      {formatTokens(totals.tokens)} tok · <span className="text-fg">{formatCost(totals.cost)}</span>
    </span>
  );
}

function BarList({ title, rows, icon }: { title: string; rows: UsageKeyed[]; icon?: (key: string) => ReactNode }) {
  if (rows.length === 0) return null;
  const value = measure(rows);
  const max = Math.max(...rows.map(value), 1e-9);
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-xs font-medium text-subtle">{title}</h3>
      <ul className="flex flex-col gap-2">
        {rows.slice(0, 10).map((row) => (
          <li key={row.key} className="flex flex-col gap-1">
            <div className="flex items-center gap-2">
              {icon?.(row.key)}
              <span className="min-w-0 flex-1 truncate text-fg" title={row.key}>
                {isAgentKind(row.key) ? AGENTS[row.key].name : row.key}
              </span>
              <Figure totals={row} />
            </div>
            <div className="h-1 rounded-full bg-base">
              <div className="h-1 rounded-full bg-muted" style={{ width: `${(value(row) / max) * 100}%` }} />
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}

function DayChart({ days }: { days: UsageKeyed[] }) {
  const value = measure(days);
  const max = Math.max(...days.map(value), 1e-9);
  const label = (key: string) => {
    const [, m, d] = key.split("-");
    return `${m}/${d}`;
  };
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-xs font-medium text-subtle">By day</h3>
      <div className="flex h-32 items-end gap-1" role="img" aria-label="Usage by day">
        {days.map((day) => (
          <div
            key={day.key}
            className="group flex h-full min-w-0 flex-1 flex-col justify-end"
            title={`${label(day.key)}\n${usageDetails(day)}`}
          >
            <div
              className="min-h-px rounded-t-sm bg-muted transition-colors duration-150 group-hover:bg-fg"
              style={{ height: `${(value(day) / max) * 100}%` }}
            />
          </div>
        ))}
      </div>
      <div className="flex justify-between font-mono text-[11px] text-subtle">
        <span>{label(days[0].key)}</span>
        <span>{label(days[days.length - 1].key)}</span>
      </div>
    </section>
  );
}

export function UsageDialog({ onClose }: { onClose: () => void }) {
  const [range, setRange] = useState<UsageRange>("today");
  const [data, setData] = useState<UsageSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const workspaces = useDeck((s) => s.workspaces);

  useEffect(() => {
    let alive = true;
    setData(null);
    setError(null);
    usageSummary(range).then(
      (d) => alive && setData(d),
      (err) => alive && setError(String(err)),
    );
    return () => {
      alive = false;
    };
  }, [range]);

  return (
    <Modal title="Usage" onClose={onClose}>
      <div className="flex flex-col gap-5">
        <div className="flex items-center justify-between gap-3">
          <Segmented label="Period" options={RANGES} value={range} onChange={setRange} />
          <span className="text-xs text-subtle">Claude Code and Codex · cost estimated at API pricing</span>
        </div>
        {error ? (
          <p role="alert" className="flex items-center gap-2 text-xs text-bad">
            <TriangleAlert size={14} strokeWidth={1.5} aria-hidden="true" />
            Could not read usage: {error}
          </p>
        ) : !data ? (
          <p className="py-8 text-center text-muted" aria-busy="true">
            Reading transcripts…
          </p>
        ) : (
          <>
            <div className="flex items-baseline gap-3">
              <span className="text-2xl font-semibold tabular-nums">{formatCost(data.total.cost)}</span>
              <span className="font-mono text-xs text-muted" title={usageDetails(data.total)}>
                {formatTokens(data.total.tokens)} tokens · {formatTokens(data.total.output)} output
              </span>
            </div>
            {data.total.unpricedTokens > 0 && (
              <p className="text-xs text-subtle">
                {formatTokens(data.total.unpricedTokens)} tokens from models with no known price are not included in the
                cost.
              </p>
            )}
            {data.total.tokens === 0 ? (
              <p className="py-6 text-center text-muted">No usage in this period</p>
            ) : (
              <>
                {data.byDay.length > 1 && <DayChart days={data.byDay} />}
                <BarList
                  title="By agent"
                  rows={data.byAgent}
                  icon={(key) => (isAgentKind(key) ? <AgentLogo kind={key} size={14} /> : null)}
                />
                <BarList title="By workspace" rows={byWorkspace(data.byWorkspaceCwd, workspaces)} />
              </>
            )}
          </>
        )}
      </div>
    </Modal>
  );
}

/** Title bar button + dialog. */
export function UsageButton() {
  const open = useDialogs((s) => s.open === "usage");
  return (
    <>
      <IconButton icon={Gauge} label="Usage" hint="Usage: tokens and cost" onClick={() => openDialog("usage")} />
      {open && <UsageDialog onClose={closeDialog} />}
    </>
  );
}
