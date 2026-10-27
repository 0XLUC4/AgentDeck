import {
  CheckCheck,
  ExternalLink,
  GitBranch,
  Maximize2,
  Minimize2,
  Pause,
  Pencil,
  RotateCcw,
  ScanEye,
  SquareArrowDownLeft,
  SquareTerminal,
  X,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { useState } from "react";
import { AGENTS } from "../../lib/agents";
import { formatElapsed, useNow } from "../../lib/clock";
import type { Pane } from "../../store/deck";
import { deck, FONT_SIZE, useDeck } from "../../store/deck";
import { useReviews } from "../../store/review";
import { closePane, focusPane, interruptPane, restartPane } from "../../store/runtime";
import { AgentLogo } from "../AgentLogo/AgentLogo";
import { toggleDetached, useDetached } from "../Detached/host";
import { GitPanel } from "../Git/GitPanel";
import { IconButton } from "../IconButton/IconButton";
import { OrchestratorButton } from "../Orchestrator/OrchestratorButton";
import { QueueButton } from "../Queue/QueueButton";
import { StatusBadge } from "../StatusBadge/StatusBadge";
import { UsageChip } from "../Usage/UsageChip";

interface Props {
  pane: Pane;
  active: boolean;
  maximized: boolean;
  /** Several panes stacked in one dockview group: show only identity, status and close. */
  compact: boolean;
  onToggleMaximize: () => void;
}

/** Closes the pane, asking first if the agent is still working. */
export function confirmClose(pane: Pane) {
  if (pane.status !== "working" || window.confirm(`${AGENTS[pane.agent].name} is still working. Close anyway?`)) {
    void closePane(pane.id);
  }
}

function Elapsed({ since }: { since: number }) {
  const now = useNow();
  return (
    <span className="shrink-0 font-mono text-xs tabular-nums text-muted" title="Time spent on the task">
      {formatElapsed(now - since)}
    </span>
  );
}

// Keeps dockview from treating clicks in the title field as a tab drag.
const stop = (e: { stopPropagation: () => void }) => e.stopPropagation();

function TitleField({ pane, onDone }: { pane: Pane; onDone: () => void }) {
  return (
    <input
      // biome-ignore lint/a11y/noAutofocus: the user just asked to rename this pane
      autoFocus
      defaultValue={pane.title}
      aria-label="Task title"
      className="h-6 w-40 min-w-0 rounded-md border border-line-strong bg-base px-1.5 text-fg outline-none focus:border-accent"
      onPointerDown={stop}
      onMouseDown={stop}
      onBlur={(e) => {
        deck.rename(pane.id, e.target.value.trim() || pane.title);
        onDone();
      }}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") onDone();
      }}
    />
  );
}

