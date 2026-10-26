// Voice targeting: "Codex, lance les tests" goes to a Codex pane (prefix stripped) instead of the active pane.
import type { AgentKind } from "./types";

export interface RouteCandidate {
  id: string;
  agent: AgentKind;
  title: string;
}

/** Spoken names per agent, normalized. Includes common mis-transcriptions ("cloud" for Claude). */
export const AGENT_ALIASES: Record<AgentKind, string[]> = {
  claude: ["claude", "claude code", "claud", "clode", "clod", "cloud", "cloude"],
  codex: ["codex", "codecs", "codec", "kodex", "codexe", "code x"],
  gemini: ["gemini", "gemini cli", "gemeni", "jemini", "jiminy", "gemmini"],
  grok: ["grok", "groc", "grock", "groque", "grok build"],
  shell: ["shell", "chell", "terminal", "le shell", "le terminal"],
};

/** Lowercase, accents removed. */
const norm = (s: string) => s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();

const WORD = /[\p{L}\p{N}]+/gu;
const words = (s: string) => norm(s).match(WORD) ?? [];

/**
 * Picks the pane a transcript is addressed to. `candidates` are in preference order (current session, most
 * recently active first). A pane title beats an agent name of the same length; the longest spoken name wins.
 * Returns `paneId: null` and the text untouched when nothing matches (caller uses the active pane).
 */
export function routeTranscript(text: string, candidates: RouteCandidate[]): { paneId: string | null; text: string } {
  // Leading words with their end offsets in the original text, so the prefix is cut from the original.
  const tokens: { word: string; end: number }[] = [];
  for (const m of text.matchAll(WORD)) {
    if (tokens.length === 4) break;
    // Only plain separators between the leading words ("Claude Code", not "Claude. Code").
    const gap = text.slice(tokens.at(-1)?.end ?? 0, m.index);
    if (tokens.length > 0 && !/^[\s'-]*$/.test(gap)) break;
    if (tokens.length === 0 && !/^[\s"'«(]*$/.test(gap)) break;
    tokens.push({ word: norm(m[0]), end: m.index + m[0].length });
  }
  const entries = [
    ...candidates.map((c) => ({ id: c.id, key: words(c.title) })),
    ...candidates.flatMap((c) => AGENT_ALIASES[c.agent].map((alias) => ({ id: c.id, key: alias.split(" ") }))),
  ];
  let best: { id: string; end: number; length: number } | null = null;
  for (const { id, key } of entries) {
    if (key.length === 0 || key.length > tokens.length || (best && key.length <= best.length)) continue;
    if (key.every((w, i) => tokens[i].word === w)) best = { id, end: tokens[key.length - 1].end, length: key.length };
  }
  if (!best) return { paneId: null, text };
  const rest = text.slice(best.end).replace(/^[\s,:;.!?»"')—–-]+/, "");
  // "Claude." alone: nothing to send, keep the transcript as is.
  return rest ? { paneId: best.id, text: rest } : { paneId: null, text };
}
