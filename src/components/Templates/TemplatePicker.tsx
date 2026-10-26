// "New session from template" + "Save session as template".
import { LayoutTemplate, Save, X } from "lucide-react";
import { type FormEvent, useEffect, useRef, useState } from "react";
import { create } from "zustand";
import { AGENTS } from "../../lib/agents";
import { activeWorkspace, deck, useDeck } from "../../store/deck";
import { allTemplates, newSessionFromTemplate, saveSessionAsTemplate, useTemplates } from "../../store/templates";
import { AgentLogo } from "../AgentLogo/AgentLogo";
import { IconButton } from "../IconButton/IconButton";

const usePicker = create<{ open: boolean }>(() => ({ open: false }));

/** Opens the template picker for the active workspace (sidebar, command palette). */
export const openTemplatePicker = () => usePicker.setState({ open: true });

/** Opens the picker for a given workspace (selects it first). */
export function openTemplatePickerFor(workspaceId: string) {
  deck.selectWorkspace(workspaceId);
  openTemplatePicker();
}

const errorMessage = (err: unknown) => (err instanceof Error ? err.message : String(err));

function Picker({ onClose }: { onClose: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const list = useTemplates((s) => s.list);
  const workspace = useDeck((s) => activeWorkspace(s));
  const session = useDeck((s) => s.tabs.find((t) => t.id === s.activeTabId));
  const [name, setName] = useState("");
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    dialogRef.current?.showModal();
  }, []);

  function save(e: FormEvent) {
    e.preventDefault();
    const label = name.trim() || session?.name || "Template";
    saveSessionAsTemplate(label).then(
      () => {
        setName("");
        setMessage({ ok: true, text: `Template "${label}" saved.` });
      },
      (err) => setMessage({ ok: false, text: `Could not save: ${errorMessage(err)}` }),
    );
  }

  return (
    <dialog
      ref={dialogRef}
      onClose={onClose}
      aria-labelledby="templates-title"
      className="m-auto w-[min(32rem,92vw)] rounded-xl border border-line bg-panel p-0 text-fg backdrop:bg-gutter/70"
    >
      <div className="flex items-center justify-between pt-4 pr-3 pl-5">
        <div>
          <h2 id="templates-title" className="text-[15px] font-medium">
            New session from template
          </h2>
          <p className="text-xs text-subtle">In {workspace.name}</p>
        </div>
        <IconButton icon={X} label="Close" hint="Close (Esc)" onClick={onClose} />
      </div>

      <ul className="scroll-thin flex max-h-80 flex-col gap-px overflow-y-auto p-3">
        {allTemplates(list).map((t) => (
          <li key={t.id}>
            <button
              type="button"
              onClick={() => {
                newSessionFromTemplate(t.id);
                onClose();
              }}
              className="flex h-10 w-full items-center gap-3 rounded-md px-2 text-left text-muted transition-colors duration-150 hover:bg-hover hover:text-fg"
            >
              <LayoutTemplate size={15} strokeWidth={1.5} className="shrink-0 text-subtle" aria-hidden="true" />
              <span className="truncate text-fg">{t.name}</span>
              {t.builtin && <span className="text-xs text-subtle">built-in</span>}
              <span className="ml-auto flex shrink-0 items-center gap-1" title={t.panes.map((p) => p.title).join(", ")}>
                {t.panes.map((p) => (
                  <AgentLogo key={p.key} kind={p.agent} size={13} />
                ))}
                <span className="sr-only">{t.panes.map((p) => AGENTS[p.agent].name).join(", ")}</span>
              </span>
            </button>
          </li>
        ))}
      </ul>

      <form onSubmit={save} className="flex flex-col gap-2 border-t border-line px-5 py-4">
        <label htmlFor="template-name" className="text-xs font-medium text-muted">
          Save current session as template
        </label>
        <div className="flex gap-2">
          <input
            id="template-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={session?.name ?? "Template name"}
            className="h-8 min-w-0 flex-1 rounded-md border border-line bg-base px-2.5 text-fg outline-none transition-colors duration-150 placeholder:text-subtle hover:border-line-strong focus:border-accent"
          />
          <button
            type="submit"
            disabled={!session?.paneIds.length}
            className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border border-line px-3 text-muted transition-colors duration-150 hover:bg-hover hover:text-fg disabled:pointer-events-none disabled:opacity-40"
          >
            <Save size={14} strokeWidth={1.5} aria-hidden="true" />
            Save
          </button>
        </div>
        {message && (
          <p role="status" className={`text-xs ${message.ok ? "text-ok" : "text-bad"}`}>
            {message.text}
          </p>
        )}
      </form>
    </dialog>
  );
}

/** Mounted once in App. */
export function TemplatePickerHost() {
  const open = usePicker((s) => s.open);
  return open ? <Picker onClose={() => usePicker.setState({ open: false })} /> : null;
}
