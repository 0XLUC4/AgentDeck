// Pane header badge for an orchestrator pane: crown + journal of its MCP tool calls + global pause.
import { Crown, Pause, Play, ScrollText, X } from "lucide-react";
import { type SyntheticEvent, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { type ActivityEntry, setOrchestratorPaused, useOrchestrator } from "../../lib/orchestrator";
import type { Pane } from "../../store/deck";
import { IconButton } from "../IconButton/IconButton";

const EMPTY: ActivityEntry[] = [];
// Portaled out of the dockview tab, but React events still bubble to the header (drag, maximize).
const stop = (e: SyntheticEvent) => e.stopPropagation();
const time = (at: number) => new Date(at).toLocaleTimeString("en-US");

export function OrchestratorCrown({ size = 13 }: { size?: number }) {
  return (
    <Crown size={size} strokeWidth={1.5} aria-label="Orchestrator" className="shrink-0 text-[var(--color-wait)]" />
  );
}

export function PauseToggle() {
  const paused = useOrchestrator((s) => s.paused);
  return (
    <button
      type="button"
      aria-pressed={paused}
      onClick={() => void setOrchestratorPaused(!paused)}
      className="inline-flex h-8 items-center gap-1.5 rounded-md border border-line px-3 text-muted transition-colors duration-150 hover:bg-hover hover:text-fg aria-pressed:border-line-strong aria-pressed:text-fg"
    >
      {paused ? (
        <Play size={14} strokeWidth={1.5} aria-hidden="true" />
      ) : (
        <Pause size={14} strokeWidth={1.5} aria-hidden="true" />
      )}
      {paused ? "Resume orchestrator" : "Pause orchestrator"}
    </button>
  );
}

function ActivityDialog({ pane, onClose }: { pane: Pane; onClose: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const entries = useOrchestrator((s) => s.log[pane.id] ?? EMPTY);
  const paused = useOrchestrator((s) => s.paused);

  useEffect(() => {
    dialogRef.current?.showModal();
  }, []);

  return (
    <dialog
      ref={dialogRef}
      onClose={onClose}
      onMouseDown={stop}
      onPointerDown={stop}
      onDoubleClick={stop}
      onKeyDown={stop}
      aria-labelledby="orchestrator-log-title"
      className="m-auto w-[min(40rem,92vw)] rounded-xl border border-line bg-panel p-0 text-[13px] text-fg backdrop:bg-gutter/70"
    >
      <div className="flex items-center justify-between pt-4 pr-3 pl-5">
        <div className="flex min-w-0 items-center gap-2">
          <OrchestratorCrown size={16} />
          <h2 id="orchestrator-log-title" className="text-[15px] font-medium">
            Orchestrator actions
          </h2>
        </div>
        <IconButton icon={X} label="Close" hint="Close (Esc)" onClick={onClose} />
      </div>
      <div className="p-5">
        {entries.length === 0 ? (
          <p className="py-3 text-center text-muted">No actions yet.</p>
        ) : (
          <ol className="scroll-thin flex max-h-96 flex-col overflow-y-auto rounded-md border border-line">
            {entries.map((e) => (
              <li
                key={`${e.at}-${e.tool}-${e.summary}`}
                className="flex items-baseline gap-3 border-b border-line px-3 py-1.5 last:border-b-0"
              >
                <span className="shrink-0 font-mono text-xs text-subtle tabular-nums">{time(e.at)}</span>
                <span className="w-24 shrink-0 font-mono text-xs text-fg">{e.tool}</span>
                <span className="min-w-0 flex-1 break-words">
                  <span className="text-muted">{e.summary}</span>
                  {e.error && <span className="block text-xs text-bad">{e.error}</span>}
                </span>
              </li>
            ))}
          </ol>
        )}
      </div>
      <div className="flex items-center gap-2 border-t border-line px-5 py-3">
        <span className="mr-auto text-xs text-subtle" role="status">
          {paused ? "Paused: launch, send and interrupt are blocked" : "Cannot answer permissions or close agents"}
        </span>
        <PauseToggle />
      </div>
    </dialog>
  );
}

/** Crown + activity log button; renders nothing for ordinary panes. */
export function OrchestratorButton({ pane }: { pane: Pane }) {
  const [open, setOpen] = useState(false);
  const count = useOrchestrator((s) => s.log[pane.id]?.length ?? 0);
  const paused = useOrchestrator((s) => s.paused);
  if (!pane.orchestrator) return null;
  return (
    <span className="inline-flex shrink-0 items-center gap-1">
      <OrchestratorCrown />
      <IconButton
        icon={ScrollText}
        size="sm"
        label="Orchestrator actions"
        hint={`Orchestrator actions (${count})${paused ? " · paused" : ""}`}
        onClick={() => setOpen(true)}
      />
      {open && createPortal(<ActivityDialog pane={pane} onClose={() => setOpen(false)} />, document.body)}
    </span>
  );
}
