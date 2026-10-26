import { Plus, X } from "lucide-react";
import { useState } from "react";
import { IconButton } from "../components/IconButton/IconButton";
import { StatusDot } from "../components/StatusBadge/StatusBadge";
import { activeWorkspace, deck, sessionStatus, type Tab, useDeck } from "../store/deck";
import { archiveSession } from "../store/runtime";

const isWorking = (paneIds: string[]) => paneIds.some((id) => useDeck.getState().panes[id]?.status === "working");

/** Closes a session tab into the archive, asking first if agents are still working. */
export function closeSession(tabId: string) {
  const tab = useDeck.getState().tabs.find((t) => t.id === tabId);
  if (!tab) return;
  if (isWorking(tab.paneIds) && !window.confirm(`Agents in "${tab.name}" are still working. Archive anyway?`)) return;
  archiveSession(tabId);
}

function SessionTab({ tab, active, onRename }: { tab: Tab; active: boolean; onRename: () => void }) {
  const status = useDeck((s) => sessionStatus(tab.paneIds, s.panes));
  return (
    <div
      className={`group relative flex h-8 shrink-0 items-center rounded-md pr-0.5 transition-colors duration-150 ${
        active ? "bg-raised text-fg" : "text-muted hover:bg-hover hover:text-fg"
      }`}
    >
      <button
        type="button"
        role="tab"
        aria-selected={active}
        title="Double-click or F2 to rename"
        className="flex h-full items-center gap-2 pr-1 pl-3"
        onClick={() => useDeck.setState({ activeTabId: tab.id })}
        onDoubleClick={onRename}
        onKeyDown={(e) => {
          if (e.key === "F2") onRename();
        }}
      >
        {status && <StatusDot status={status} />}
        <span className="max-w-48 truncate">{tab.name}</span>
      </button>
      <IconButton
        icon={X}
        size="sm"
        label={`Close and archive ${tab.name}`}
        hint="Close (the session stays in the sidebar and can be reopened)"
        className={active ? "" : "opacity-0 group-focus-within:opacity-100 group-hover:opacity-100"}
        onClick={() => closeSession(tab.id)}
      />
    </div>
  );
}

/** Open sessions of the current workspace, plus "new session" and the primary "add agent" action. */
export function TabBar({ onAddAgent }: { onAddAgent: () => void }) {
  const workspaceId = useDeck((s) => activeWorkspace(s).id);
  const allTabs = useDeck((s) => s.tabs);
  const activeTabId = useDeck((s) => s.activeTabId);
  const [editing, setEditing] = useState<string | null>(null);
  const tabs = allTabs.filter((t) => t.workspaceId === workspaceId);

  return (
    <nav className="flex h-10 shrink-0 items-center gap-2 border-b border-line bg-panel px-2" aria-label="Sessions">
      <div role="tablist" className="scroll-thin flex min-w-0 items-center gap-1 overflow-x-auto">
        {tabs.map((tab) =>
          editing === tab.id ? (
            <input
              key={tab.id}
              // biome-ignore lint/a11y/noAutofocus: the user just asked to rename this tab
              autoFocus
              defaultValue={tab.name}
              aria-label="Session name"
              className="h-8 w-40 shrink-0 rounded-md border border-line-strong bg-base px-2.5 text-fg outline-none focus:border-accent"
              onBlur={(e) => {
                deck.renameTab(tab.id, e.target.value.trim() || tab.name);
                setEditing(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") e.currentTarget.blur();
                if (e.key === "Escape") setEditing(null);
              }}
            />
          ) : (
            <SessionTab key={tab.id} tab={tab} active={tab.id === activeTabId} onRename={() => setEditing(tab.id)} />
          ),
        )}
      </div>
      <IconButton icon={Plus} label="New session" onClick={() => deck.addTab()} />

      <button
        type="button"
        onClick={onAddAgent}
        className="ml-auto inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md bg-accent px-3 font-medium text-on-accent transition-colors duration-150 hover:bg-accent-hover"
      >
        <Plus size={15} strokeWidth={1.5} aria-hidden="true" />
        Add agent
      </button>
    </nav>
  );
}
