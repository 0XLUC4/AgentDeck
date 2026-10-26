// Pane header button + editor for the pane's prompt queue (sent one by one each time the agent finishes).
import { ArrowDown, ArrowUp, ListPlus, Pause, Play, Plus, Send, Trash2, X } from "lucide-react";
import { type FormEvent, type SyntheticEvent, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AGENTS } from "../../lib/agents";
import type { Pane } from "../../store/deck";
import { EMPTY_QUEUE, QUEUE_AGENTS, queues, sendNext, useQueues } from "../../store/queue";
import { IconButton } from "../IconButton/IconButton";

// The dialog is portaled out of the dockview tab, but React events still bubble to the header (drag, maximize).
const stop = (e: SyntheticEvent) => e.stopPropagation();

function QueueDialog({ pane, onClose }: { pane: Pane; onClose: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const queue = useQueues((s) => s[pane.id] ?? EMPTY_QUEUE);
  const [text, setText] = useState("");
  const busy = pane.status === "working" || pane.status === "waiting" || pane.exited;

  useEffect(() => {
    dialogRef.current?.showModal();
  }, []);

  function add(e?: FormEvent) {
    e?.preventDefault();
    queues.add(pane.id, text);
    setText("");
  }

  return (
    <dialog
      ref={dialogRef}
      onClose={onClose}
      onMouseDown={stop}
      onPointerDown={stop}
      onDoubleClick={stop}
      onKeyDown={stop}
      aria-labelledby="queue-title"
      className="m-auto w-[min(34rem,92vw)] rounded-xl border border-line bg-panel p-0 text-[13px] text-fg backdrop:bg-gutter/70"
    >
      <div className="flex items-center justify-between pt-4 pr-3 pl-5">
        <div className="min-w-0">
          <h2 id="queue-title" className="text-[15px] font-medium">
            File d'attente
          </h2>
          <p className="truncate text-xs text-subtle">
            {pane.title || AGENTS[pane.agent].name} · envoi automatique 1,5 s après chaque tâche terminée
          </p>
        </div>
        <IconButton icon={X} label="Fermer" hint="Fermer (Échap)" onClick={onClose} />
      </div>

      <div className="flex flex-col gap-3 p-5">
        <form onSubmit={add} className="flex items-start gap-2">
          <textarea
            autoFocus
            value={text}
            rows={2}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && e.ctrlKey) add();
            }}
            placeholder="Prompt à mettre en file (Ctrl+Entrée)"
            aria-label="Nouveau prompt"
            className="min-h-16 min-w-0 flex-1 resize-y rounded-md border border-line bg-base px-2.5 py-1.5 text-fg outline-none transition-colors duration-150 placeholder:text-subtle hover:border-line-strong focus:border-accent"
          />
          <IconButton icon={Plus} type="submit" label="Ajouter" disabled={!text.trim()} />
        </form>

        {queue.items.length === 0 ? (
          <p className="py-3 text-center text-muted">File vide.</p>
        ) : (
          <ol className="scroll-thin flex max-h-72 flex-col overflow-y-auto rounded-md border border-line">
            {queue.items.map((item, i) => (
              <li
                key={item.id}
                className="flex items-start gap-2 border-b border-line py-1.5 pr-1 pl-3 last:border-b-0"
              >
                <span className="pt-1 font-mono text-xs text-subtle tabular-nums">{i + 1}</span>
                <span className="min-w-0 flex-1 whitespace-pre-wrap break-words pt-0.5 text-fg">{item.text}</span>
                <IconButton
                  icon={ArrowUp}
                  size="sm"
                  label="Monter"
                  disabled={i === 0}
                  onClick={() => queues.move(pane.id, item.id, -1)}
                />
                <IconButton
                  icon={ArrowDown}
                  size="sm"
                  label="Descendre"
                  disabled={i === queue.items.length - 1}
                  onClick={() => queues.move(pane.id, item.id, 1)}
                />
                <IconButton
                  icon={Trash2}
                  size="sm"
                  tone="danger"
                  label="Supprimer"
                  onClick={() => queues.remove(pane.id, item.id)}
                />
              </li>
            ))}
          </ol>
        )}
      </div>

      <div className="flex items-center gap-2 border-t border-line px-5 py-3">
        <span className="mr-auto text-xs text-subtle" role="status">
          {queue.paused ? "En pause" : `${queue.items.length} en attente`}
        </span>
        <button
          type="button"
          onClick={() => queues.setPaused(pane.id, !queue.paused)}
          className="inline-flex h-8 items-center gap-1.5 rounded-md border border-line px-3 text-muted transition-colors duration-150 hover:bg-hover hover:text-fg"
        >
          {queue.paused ? (
            <Play size={14} strokeWidth={1.5} aria-hidden="true" />
          ) : (
            <Pause size={14} strokeWidth={1.5} aria-hidden="true" />
          )}
          {queue.paused ? "Reprendre" : "Mettre en pause"}
        </button>
        <button
          type="button"
          disabled={busy || queue.items.length === 0}
          title={busy ? "L'agent est occupé : le suivant partira quand il aura terminé" : undefined}
          onClick={() => void sendNext(pane.id)}
          className="inline-flex h-8 items-center gap-1.5 rounded-md bg-accent px-3 font-medium text-on-accent transition-colors duration-150 hover:bg-accent-hover disabled:pointer-events-none disabled:opacity-40"
        >
          <Send size={14} strokeWidth={1.5} aria-hidden="true" />
          Envoyer le suivant
        </button>
      </div>
    </dialog>
  );
}

/** Header button with the queue size badge. Disabled for agents without hooks (no "done" event). */
export function QueueButton({ pane, active }: { pane: Pane; active: boolean }) {
  const [open, setOpen] = useState(false);
  const count = useQueues((s) => s[pane.id]?.items.length ?? 0);
  const paused = useQueues((s) => s[pane.id]?.paused ?? false);
  const supported = QUEUE_AGENTS.includes(pane.agent);
  // Like the other header actions, hidden until hover/focus/active, but always shown while the queue has items.
  const visibility =
    count > 0 || active ? "" : "opacity-0 group-hover/pane:opacity-100 focus-within:opacity-100 transition-opacity";

  if (!supported) {
    return (
      // Disabled buttons get no pointer events: the tooltip lives on the wrapper.
      <span
        className={`inline-flex ${visibility}`}
        title="File d'attente : uniquement pour Claude Code et Codex (fin de tâche détectée par les hooks)"
      >
        <IconButton icon={ListPlus} size="sm" label="File d'attente indisponible" disabled />
      </span>
    );
  }
  return (
    <span className={`relative inline-flex ${visibility}`}>
      <IconButton
        icon={ListPlus}
        size="sm"
        label="File d'attente"
        hint={count > 0 ? `File d'attente : ${count} prompt(s)${paused ? " (en pause)" : ""}` : "File d'attente"}
        onClick={() => setOpen(true)}
      />
      {count > 0 && (
        <span
          aria-hidden="true"
          className={`pointer-events-none absolute -top-0.5 -right-0.5 flex h-3.5 min-w-3.5 items-center justify-center rounded-full px-0.5 font-mono text-[10px] leading-none tabular-nums ${
            paused ? "bg-raised text-muted" : "bg-accent text-on-accent"
          }`}
        >
          {count}
        </span>
      )}
      {open && createPortal(<QueueDialog pane={pane} onClose={() => setOpen(false)} />, document.body)}
    </span>
  );
}
