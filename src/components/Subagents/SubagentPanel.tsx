// Read-only viewer of one of a Claude pane's internal subagents: what it does, live, and its final answer.
import type { IDockviewPanelProps } from "dockview-react";
import { X } from "lucide-react";
import { useEffect, useRef } from "react";
import { formatElapsed, useNow } from "../../lib/clock";
import { useDeck } from "../../store/deck";
import { IconButton } from "../IconButton/IconButton";
import { StatusDot } from "../StatusBadge/StatusBadge";
import { dismiss, useSubagents } from "./subagents";

const time = (at: number) => new Date(at).toLocaleTimeString([], { hour12: false });

const useSubagent = (paneId: string, agentId: string) => useSubagents((s) => s[paneId]?.find((a) => a.id === agentId));

export function SubagentPanel({ params }: IDockviewPanelProps<{ paneId: string; agentId: string }>) {
  const agent = useSubagent(params.paneId, params.agentId);
  const body = useRef<HTMLDivElement>(null);
  const count = agent?.actions.length ?? 0;

  // biome-ignore lint/correctness/useExhaustiveDependencies: follow the output as new lines arrive
  useEffect(() => {
    body.current?.scrollTo({ top: body.current.scrollHeight });
  }, [count, agent?.summary]);

  return (
    <div
      ref={body}
      className="scroll-thin h-full overflow-y-auto bg-base px-3 py-2 font-mono text-xs leading-5 text-muted"
      role="log"
      aria-label={`Sub-agent ${agent?.type ?? ""} activity`}
    >
      {count === 0 && !agent?.endedAt && <p className="text-subtle">Starting…</p>}
      {agent?.actions.map((a, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: append-only list, the same action text can repeat
        <p key={i} className="truncate" title={a.text}>
          <span className="text-subtle">{time(a.at)} </span>
          <span className="text-fg">{a.text}</span>
        </p>
      ))}
      {agent?.endedAt && (
        <p className="mt-2 whitespace-pre-wrap border-t border-line pt-2 text-fg">{agent.summary || "Done."}</p>
      )}
    </div>
  );
}

/** Dockview tab of a subagent viewer: type, parent, status, elapsed time, dismiss. */
export function SubagentHeader({ paneId, agentId }: { paneId: string; agentId: string }) {
  const agent = useSubagent(paneId, agentId);
  const parent = useDeck((s) => s.panes[paneId]?.title);
  const now = useNow();
  if (!agent) return null;
  return (
    <header className="flex h-full min-w-0 items-center gap-2 px-2.5 text-[13px] text-fg">
      <StatusDot status={agent.endedAt ? "done" : "working"} />
      <span className="min-w-0 flex-1 truncate">
        <span className="font-medium">↳ {agent.type}</span>
        {parent && <span className="text-subtle"> · {parent}</span>}
      </span>
      <span className="shrink-0 font-mono text-xs tabular-nums text-muted">
        {formatElapsed((agent.endedAt ?? now) - agent.startedAt)}
      </span>
      <IconButton
        icon={X}
        size="sm"
        label={`Dismiss sub-agent ${agent.type}`}
        onClick={() => dismiss(paneId, agentId)}
      />
    </header>
  );
}
