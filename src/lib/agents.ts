import claudeLogo from "../assets/agents/claude.svg";
import codexLogo from "../assets/agents/codex.svg";
import geminiLogo from "../assets/agents/gemini.svg";
import shellLogo from "../assets/agents/shell.svg";
import type { AgentKind } from "./types";

export interface AgentMeta {
  name: string;
  logo: string;
  /** CSS color, defined in index.css. */
  color: string;
}

export const AGENTS: Record<AgentKind, AgentMeta> = {
  claude: { name: "Claude Code", logo: claudeLogo, color: "var(--color-agent-claude)" },
  codex: { name: "Codex", logo: codexLogo, color: "var(--color-agent-codex)" },
  gemini: { name: "Gemini CLI", logo: geminiLogo, color: "var(--color-agent-gemini)" },
  shell: { name: "Shell", logo: shellLogo, color: "var(--color-agent-shell)" },
};

export const AGENT_KINDS = Object.keys(AGENTS) as AgentKind[];

/** Key that interrupts the current task without killing the session: Esc for agents, Ctrl+C for a shell. */
export const interruptKey = (kind: AgentKind) => (kind === "shell" ? "\x03" : "\x1b");
