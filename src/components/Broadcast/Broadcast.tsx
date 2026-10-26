// "Broadcast prompt": one prompt sent to several running agents of the current workspace.
import { Check, CircleAlert, Send, X } from "lucide-react";
import { type FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { create } from "zustand";
import { AGENT_KINDS, AGENTS } from "../../lib/agents";
import type { AgentKind } from "../../lib/types";
import { activeWorkspace, useDeck } from "../../store/deck";
import { AgentLogo } from "../AgentLogo/AgentLogo";
import { IconButton } from "../IconButton/IconButton";
import { STATUS, StatusDot } from "../StatusBadge/StatusBadge";
import { sendPrompt } from "./send";

const useBroadcast = create<{ open: boolean }>(() => ({ open: false }));

/** Opens the broadcast composer (TitleBar button, Ctrl+Shift+B, command palette). */
export const openBroadcast = () => useBroadcast.setState({ open: true });

type Result = { ok: true } | { ok: false; error: string };

const chip =
  "inline-flex h-7 items-center gap-1.5 rounded-md border border-line px-2.5 text-xs text-muted transition-colors duration-150 hover:bg-hover hover:text-fg";

function Composer({ onClose }: { onClose: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const state = useDeck();
  const workspace = activeWorkspace(state);
  const [text, setText] = useState("");
  const [results, setResults] = useState<Record<string, Result>>({});
  const [sending, setSending] = useState(false);

  // Running panes of the current workspace, with their session name.
  const targets = useMemo(
    () =>
      state.tabs
        .filter((t) => t.workspaceId === workspace.id)
        .flatMap((tab) => tab.paneIds.map((id) => ({ pane: state.panes[id], tab })))
        .filter(({ pane }) => pane && !pane.exited),
    [state.tabs, state.panes, workspace.id],
  );
  const sessionIds = () =>
    targets.filter(({ tab }) => tab.id === useDeck.getState().activeTabId).map(({ pane }) => pane.id);
  const [selected, setSelected] = useState<Set<string>>(() => new Set(sessionIds()));
  const kinds = AGENT_KINDS.filter((k) => targets.some(({ pane }) => pane.agent === k));
  const chosen = targets.filter(({ pane }) => selected.has(pane.id));
  const waiting = chosen.filter(({ pane }) => pane.status === "waiting").length;

  useEffect(() => {
    dialogRef.current?.showModal();
  }, []);

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  const selectKind = (kind: AgentKind) =>
    setSelected(new Set(targets.filter(({ pane }) => pane.agent === kind).map(({ pane }) => pane.id)));

  async function submit(e?: FormEvent) {
    e?.preventDefault();
    if (!text.trim() || chosen.length === 0 || sending) return;
    setSending(true);
    const settled = await Promise.all(
      chosen.map(({ pane }) =>
        sendPrompt(pane.id, text.trim()).then(
          (): [string, Result] => [pane.id, { ok: true }],
          (err): [string, Result] => [pane.id, { ok: false, error: err instanceof Error ? err.message : String(err) }],
        ),
      ),
    );
    setResults(Object.fromEntries(settled));
    setSending(false);
  }

  return (
    <dialog
      ref={dialogRef}
      onClose={onClose}
      aria-labelledby="broadcast-title"
      className="m-auto w-[min(40rem,92vw)] rounded-xl border border-line bg-panel p-0 text-fg backdrop:bg-gutter/70"
    >
      <form onSubmit={submit} className="flex flex-col">
        <div className="flex items-center justify-between pt-4 pr-3 pl-5">
          <div>
            <h2 id="broadcast-title" className="text-[15px] font-medium">
              Broadcast prompt
            </h2>
            <p className="text-xs text-subtle">Running agents in {workspace.name}</p>
          </div>
          <IconButton icon={X} label="Close" hint="Close (Esc)" onClick={onClose} />
        </div>

        <div className="flex flex-col gap-4 p-5">
          <textarea
            autoFocus
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && e.ctrlKey) void submit();
            }}
            rows={5}
            placeholder="Prompt sent to every selected agent…"
            aria-label="Prompt"
            className="min-h-24 resize-y rounded-md border border-line bg-base px-3 py-2 text-fg outline-none transition-colors duration-150 placeholder:text-subtle hover:border-line-strong focus:border-accent"
          />

          <fieldset className="flex flex-wrap items-center gap-1.5">
            <legend className="sr-only">Quick select</legend>
            <button
              type="button"
              className={chip}
              onClick={() => setSelected(new Set(targets.map(({ pane }) => pane.id)))}
            >
              All
            </button>
            <button type="button" className={chip} onClick={() => setSelected(new Set(sessionIds()))}>
              Current session
            </button>
            {kinds.map((kind) => (
              <button key={kind} type="button" className={chip} onClick={() => selectKind(kind)}>
                <AgentLogo kind={kind} size={12} />
                {AGENTS[kind].name}
              </button>
            ))}
            <button type="button" className={`${chip} ml-auto`} onClick={() => setSelected(new Set())}>
              None
            </button>
          </fieldset>

          {targets.length === 0 ? (
            <p className="py-4 text-center text-muted">No running agents in this workspace.</p>
          ) : (
            <ul className="scroll-thin flex max-h-64 flex-col overflow-y-auto rounded-md border border-line">
              {targets.map(({ pane, tab }) => {
                const result = results[pane.id];
                return (
                  <li key={pane.id} className="border-b border-line last:border-b-0">
                    <label className="flex h-9 cursor-pointer items-center gap-2.5 px-3 hover:bg-hover">
                      <input
                        type="checkbox"
                        checked={selected.has(pane.id)}
                        onChange={() => toggle(pane.id)}
                        className="size-4 accent-[var(--color-accent)]"
                      />
                      <AgentLogo kind={pane.agent} size={14} />
                      <span className="truncate text-fg">{pane.title || AGENTS[pane.agent].name}</span>
                      <span className="truncate text-xs text-subtle">{tab.name}</span>
                      <span className="ml-auto flex shrink-0 items-center gap-2 text-xs">
                        {pane.status === "waiting" && <span className="text-wait">waiting for a reply</span>}
                        <span title={STATUS[pane.status].label} className="inline-flex">
                          <StatusDot status={pane.status} />
                        </span>
                        {result?.ok && (
                          <span className="inline-flex items-center gap-1 text-ok">
                            <Check size={12} strokeWidth={1.5} aria-hidden="true" />
                            Sent
                          </span>
                        )}
                        {result && !result.ok && (
                          <span className="inline-flex max-w-48 items-center gap-1 text-bad" title={result.error}>
                            <CircleAlert size={12} strokeWidth={1.5} aria-hidden="true" />
                            <span className="truncate">{result.error}</span>
                          </span>
                        )}
                      </span>
                    </label>
                  </li>
                );
              })}
            </ul>
          )}

          {waiting > 0 && (
            <p className="flex items-center gap-2 text-xs text-wait">
              <CircleAlert size={13} strokeWidth={1.5} aria-hidden="true" />
              {waiting} agent(s) waiting for a reply: the prompt will arrive as their answer.
            </p>
          )}
        </div>

        <div className="flex items-center justify-end gap-3 border-t border-line px-5 py-3">
          <span className="mr-auto text-xs text-subtle">Ctrl+Enter to send</span>
          <button
            type="submit"
            disabled={!text.trim() || chosen.length === 0 || sending}
            className="inline-flex h-8 items-center gap-2 rounded-md bg-accent px-3 font-medium text-on-accent transition-colors duration-150 hover:bg-accent-hover disabled:pointer-events-none disabled:opacity-40"
          >
            <Send size={14} strokeWidth={1.5} aria-hidden="true" />
            Send to {chosen.length}
          </button>
        </div>
      </form>
    </dialog>
  );
}

/** Mounted once in App: renders the composer when open and binds Ctrl+Shift+B. */
export function BroadcastHost() {
  const open = useBroadcast((s) => s.open);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey && e.shiftKey && !e.altKey && e.code === "KeyB")) return;
      if (e.target instanceof HTMLElement && e.target.closest("[data-shortcut-recorder]")) return;
      e.preventDefault();
      e.stopPropagation();
      if (!e.repeat) openBroadcast();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  return open ? <Composer onClose={() => useBroadcast.setState({ open: false })} /> : null;
}
