// Pane header button + dialog listing the agent's instruction and memory files (CLAUDE.md, AGENTS.md, memory/…), editable.
import { FilePlus, NotebookPen, Pencil } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { type NoteFile, notesList } from "../../lib/ipc";
import type { Pane } from "../../store/deck";
import { IconButton } from "../IconButton/IconButton";
import { AgentDialog, FileEditor } from "./AgentDialog";

function NotesDialog({ pane, onClose }: { pane: Pane; onClose: () => void }) {
  const [notes, setNotes] = useState<NoteFile[] | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [status, setStatus] = useState("");

  const refresh = useCallback(
    () =>
      notesList(pane.agent, pane.cwd)
        .then(setNotes)
        .catch((err) => setStatus(String(err))),
    [pane.agent, pane.cwd],
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <AgentDialog
      pane={pane}
      title="Notes"
      onClose={onClose}
      status={
        status || "Instructions the agent reads at start, and its memory files. Changes apply to its next session."
      }
    >
      {editing ? (
        <FileEditor
          pane={pane}
          path={editing}
          onStatus={setStatus}
          onBack={() => {
            setEditing(null);
            void refresh();
          }}
        />
      ) : notes === null ? (
        <p className="py-3 text-center text-muted">Loading…</p>
      ) : (
        <ul className="scroll-thin flex max-h-96 flex-col overflow-y-auto rounded-md border border-line">
          {notes.map((note) => (
            <li
              key={note.path}
              className="flex items-center gap-2 border-b border-line py-1.5 pr-1 pl-3 last:border-b-0"
            >
              <div className="min-w-0 flex-1">
                <span className={`block truncate ${note.exists ? "text-fg" : "text-subtle"}`}>{note.label}</span>
                <span className="block truncate font-mono text-[11px] text-subtle" title={note.path}>
                  {note.path}
                </span>
              </div>
              <IconButton
                icon={note.exists ? Pencil : FilePlus}
                size="sm"
                label={note.exists ? `Edit ${note.label}` : `Create ${note.label}`}
                onClick={() => setEditing(note.path)}
              />
            </li>
          ))}
        </ul>
      )}
    </AgentDialog>
  );
}

/** Header button (agent panes) opening the notes editor. */
export function NotesButton({ pane }: { pane: Pane }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <IconButton
        icon={NotebookPen}
        size="sm"
        label="Notes"
        hint="Notes: the agent's instructions and memory files"
        onClick={() => setOpen(true)}
      />
      {open && <NotesDialog pane={pane} onClose={() => setOpen(false)} />}
    </>
  );
}
