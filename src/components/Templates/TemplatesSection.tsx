// Réglages > Modèles: rename or delete saved session templates (builtins are read-only).
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
          aria-label="Nom du modèle"
          className={`${field} flex-1`}
          onBlur={(e) => {
            const name = e.target.value.trim();
            if (name && name !== t.name) save(renameTemplate(t.id, name));
            setRenaming(false);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
            if (e.key === "Escape") {
              e.stopPropagation(); // keep Réglages open
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
        <span className="w-[3.25rem] shrink-0 text-right text-xs text-subtle">intégré</span>
      ) : (
        <span className="flex shrink-0">
          <IconButton icon={Pencil} size="sm" label={`Renommer ${t.name}`} onClick={() => setRenaming(true)} />
          <IconButton
            icon={Trash2}
            size="sm"
            tone="danger"
            label={`Supprimer ${t.name}`}
            onClick={() => window.confirm(`Supprimer le modèle « ${t.name} » ?`) && save(deleteTemplate(t.id))}
          />
        </span>
      )}
    </div>
  );
}

export function TemplatesSection({ save }: { save: Save }) {
  const list = useTemplates((s) => s.list);
  return (
    <Section title="Modèles de session">
      <p className="pb-2 text-xs text-subtle">
        Enregistrez une session depuis « Nouvelle session depuis un modèle » (barre latérale). Les dossiers sont
        mémorisés relativement au workspace.
      </p>
      {[...BUILTIN_TEMPLATES, ...list].map((t) => (
        <TemplateRow key={t.id} t={t} save={save} />
      ))}
    </Section>
  );
}
