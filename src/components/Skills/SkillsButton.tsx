// Pane header button + dialog listing the agent's skills (user and project): install from GitHub, edit, uninstall.
import { Download, Pencil, Puzzle, Trash2 } from "lucide-react";
import { type FormEvent, useCallback, useEffect, useState } from "react";
import { type SkillInfo, type SkillScope, skillsInstall, skillsList, skillsRemove } from "../../lib/ipc";
import type { Pane } from "../../store/deck";
import { IconButton } from "../IconButton/IconButton";
import { AgentDialog, FileEditor } from "./AgentDialog";

const SCOPE_LABEL: Record<SkillScope, string> = { user: "All projects", project: "This project" };

function SkillsDialog({ pane, onClose }: { pane: Pane; onClose: () => void }) {
  const [skills, setSkills] = useState<SkillInfo[] | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [url, setUrl] = useState("");
  const [scope, setScope] = useState<SkillScope>("user");
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");

  const refresh = useCallback(
    () =>
      skillsList(pane.agent, pane.cwd)
        .then(setSkills)
        .catch((err) => setStatus(String(err))),
    [pane.agent, pane.cwd],
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function install(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setStatus("Downloading…");
    try {
      const names = await skillsInstall(pane.agent, pane.cwd, url.trim(), scope);
      setStatus(`Installed: ${names.join(", ")}. Restart the agent if it does not see them yet.`);
      setUrl("");
      await refresh();
    } catch (err) {
      setStatus(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function remove(skill: SkillInfo) {
    if (!window.confirm(`Uninstall the skill "${skill.name}"? Its folder is deleted.`)) return;
    try {
      await skillsRemove(pane.agent, pane.cwd, skill.path);
      setStatus(`Removed ${skill.name}.`);
      await refresh();
    } catch (err) {
      setStatus(String(err));
    }
  }

  return (
    <AgentDialog
      pane={pane}
      title="Skills"
      onClose={onClose}
      status={status || `${skills?.length ?? 0} skill(s). Plugin and bundled skills are managed in the agent itself.`}
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
      ) : (
        <>
          <form onSubmit={install} className="flex items-center gap-2">
            <input
              // biome-ignore lint/a11y/noAutofocus: the dialog opens to install a skill
              autoFocus
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://github.com/owner/repo (or …/tree/main/skills/name)"
              aria-label="GitHub link"
              className="h-8 min-w-0 flex-1 rounded-md border border-line bg-base px-2.5 text-fg outline-none transition-colors duration-150 placeholder:text-subtle hover:border-line-strong focus:border-accent"
            />
            <select
              value={scope}
              onChange={(e) => setScope(e.target.value as SkillScope)}
              aria-label="Install for"
              className="h-8 rounded-md border border-line bg-base px-2 text-fg outline-none focus:border-accent"
            >
              <option value="user">{SCOPE_LABEL.user}</option>
              <option value="project">{SCOPE_LABEL.project}</option>
            </select>
            <button
              type="submit"
              disabled={busy || !url.trim()}
              className="inline-flex h-8 items-center gap-1.5 rounded-md bg-accent px-3 font-medium text-on-accent transition-colors duration-150 hover:bg-accent-hover disabled:pointer-events-none disabled:opacity-40"
            >
              <Download size={14} strokeWidth={1.5} aria-hidden="true" />
              Install
            </button>
          </form>

          {skills === null ? (
            <p className="py-3 text-center text-muted">Loading…</p>
          ) : skills.length === 0 ? (
            <p className="py-3 text-center text-muted">No skills installed.</p>
          ) : (
            <ul className="scroll-thin flex max-h-96 flex-col overflow-y-auto rounded-md border border-line">
              {skills.map((skill) => (
                <li
                  key={skill.path}
                  className="flex items-start gap-2 border-b border-line py-1.5 pr-1 pl-3 last:border-b-0"
                >
                  <div className="min-w-0 flex-1 pt-0.5" title={skill.path}>
                    <div className="flex items-center gap-2">
                      <span className="truncate font-medium text-fg">{skill.name}</span>
                      <span className="shrink-0 rounded bg-base px-1.5 text-[11px] text-subtle">
                        {SCOPE_LABEL[skill.scope]}
                      </span>
                    </div>
                    {skill.description && <p className="line-clamp-2 text-xs text-muted">{skill.description}</p>}
                  </div>
                  <IconButton
                    icon={Pencil}
                    size="sm"
                    label={`Edit ${skill.name}`}
                    onClick={() => setEditing(skill.path)}
                  />
                  <IconButton
                    icon={Trash2}
                    size="sm"
                    tone="danger"
                    label={`Uninstall ${skill.name}`}
                    onClick={() => void remove(skill)}
                  />
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </AgentDialog>
  );
}

/** Header button (agent panes) opening the skills manager. */
export function SkillsButton({ pane }: { pane: Pane }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <IconButton
        icon={Puzzle}
        size="sm"
        label="Skills"
        hint="Skills: see, edit, install (GitHub link), uninstall"
        onClick={() => setOpen(true)}
      />
      {open && <SkillsDialog pane={pane} onClose={() => setOpen(false)} />}
    </>
  );
}
