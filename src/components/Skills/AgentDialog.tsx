// Dialog shell + Markdown file editor shared by the Skills and Notes header buttons.
import { ArrowLeft, Save, X } from "lucide-react";
import { type ReactNode, type SyntheticEvent, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AGENTS } from "../../lib/agents";
import { agentFileRead, agentFileWrite } from "../../lib/ipc";
import type { Pane } from "../../store/deck";
import { IconButton } from "../IconButton/IconButton";

// The dialog is portaled out of the dockview tab, but React events still bubble to the header (drag, maximize).
const stop = (e: SyntheticEvent) => e.stopPropagation();

/** Modal in <body> for one pane's agent. `status` is the footer line. */
export function AgentDialog({
  pane,
  title,
  status,
  onClose,
  children,
}: {
  pane: Pane;
  title: string;
  status: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => ref.current?.showModal(), []);

  return createPortal(
    <dialog
      ref={ref}
      onClose={onClose}
      onMouseDown={stop}
      onPointerDown={stop}
      onDoubleClick={stop}
      onKeyDown={stop}
      aria-labelledby={titleId}
      className="m-auto w-[min(48rem,94vw)] rounded-xl border border-line bg-panel p-0 text-[13px] text-fg backdrop:bg-gutter/70"
    >
      <div className="flex items-center justify-between pt-4 pr-3 pl-5">
        <div className="min-w-0">
          <h2 id={titleId} className="text-[15px] font-medium">
            {title}
          </h2>
          <p className="truncate text-xs text-subtle">
            {AGENTS[pane.agent].name} · {pane.cwd}
          </p>
        </div>
        <IconButton icon={X} label="Close" hint="Close (Esc)" onClick={onClose} />
      </div>
      <div className="flex flex-col gap-3 p-5">{children}</div>
      <p className="border-t border-line px-5 py-3 text-xs break-words text-subtle" role="status">
        {status}
      </p>
    </dialog>,
    document.body,
  );
}

/** Edits one listed file; creates it on save when missing. */
export function FileEditor({
  pane,
  path,
  onBack,
  onStatus,
}: {
  pane: Pane;
  path: string;
  onBack: () => void;
  onStatus: (text: string) => void;
}) {
  const [saved, setSaved] = useState<string | null>(null);
  const [text, setText] = useState("");
  const dirty = saved !== null && text !== saved;

  useEffect(() => {
    agentFileRead(pane.agent, pane.cwd, path)
      .then((content) => {
        setSaved(content);
        setText(content);
      })
      .catch((err) => onStatus(String(err)));
  }, [pane.agent, pane.cwd, path, onStatus]);

  async function save() {
    try {
      await agentFileWrite(pane.agent, pane.cwd, path, text);
      setSaved(text);
      onStatus(`Saved ${path}`);
    } catch (err) {
      onStatus(String(err));
    }
  }

  return (
    <>
      <div className="flex items-center gap-2">
        <IconButton
          icon={ArrowLeft}
          size="sm"
          label="Back"
          onClick={() => (!dirty || window.confirm("Discard your changes?")) && onBack()}
        />
        <span className="min-w-0 flex-1 truncate font-mono text-xs text-muted" title={path}>
          {path}
        </span>
        <button
          type="button"
          disabled={!dirty}
          onClick={() => void save()}
          className="inline-flex h-8 items-center gap-1.5 rounded-md bg-accent px-3 font-medium text-on-accent transition-colors duration-150 hover:bg-accent-hover disabled:pointer-events-none disabled:opacity-40"
        >
          <Save size={14} strokeWidth={1.5} aria-hidden="true" />
          Save
        </button>
      </div>
      <textarea
        value={text}
        disabled={saved === null}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "s" && e.ctrlKey) {
            e.preventDefault();
            if (dirty) void save();
          }
        }}
        aria-label="File contents"
        spellCheck={false}
        className="scroll-thin h-[55vh] resize-none rounded-md border border-line bg-base p-3 font-mono text-xs leading-relaxed text-fg outline-none focus:border-accent"
      />
    </>
  );
}
