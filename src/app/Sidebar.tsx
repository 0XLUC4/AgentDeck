import {
  Archive,
  ArchiveRestore,
  ChevronRight,
  Folder,
  FolderOpen,
  FolderPlus,
  GitBranch,
  LayoutTemplate,
  Pencil,
  Plus,
  Trash2,
  Unlink,
} from "lucide-react";
import { useState } from "react";
import { IconButton } from "../components/IconButton/IconButton";
import { StatusDot } from "../components/StatusBadge/StatusBadge";
import { openTemplatePickerFor } from "../components/Templates/TemplatePicker";
import {
  activeWorkspace,
  deck,
  type SavedSession,
  sessionStatus,
  type Tab,
  useDeck,
  type Workspace,
} from "../store/deck";
import { reopenSession, unlinkWorkspace } from "../store/runtime";
import { closeSession } from "./TabBar";

const dateFormat = new Intl.DateTimeFormat("fr-FR", { dateStyle: "short", timeStyle: "short" });
function confirmUnlink(ws: Workspace) {
  const { tabs, archived } = useDeck.getState();
  const count =
    tabs.filter((t) => t.workspaceId === ws.id).length + archived.filter((s) => s.workspaceId === ws.id).length;
  const message = `Délier « ${ws.name} » ?\n\nSes ${count} session(s) seront fermées et oubliées par AgentDeck. Aucun fichier n'est supprimé du disque.`;
  if (window.confirm(message)) unlinkWorkspace(ws.id);
}

function RenameField({
  value,
  label,
  onDone,
}: {
  value: string;
  label: string;
  onDone: (name: string | null) => void;
}) {
  return (
    <input
      // biome-ignore lint/a11y/noAutofocus: the user just asked to rename
      autoFocus
      defaultValue={value}
      aria-label={label}
      className="h-7 min-w-0 flex-1 rounded-md border border-line-strong bg-base px-2 text-fg outline-none focus:border-accent"
      onBlur={(e) => onDone(e.target.value.trim() || null)}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") onDone(null);
      }}
    />
  );
}

const rowBase = "group flex h-8 items-center gap-2 rounded-md pr-1 transition-colors duration-150";

function OpenSessionRow({ tab, active }: { tab: Tab; active: boolean }) {
  const status = useDeck((s) => sessionStatus(tab.paneIds, s.panes));
  return (
    <li className={`${rowBase} pl-7 ${active ? "bg-raised text-fg" : "text-muted hover:bg-hover hover:text-fg"}`}>
      <button
        type="button"
        className="flex h-full min-w-0 flex-1 items-center gap-2 text-left"
        aria-current={active ? "page" : undefined}
        onClick={() => useDeck.setState({ activeTabId: tab.id })}
      >
        {status ? <StatusDot status={status} /> : <span className="size-2 rounded-full border border-line-strong" />}
        <span className="truncate">{tab.name}</span>
        <span className="ml-auto shrink-0 text-xs tabular-nums text-subtle">{tab.paneIds.length || ""}</span>
      </button>
      <IconButton
        icon={Archive}
        size="sm"
        label={`Fermer et archiver ${tab.name}`}
        className="opacity-0 group-focus-within:opacity-100 group-hover:opacity-100"
        onClick={() => closeSession(tab.id)}
      />
    </li>
  );
}

function ArchivedRow({ session }: { session: SavedSession }) {
  return (
    <li className={`${rowBase} pl-7 text-subtle hover:bg-hover hover:text-muted`}>
      <button
        type="button"
        className="flex h-full min-w-0 flex-1 items-center gap-2 text-left"
        title={`Archivée le ${dateFormat.format(session.savedAt)} · ${session.panes.length} agent(s). Cliquer pour rouvrir.`}
        onClick={() => reopenSession(session.id)}
      >
        <Archive size={13} strokeWidth={1.5} className="shrink-0" aria-hidden="true" />
        <span className="truncate">{session.name}</span>
      </button>
      <div className="flex opacity-0 group-focus-within:opacity-100 group-hover:opacity-100">
        <IconButton
          icon={ArchiveRestore}
          size="sm"
          label={`Rouvrir ${session.name}`}
          onClick={() => reopenSession(session.id)}
        />
        <IconButton
          icon={Trash2}
          size="sm"
          tone="danger"
          label={`Supprimer ${session.name}`}
          onClick={() =>
            window.confirm(`Supprimer définitivement « ${session.name} » ?`) && deck.deleteArchived(session.id)
          }
        />
      </div>
    </li>
  );
}

