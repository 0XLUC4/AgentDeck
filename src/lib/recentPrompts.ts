// Prompts sent to agents (typed in a terminal, broadcast or dictated), newest first, to reuse them.
import { create } from "zustand";

const KEY = "recent-prompts";
const MAX = 30;
/** Longer prompts are pasted documents, not something worth re-sending from a list. */
const MAX_LENGTH = 2000;

function load(): string[] {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(saved) ? saved.filter((p): p is string => typeof p === "string").slice(0, MAX) : [];
  } catch {
    return [];
  }
}

export const useRecentPrompts = create<string[]>(load);

export function rememberPrompt(prompt: string): void {
  const text = prompt.trim();
  if (!text || text.length > MAX_LENGTH) return;
  const next = [text, ...useRecentPrompts.getState().filter((p) => p !== text)].slice(0, MAX);
  useRecentPrompts.setState(next, true);
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // Storage unavailable: the list just won't survive a restart.
  }
}
