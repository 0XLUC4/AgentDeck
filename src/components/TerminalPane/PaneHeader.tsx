import {
  CheckCheck,
  ExternalLink,
  GitBranch,
  Maximize2,
  Minimize2,
  Pause,
  Pencil,
  RotateCcw,
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
import { deck, FONT_SIZE } from "../../store/deck";
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
  if (pane.status !== "working" || window.confirm(`${AGENTS[pane.agent].name} travaille encore. Fermer quand même ?`)) {
    void closePane(pane.id);
  }
}

function Elapsed({ since }: { since: number }) {
  const now = useNow();
  return (
    <span className="shrink-0 font-mono text-xs tabular-nums text-muted" title="Temps écoulé sur la tâche">
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
      aria-label="Titre de la tâche"
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
              title={`${pane.completedCount} tâche(s) terminée(s)`}
            >
              <CheckCheck size={12} strokeWidth={1.5} aria-hidden="true" />
              {pane.completedCount}
            </span>
          )}
          <QueueButton pane={pane} active={active} />
          {/* Actions stay out of the way until the pane is hovered, focused or active. */}
          <div
            className={`flex shrink-0 items-center transition-opacity duration-150 group-hover/pane:opacity-100 focus-within:opacity-100 ${
              active ? "opacity-100" : "opacity-0"
            }`}
            role="toolbar"
            aria-label={`Actions ${agent.name}`}
          >
            <IconButton icon={Pencil} size="sm" label="Renommer" onClick={() => setRenaming(true)} />
            <IconButton
              icon={GitBranch}
              size="sm"
              label="Git"
              hint="Git : modifications, commit, merge, points de restauration"
              aria-pressed={gitOpen}
              onClick={() => setGitOpen(true)}
            />
            <IconButton
              icon={Pause}
              size="sm"
              label="Interrompre"
              hint="Interrompre la tâche en cours (Échap, ou Ctrl+C pour un shell). L'agent reste ouvert."
              onClick={() => interruptPane(pane.id)}
              disabled={pane.exited}
            />
            <IconButton icon={RotateCcw} size="sm" label="Relancer" onClick={() => void restartPane(pane.id)} />
            <IconButton
              icon={ZoomOut}
              size="sm"
              label="Dézoomer"
              hint="Dézoomer (Ctrl+-)"
              onClick={() => deck.zoom(pane.id, -1)}
              disabled={pane.fontSize <= FONT_SIZE.min}
            />
            <IconButton
              icon={ZoomIn}
              size="sm"
              label="Zoomer"
              hint="Zoomer (Ctrl+=, Ctrl+0 pour réinitialiser)"
              onClick={() => deck.zoom(pane.id, 1)}
              disabled={pane.fontSize >= FONT_SIZE.max}
            />
            <IconButton
              icon={detached ? SquareArrowDownLeft : ExternalLink}
              size="sm"
              label={detached ? "Rattacher" : "Détacher dans une fenêtre"}
              onClick={() => toggleDetached(pane.id)}
            />
            <IconButton
              icon={maximized ? Minimize2 : Maximize2}
              size="sm"
              label={maximized ? "Restaurer" : "Agrandir"}
              hint={maximized ? "Restaurer (Échap)" : "Agrandir (double-clic sur l'en-tête)"}
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
        label="Fermer"
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
