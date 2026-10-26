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
            Queue
          </h2>
          <p className="truncate text-xs text-subtle">
            {pane.title || AGENTS[pane.agent].name} · sent automatically 1.5 s after each finished task
          </p>
        </div>
        <IconButton icon={X} label="Close" hint="Close (Esc)" onClick={onClose} />
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
            placeholder="Prompt to queue (Ctrl+Enter)"
            aria-label="New prompt"
            className="min-h-16 min-w-0 flex-1 resize-y rounded-md border border-line bg-base px-2.5 py-1.5 text-fg outline-none transition-colors duration-150 placeholder:text-subtle hover:border-line-strong focus:border-accent"
          />
          <IconButton icon={Plus} type="submit" label="Add" disabled={!text.trim()} />
        </form>

        {queue.items.length === 0 ? (
          <p className="py-3 text-center text-muted">Queue is empty.</p>
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
                  label="Move up"
                  disabled={i === 0}
                  onClick={() => queues.move(pane.id, item.id, -1)}
                />
                <IconButton
                  icon={ArrowDown}
                  size="sm"
                  label="Move down"
                  disabled={i === queue.items.length - 1}
                  onClick={() => queues.move(pane.id, item.id, 1)}
                />
                <IconButton
                  icon={Trash2}
                  size="sm"
                  tone="danger"
                  label="Remove"
                  onClick={() => queues.remove(pane.id, item.id)}
                />
              </li>
            ))}
          </ol>
        )}
      </div>

      <div className="flex items-center gap-2 border-t border-line px-5 py-3">
        <span className="mr-auto text-xs text-subtle" role="status">
          {queue.paused ? "Paused" : `${queue.items.length} queued`}
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
          {queue.paused ? "Resume" : "Pause"}
        </button>
        <button
          type="button"
          disabled={busy || queue.items.length === 0}
          title={busy ? "The agent is busy: the next prompt goes out when it finishes" : undefined}
          onClick={() => void sendNext(pane.id)}
          className="inline-flex h-8 items-center gap-1.5 rounded-md bg-accent px-3 font-medium text-on-accent transition-colors duration-150 hover:bg-accent-hover disabled:pointer-events-none disabled:opacity-40"
        >
          <Send size={14} strokeWidth={1.5} aria-hidden="true" />
          Send next
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
        title="Queue: Claude Code and Codex only (task end is detected by hooks)"
      >
        <IconButton icon={ListPlus} size="sm" label="Queue unavailable" disabled />
      </span>
    );
  }
  return (
    <span className={`relative inline-flex ${visibility}`}>
      <IconButton
        icon={ListPlus}
        size="sm"
        label="Queue"
        hint={count > 0 ? `Queue: ${count} prompt(s)${paused ? " (paused)" : ""}` : "Queue"}
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
