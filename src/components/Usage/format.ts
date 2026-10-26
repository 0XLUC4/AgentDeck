// Number formatting for usage figures (French locale).
import { AGENT_KINDS } from "../../lib/agents";
import type { AgentKind, UsageTotals } from "../../lib/types";

const decimal = (n: number, digits: number) =>
  n.toLocaleString("fr-FR", { minimumFractionDigits: 0, maximumFractionDigits: digits });

/** 950 -> "950", 12 345 -> "12k", 1 234 567 -> "1,2M", 5,4e9 -> "5,4Md". */
export function formatTokens(n: number): string {
  if (n < 1000) return String(Math.round(n));
  if (n < 1e6) return `${decimal(n / 1e3, n < 1e4 ? 1 : 0)}k`;
  if (n < 1e9) return `${decimal(n / 1e6, n < 1e8 ? 1 : 0)}M`;
  return `${decimal(n / 1e9, 1)}Md`;
}

/** "0,31 $", "2 061,04 $". */
export const formatCost = (usd: number) =>
  `${usd.toLocaleString("fr-FR", { minimumFractionDigits: 2, maximumFractionDigits: 2 })} $`;

/** Chip text: tokens, plus cost when some of it is priced. */
export const usageLabel = (u: UsageTotals) =>
  u.cost > 0 ? `${formatTokens(u.tokens)} tok · ${formatCost(u.cost)}` : `${formatTokens(u.tokens)} tok`;

/** Multi-line tooltip with the breakdown. */
export function usageDetails(u: UsageTotals): string {
  const lines = [
    `Entrée : ${formatTokens(u.input)}`,
    `Sortie : ${formatTokens(u.output)}`,
    `Cache écrit : ${formatTokens(u.cacheWrite)}`,
    `Cache lu : ${formatTokens(u.cacheRead)}`,
    `Coût estimé (tarif API) : ${formatCost(u.cost)}`,
  ];
  if (u.unpricedTokens > 0) lines.push(`${formatTokens(u.unpricedTokens)} tokens d'un modèle sans tarif connu`);
  return lines.join("\n");
}

export const isAgentKind = (v: string): v is AgentKind => (AGENT_KINDS as string[]).includes(v);
