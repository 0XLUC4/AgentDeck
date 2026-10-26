import { Hexagon, type LucideIcon, SquareTerminal } from "lucide-react";
import { siClaude, siGooglegemini } from "simple-icons";
import type { AgentKind } from "./types";

export interface AgentMeta {
  name: string;
  /** Brand mark: an SVG path (24x24 viewBox) from simple-icons, or a lucide icon when no mark is available. */
  mark: string | LucideIcon;
  /** CSS color, defined in index.css. */
  color: string;
}

export const AGENTS: Record<AgentKind, AgentMeta> = {
  claude: { name: "Claude Code", mark: siClaude.path, color: "var(--color-agent-claude)" },
  // shortcut: simple-icons v16 has no OpenAI mark; neutral glyph until an official SVG is added (see assets/agents/README.md).
  codex: { name: "Codex", mark: Hexagon, color: "var(--color-agent-codex)" },
  gemini: { name: "Gemini CLI", mark: siGooglegemini.path, color: "var(--color-agent-gemini)" },
  shell: { name: "Shell", mark: SquareTerminal, color: "var(--color-agent-shell)" },
};

export const AGENT_KINDS = Object.keys(AGENTS) as AgentKind[];

/** Key that interrupts the current task without killing the session: Esc for agents, Ctrl+C for a shell. */
export const interruptKey = (kind: AgentKind) => (kind === "shell" ? "\x03" : "\x1b");
