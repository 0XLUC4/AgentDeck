import { ChevronDown, Plus, SlidersHorizontal, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { AgentLogo } from "../components/AgentLogo/AgentLogo";
import { lastAgent, spawnAgent } from "../components/AgentPicker/AgentPicker";
import { IconButton } from "../components/IconButton/IconButton";
import { AGENT_KINDS, AGENTS } from "../lib/agents";
import { splitArgs } from "../lib/splitArgs";
import type { AgentKind } from "../lib/types";
import { activeWorkspace, canArchive, deck, type Tab, useDeck } from "../store/deck";
import { archiveSession } from "../store/runtime";
import { useSettings } from "../store/settings";

const isWorking = (paneIds: string[]) => paneIds.some((id) => useDeck.getState().panes[id]?.status === "working");

/** Closes a session tab into the archive, asking first if agents are still working. */
export function closeSession(tabId: string) {
  const tab = useDeck.getState().tabs.find((t) => t.id === tabId);
  if (!tab || !canArchive(useDeck.getState().tabs, tabId)) return;
  if (isWorking(tab.paneIds) && !window.confirm(`Agents in "${tab.name}" are still working. Archive anyway?`)) return;
  archiveSession(tabId);
}

function SessionTab({ tab, active, onRename }: { tab: Tab; active: boolean; onRename: () => void }) {
  const archivable = useDeck((s) => canArchive(s.tabs, tab.id));
  return (
    <div
      className={`group relative flex h-full shrink-0 items-center border-b-2 pr-0.5 transition-colors duration-150 ${
        active ? "border-accent text-fg" : "border-transparent text-muted hover:text-fg"
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
        <span className="max-w-48 truncate">{tab.name}</span>
      </button>
      {archivable && (
        <IconButton
          icon={X}
          size="sm"
          label={`Close and archive ${tab.name}`}
          hint="Close (the session stays in the sidebar and can be reopened)"
          className={active ? "" : "opacity-0 group-focus-within:opacity-100 group-hover:opacity-100"}
          onClick={() => closeSession(tab.id)}
        />
      )}
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
      <div role="tablist" className="scroll-thin -mb-px flex min-w-0 items-center gap-1 self-stretch overflow-x-auto">
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

      <AddAgentButton onOpenPicker={onAddAgent} />
    </nav>
  );
}

const primary =
  "inline-flex h-8 shrink-0 items-center gap-1.5 bg-accent font-medium text-on-accent transition-colors duration-150 hover:bg-accent-hover";

/**
 * Workspace with a folder: one click starts the last used agent there (with its Settings defaults), the arrow picks
 * another one, both without a dialog, so agents can be added in a row. No folder yet: opens the full picker.
 */
export function AddAgentButton({
  onOpenPicker,
  className = "ml-auto",
}: {
  onOpenPicker: () => void;
  className?: string;
}) {
  const path = useDeck((s) => activeWorkspace(s).path);
  const [agent, setAgent] = useState<AgentKind>(lastAgent);
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    menuRef.current?.querySelector("button")?.focus();
    const onPointer = (e: PointerEvent) => menuRef.current?.contains(e.target as Node) || setMenuOpen(false);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setMenuOpen(false);
    document.addEventListener("pointerdown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  if (!path) {
    return (
      <button type="button" onClick={onOpenPicker} className={`${primary} ${className} rounded-md px-3`}>
        <Plus size={15} strokeWidth={1.5} aria-hidden="true" />
        Add agent
      </button>
    );
  }

  const add = (kind: AgentKind) => {
    setMenuOpen(false);
    const defaults = useSettings.getState().agents[kind];
    spawnAgent(kind, path, splitArgs(defaults.args), defaults.worktree);
    setAgent(kind);
  };

  return (
    <div ref={menuRef} className={`relative flex shrink-0 ${className}`}>
      <button
        type="button"
        onClick={() => add(agent)}
        title={`Start ${AGENTS[agent].name} in ${path}`}
        className={`${primary} rounded-l-md pr-2.5 pl-3`}
      >
        <AgentLogo kind={agent} size={15} color="currentColor" />
        Add {AGENTS[agent].name}
      </button>
      <button
        type="button"
        aria-label="Choose agent"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        onClick={() => setMenuOpen((open) => !open)}
        className={`${primary} rounded-r-md border-l border-on-accent/20 px-1.5`}
      >
        <ChevronDown size={15} strokeWidth={1.5} aria-hidden="true" />
      </button>
      {menuOpen && (
        <div
          role="menu"
          aria-label="Add agent"
          className="absolute top-full right-0 z-50 mt-1 flex w-48 flex-col rounded-lg border border-line bg-panel p-1 shadow-lg"
        >
          {AGENT_KINDS.map((kind) => (
            <button
              key={kind}
              type="button"
              role="menuitem"
              onClick={() => add(kind)}
              className="flex h-8 items-center gap-2.5 rounded-md px-2 text-left text-fg transition-colors duration-150 hover:bg-hover focus-visible:bg-hover"
            >
              <AgentLogo kind={kind} size={15} />
              {AGENTS[kind].name}
            </button>
          ))}
          <div className="my-1 border-t border-line" />
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setMenuOpen(false);
              onOpenPicker();
            }}
            className="flex h-8 items-center gap-2.5 rounded-md px-2 text-left text-muted transition-colors duration-150 hover:bg-hover hover:text-fg focus-visible:bg-hover"
          >
            <SlidersHorizontal size={15} strokeWidth={1.5} aria-hidden="true" />
            More options…
          </button>
        </div>
      )}
    </div>
  );
}
