import { AGENTS } from "../../lib/agents";
import { formatElapsed, useNow } from "../../lib/clock";
import type { Pane } from "../../store/deck";
import { deck, FONT_SIZE } from "../../store/deck";
import { closePane, interruptPane, restartPane } from "../../store/runtime";
import { StatusBadge } from "../StatusBadge/StatusBadge";

interface Props {
  pane: Pane;
  maximized: boolean;
  onToggleMaximize: () => void;
}

function MagnifierIcon({ plus }: { plus: boolean }) {
  return (
    <svg viewBox="0 0 16 16" className="size-4" fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden="true">
      <circle cx="6.5" cy="6.5" r="4.5" />
      <path d="M10 10l4.5 4.5M4.5 6.5h4" />
      {plus && <path d="M6.5 4.5v4" />}
    </svg>
  );
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
    <span className="tabular-nums text-zinc-400" title="Temps écoulé sur la tâche">
      {formatElapsed(now - since)}
    </span>
  );
}

const btn =
  "flex items-center rounded px-1.5 disabled:opacity-40 py-0.5 text-zinc-400 hover:bg-zinc-700 hover:text-zinc-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-blue-500";

export function PaneHeader({ pane, maximized, onToggleMaximize }: Props) {
  const agent = AGENTS[pane.agent];
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: double-click shortcut; the maximize button is the accessible path
    <header
      className="flex h-8 shrink-0 items-center gap-2 border-b border-zinc-800 bg-zinc-900 px-2 text-sm"
      onDoubleClick={(e) => {
        if (!(e.target instanceof Element && e.target.closest("button, input"))) onToggleMaximize();
      }}
    >
      <img src={agent.logo} alt="" className="size-5 shrink-0" />
      <span className="shrink-0 font-medium" style={{ color: agent.color }}>
        {agent.name}
      </span>
      <input
        value={pane.title}
        onChange={(e) => deck.rename(pane.id, e.target.value)}
        aria-label="Titre de la tâche"
        placeholder="Titre de la tâche"
        className="w-32 min-w-0 rounded bg-transparent px-1 text-zinc-200 hover:bg-zinc-800 focus:bg-zinc-800 focus:outline-none"
      />
      <StatusBadge status={pane.status} />
      <span className="min-w-0 flex-1 truncate text-xs text-zinc-400" title={pane.lastAction ?? undefined}>
        {pane.lastAction}
      </span>
      {pane.taskStartedAt !== null && <Elapsed since={pane.taskStartedAt} />}
      <span className="shrink-0 text-xs text-zinc-400" title="Tâches terminées">
        ✓ {pane.completedCount}
      </span>
      <button
        type="button"
        className={btn}
        onClick={() => interruptPane(pane.id)}
        disabled={pane.exited}
        aria-label="Interrompre"
        title="Interrompre la tâche en cours (Échap / Ctrl+C) — l'agent reste ouvert"
      >
        ⏸
      </button>
      <button
        type="button"
        className={btn}
        onClick={() => void restartPane(pane.id)}
        aria-label="Relancer"
        title="Relancer"
      >
        ↻
      </button>
      <button
        type="button"
        className={btn}
        onClick={() => deck.zoom(pane.id, -1)}
        disabled={pane.fontSize <= FONT_SIZE.min}
        aria-label="Dézoomer"
        title="Dézoomer (Ctrl+-)"
      >
        <MagnifierIcon plus={false} />
      </button>
      <button
        type="button"
        className={btn}
        onClick={() => deck.zoom(pane.id, 1)}
        disabled={pane.fontSize >= FONT_SIZE.max}
        aria-label="Zoomer"
        title="Zoomer (Ctrl+=, Ctrl+0 pour réinitialiser)"
      >
        <MagnifierIcon plus />
      </button>
      <button
        type="button"
        className={btn}
        onClick={onToggleMaximize}
        aria-pressed={maximized}
        aria-label={maximized ? "Restaurer" : "Agrandir"}
        title={maximized ? "Restaurer (Échap)" : "Agrandir (double-clic sur l'en-tête)"}
      >
        ⛶
      </button>
      <button type="button" className={btn} onClick={() => confirmClose(pane)} aria-label="Fermer" title="Fermer">
        ✕
      </button>
    </header>
  );
}
