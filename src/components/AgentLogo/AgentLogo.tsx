import { AGENTS } from "../../lib/agents";
import type { AgentKind } from "../../lib/types";

/** Brand mark of an agent, inline SVG in its brand color. Decorative: the agent name is always shown next to it. */
export function AgentLogo({ kind, size = 16 }: { kind: AgentKind; size?: number }) {
  const { mark: Mark, color } = AGENTS[kind];
  if (typeof Mark !== "string") {
    return <Mark size={size} color={color} strokeWidth={1.5} aria-hidden="true" className="shrink-0" />;
  }
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill={color}
      aria-hidden="true"
      focusable="false"
      className="shrink-0"
    >
      <path d={Mark} />
    </svg>
  );
}