/** Pane header, rendered as the dockview tab: drag it to move the pane, double-click to maximize. */
export function PaneHeader({ pane, active, maximized, compact, onToggleMaximize }: Props) {
  const [renaming, setRenaming] = useState(false);
  const [gitOpen, setGitOpen] = useState(false);
  const detached = useDetached((s) => s[pane.id] === true);
  const agent = AGENTS[pane.agent];

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: double-click shortcut; the maximize button is the accessible path
    <header
      className={`group/pane flex h-full min-w-0 items-center gap-2 px-2.5 text-[13px] ${active ? "text-fg" : "text-muted"}`}
      // Waiting for the user: a thin amber line on top, plus the pulsing dot.
      style={pane.status === "waiting" ? { boxShadow: "inset 0 1px 0 var(--color-wait)" } : undefined}
      onMouseDown={() => focusPane(pane.id)}
      onDoubleClick={(e) => {
        if (!(e.target instanceof Element && e.target.closest("button, input"))) onToggleMaximize();
      }}
    >
      <AgentLogo kind={pane.agent} size={14} />
      <OrchestratorButton pane={pane} />
      {renaming ? (
        <TitleField pane={pane} onDone={() => setRenaming(false)} />
      ) : (
        <span className="truncate font-medium" title={`${agent.name} · ${pane.cwd}`}>
          {pane.title || agent.name}
        </span>
      )}
      <StatusBadge status={pane.status} />

      {!compact && (
        <>
          <span className="min-w-0 flex-1 truncate text-xs text-subtle" title={pane.lastAction ?? undefined}>
            {pane.lastAction}
          </span>
          <UsageChip pane={pane} />
          {pane.taskStartedAt !== null && <Elapsed since={pane.taskStartedAt} />}
          {pane.completedCount > 0 && (
            <span
              className="flex shrink-0 items-center gap-1 text-xs tabular-nums text-subtle"
              title={`${pane.completedCount} task(s) done`}
            >
              <CheckCheck size={12} strokeWidth={1.5} aria-hidden="true" />
              {pane.completedCount}
            </span>
          )}
          <ReviewChip paneId={pane.id} />
          <QueueButton pane={pane} active={active} />
          {/* Actions stay out of the way until the pane is hovered, focused or active. */}
          <div
            className={`flex shrink-0 items-center transition-opacity duration-150 group-hover/pane:opacity-100 focus-within:opacity-100 ${
              active ? "opacity-100" : "opacity-0"
            }`}
            role="toolbar"
            aria-label={`${agent.name} actions`}
          >
            <IconButton icon={Pencil} size="sm" label="Rename" onClick={() => setRenaming(true)} />
            <IconButton
              icon={GitBranch}
              size="sm"
              label="Git"
              hint="Git: changes, commit, merge, restore points"
              aria-pressed={gitOpen}
              onClick={() => setGitOpen(true)}
            />
            <IconButton
              icon={Pause}
              size="sm"
              label="Interrupt"
              hint="Interrupt the current task (Esc, or Ctrl+C for a shell). The agent stays open."
              onClick={() => interruptPane(pane.id)}
              disabled={pane.exited}
            />
            <IconButton icon={RotateCcw} size="sm" label="Restart" onClick={() => void restartPane(pane.id)} />
            <IconButton
              icon={ZoomOut}
              size="sm"
              label="Zoom out"
              hint="Zoom out (Ctrl+-)"
              onClick={() => deck.zoom(pane.id, -1)}
              disabled={pane.fontSize <= FONT_SIZE.min}
            />
            <IconButton
              icon={ZoomIn}
              size="sm"
              label="Zoom in"
              hint="Zoom in (Ctrl+=, Ctrl+0 to reset)"
              onClick={() => deck.zoom(pane.id, 1)}
              disabled={pane.fontSize >= FONT_SIZE.max}
            />
            <IconButton
              icon={detached ? SquareArrowDownLeft : ExternalLink}
              size="sm"
              label={detached ? "Reattach" : "Detach to a window"}
              onClick={() => toggleDetached(pane.id)}
            />
            <IconButton
              icon={maximized ? Minimize2 : Maximize2}
              size="sm"
              label={maximized ? "Restore" : "Maximize"}
              hint={maximized ? "Restore (Esc)" : "Maximize (double-click the header)"}
              aria-pressed={maximized}
              onClick={onToggleMaximize}
            />
          </div>
        </>
      )}
      <IconButton
        icon={X}
        size="sm"
        tone="danger"
        label="Close"
        className={compact ? "ml-auto" : ""}
        onClick={() => confirmClose(pane)}
      />
      {gitOpen && <GitPanel pane={pane} onClose={() => setGitOpen(false)} />}
    </header>
  );
}

/** Fallback header while the pane is gone from the store (closing). */
export function EmptyPaneHeader() {
  return (
    <header className="flex h-full items-center gap-2 px-2 text-subtle">
      <SquareTerminal size={15} strokeWidth={1.5} aria-hidden="true" />
      Agent
    </header>
  );
}

/** Shows who reviews this agent's work (set in the Git panel). */
function ReviewChip({ paneId }: { paneId: string }) {
  const reviewerId = useReviews((r) => r[paneId]);
  const reviewer = useDeck((s) => (reviewerId ? s.panes[reviewerId] : undefined));
  if (!reviewer) return null;
  return (
    <span className="flex shrink-0 items-center gap-1 text-xs text-subtle" title={`Reviewed by ${reviewer.title}`}>
      <ScanEye size={12} strokeWidth={1.5} aria-hidden="true" />
      {reviewer.title}
    </span>
  );
}