function WorkspaceItem({ ws, active }: { ws: Workspace; active: boolean }) {
  const tabs = useDeck((s) => s.tabs);
  const archived = useDeck((s) => s.archived);
  const activeTabId = useDeck((s) => s.activeTabId);
  const [expanded, setExpanded] = useState(true);
  const [renaming, setRenaming] = useState(false);
  const open = tabs.filter((t) => t.workspaceId === ws.id);
  const closed = archived.filter((s) => s.workspaceId === ws.id);
  const FolderIcon = active ? FolderOpen : Folder;

  return (
    <li>
      <div className={`${rowBase} pl-1 ${active ? "text-fg" : "text-muted hover:bg-hover hover:text-fg"}`}>
        <button
          type="button"
          aria-expanded={expanded}
          aria-label={expanded ? `Replier ${ws.name}` : `Déplier ${ws.name}`}
          className="inline-flex size-5 shrink-0 items-center justify-center rounded text-subtle hover:text-fg"
          onClick={() => setExpanded(!expanded)}
        >
          <ChevronRight
            size={14}
            className={`transition-transform duration-150 motion-reduce:transition-none ${expanded ? "rotate-90" : ""}`}
            aria-hidden="true"
          />
        </button>
        {renaming ? (
          <RenameField
            value={ws.name}
            label="Nom du workspace"
            onDone={(name) => {
              if (name) deck.renameWorkspace(ws.id, name);
              setRenaming(false);
            }}
          />
        ) : (
          <button
            type="button"
            className="flex h-full min-w-0 flex-1 items-center gap-2 text-left"
            title={ws.path || "Pas de dossier lié : les agents démarrent dans le dernier dossier utilisé."}
            onClick={() => deck.selectWorkspace(ws.id)}
            onDoubleClick={() => setRenaming(true)}
          >
            <FolderIcon
              size={15}
              strokeWidth={1.5}
              className={`shrink-0 ${active ? "text-fg" : "text-subtle"}`}
              aria-hidden="true"
            />
            <span className="truncate font-medium">{ws.name}</span>
            {ws.git && (
              <span
                className="inline-flex min-w-0 shrink items-center gap-1 text-xs text-subtle group-hover:hidden"
                title={ws.git.branch ? `Branche ${ws.git.branch} (au moment de la liaison)` : "Dépôt Git"}
              >
                <GitBranch size={12} strokeWidth={1.5} className="shrink-0" aria-hidden="true" />
                <span className="truncate">{ws.git.branch ?? "Git"}</span>
              </span>
            )}
          </button>
        )}
        {!renaming && (
          <div className="flex opacity-0 group-focus-within:opacity-100 group-hover:opacity-100">
            <IconButton
              icon={Plus}
              size="sm"
              label={`Nouvelle session dans ${ws.name}`}
              onClick={() => {
                deck.selectWorkspace(ws.id);
                deck.addTab();
                setExpanded(true);
              }}
            />
            <IconButton
              icon={LayoutTemplate}
              size="sm"
              label={`Nouvelle session depuis un modèle dans ${ws.name}`}
              onClick={() => {
                openTemplatePickerFor(ws.id);
                setExpanded(true);
              }}
            />
            <IconButton icon={Pencil} size="sm" label={`Renommer ${ws.name}`} onClick={() => setRenaming(true)} />
            <IconButton
              icon={Unlink}
              size="sm"
              tone="danger"
              label={`Délier ${ws.name}`}
              hint="Délier ce workspace (aucun fichier supprimé)"
              onClick={() => confirmUnlink(ws)}
            />
          </div>
        )}
      </div>
      {expanded && (
        <ul className="mt-0.5 flex flex-col gap-px" aria-label={`Sessions de ${ws.name}`}>
          {open.map((tab) => (
            <OpenSessionRow key={tab.id} tab={tab} active={tab.id === activeTabId} />
          ))}
          {closed.map((session) => (
            <ArchivedRow key={session.id} session={session} />
          ))}
        </ul>
      )}
    </li>
  );
}

/** Left column: linked workspaces, each with its open and archived sessions. */
export function Sidebar({ onLink }: { onLink: () => void }) {
  const workspaces = useDeck((s) => s.workspaces);
  const activeId = useDeck((s) => activeWorkspace(s).id);

  return (
    <aside className="flex w-64 shrink-0 flex-col border-r border-line bg-panel" aria-label="Workspaces et sessions">
      <div className="flex h-10 shrink-0 items-center justify-between pr-2 pl-3">
        <h2 className="text-xs font-medium text-subtle">Workspaces</h2>
        <IconButton icon={FolderPlus} label="Lier un workspace" onClick={onLink} />
      </div>
      <ul className="scroll-thin flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-2 pb-3">
        {workspaces.map((ws) => (
          <WorkspaceItem key={ws.id} ws={ws} active={ws.id === activeId} />
        ))}
      </ul>
      <button
        type="button"
        onClick={onLink}
        className="m-2 inline-flex h-8 items-center gap-2 rounded-md px-2 text-subtle transition-colors duration-150 hover:bg-hover hover:text-fg"
      >
        <Plus size={16} strokeWidth={1.5} aria-hidden="true" />
        Lier un workspace
      </button>
    </aside>
  );
}
