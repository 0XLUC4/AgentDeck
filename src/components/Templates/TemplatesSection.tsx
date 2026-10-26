// Settings > Templates: rename or delete saved session templates (builtins are read-only).
import { Pencil, Trash2 } from "lucide-react";
import { useState } from "react";
import { AGENTS } from "../../lib/agents";
import {
  BUILTIN_TEMPLATES,
  deleteTemplate,
  renameTemplate,
  type SessionTemplate,
  useTemplates,
} from "../../store/templates";
import { AgentLogo } from "../AgentLogo/AgentLogo";
import { IconButton } from "../IconButton/IconButton";
import { field, Section } from "../Settings/controls";

/** Runs a save and reports it in the dialog header (same contract as SettingsDialog's `Save`). */
type Save = (task: Promise<unknown>) => void;

function TemplateRow({ t, save }: { t: SessionTemplate; save: Save }) {
  const [renaming, setRenaming] = useState(false);
  return (
    <div className="flex h-12 items-center gap-3">
      {renaming ? (
        <input
          // biome-ignore lint/a11y/noAutofocus: the user just asked to rename
          autoFocus
          defaultValue={t.name}
          aria-label="Template name"
          className={`${field} flex-1`}
          onBlur={(e) => {
            const name = e.target.value.trim();
            if (name && name !== t.name) save(renameTemplate(t.id, name));
            setRenaming(false);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
            if (e.key === "Escape") {
              e.stopPropagation(); // keep Settings open
              setRenaming(false);
            }
          }}
        />
      ) : (
        <span className="min-w-0 flex-1 truncate text-fg">{t.name}</span>
      )}
      <span className="flex shrink-0 items-center gap-1" title={t.panes.map((p) => p.title).join(", ")}>
        {t.panes.map((p) => (
          <AgentLogo key={p.key} kind={p.agent} size={13} />
        ))}
        <span className="sr-only">{t.panes.map((p) => AGENTS[p.agent].name).join(", ")}</span>
      </span>
      {t.builtin ? (
        <span className="w-[3.25rem] shrink-0 text-right text-xs text-subtle">built-in</span>
      ) : (
        <span className="flex shrink-0">
          <IconButton icon={Pencil} size="sm" label={`Rename ${t.name}`} onClick={() => setRenaming(true)} />
          <IconButton
            icon={Trash2}
            size="sm"
            tone="danger"
            label={`Delete ${t.name}`}
            onClick={() => window.confirm(`Delete template "${t.name}"?`) && save(deleteTemplate(t.id))}
          />
        </span>
      )}
    </div>
  );
}

export function TemplatesSection({ save }: { save: Save }) {
  const list = useTemplates((s) => s.list);
  return (
    <Section title="Session templates">
      <p className="pb-2 text-xs text-subtle">
        Save a session from "New session from template" (sidebar). Folders are stored relative to the workspace.
      </p>
      {[...BUILTIN_TEMPLATES, ...list].map((t) => (
        <TemplateRow key={t.id} t={t} save={save} />
      ))}
    </Section>
  );
}
