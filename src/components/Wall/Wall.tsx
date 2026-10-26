import { LayoutGrid, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { AGENTS } from "../../lib/agents";
import { formatSince, useNow } from "../../lib/clock";
import { type ActivityItem, activeWorkspace, activityKey, useDeck } from "../../store/deck";
import { attachOutput } from "../../store/runtime";
import { AgentLogo } from "../AgentLogo/AgentLogo";
import { IconButton } from "../IconButton/IconButton";
import { STATUS, StatusDot } from "../StatusBadge/StatusBadge";
import { jumpToPane } from "./jump";
import { lastLines, toggleWall, useWall, wallItems } from "./wallState";

/** Raw output kept per tile: enough for ~6 lines even with escape codes. */
const TAIL_CHARS = 8000;

/** Last lines of a pane's output, refreshed at most every 250 ms. */
function useLastLines(paneId: string, generation: number): string[] {
  const [lines, setLines] = useState<string[]>([]);
  useEffect(() => {
    let tail = "";
    let timer: ReturnType<typeof setTimeout> | undefined;
    const flush = () => {
      timer = undefined;
      setLines(lastLines(tail));
    };
    // `generation` changes on restart: re-attach to the fresh buffer.
    void generation;
    const detach = attachOutput(paneId, (data) => {
      tail = (tail + data).slice(-TAIL_CHARS);
      timer ??= setTimeout(flush, 250);
    });
    flush();
    return () => {
      detach();
      clearTimeout(timer);
    };
  }, [paneId, generation]);
  return lines;
}

function Since({ since }: { since: number }) {
  const now = useNow();
  return <span className="shrink-0 text-xs tabular-nums text-subtle">{formatSince(now - since)}</span>;
}

function Tile({ item }: { item: ActivityItem }) {
  const { pane, tab, workspace } = item;
  const lines = useLastLines(pane.id, pane.generation);
  const key = activityKey(pane);
  const status = key === "exited" ? null : key;
  const statusLabel = status ? STATUS[status].label : "Arrêté";
  const name = pane.title || AGENTS[pane.agent].name;
  return (
    <li>
      <button
        type="button"
        onClick={() => jumpToPane(tab.id, pane.id)}
        aria-label={`${name}, ${AGENTS[pane.agent].name}, ${statusLabel}, ${workspace.name} › ${tab.name}`}
        className="flex h-full w-full flex-col gap-1.5 rounded-lg border border-line bg-base p-3 text-left transition-colors duration-150 hover:border-line-strong hover:bg-panel"
        // Waiting for the user: thin amber line on top, like the pane header.
        style={key === "waiting" ? { boxShadow: "inset 0 2px 0 var(--color-wait)" } : undefined}
      >
        <span className="flex w-full items-center gap-2">
          <AgentLogo kind={pane.agent} size={14} />
          <span className="min-w-0 flex-1 truncate font-medium text-fg">{name}</span>
          {status ? (
            <StatusDot status={status} />
          ) : (
            <span className="size-2 shrink-0 rounded-full border border-line-strong" aria-hidden="true" />
          )}
          <span className="shrink-0 text-xs text-muted">{statusLabel}</span>
          <Since since={pane.statusSince} />
        </span>
        <span className="truncate text-xs text-subtle">
          {workspace.name} › {tab.name}
        </span>
        <span className="h-4 truncate text-xs text-muted" title={pane.lastAction ?? undefined}>
          {pane.lastAction}
        </span>
        <pre
          className="h-24 overflow-hidden rounded-md bg-gutter px-2 py-1 font-mono text-[11px] leading-4 whitespace-pre text-muted"
          aria-hidden="true"
        >
          {lines.length > 0 ? lines.join("\n") : <span className="text-subtle">Aucune sortie</span>}
        </pre>
      </button>
    </li>
  );
}

/**
 * Every pane of the workspace (or of all workspaces) as compact tiles. Overlays the session grids instead of
 * replacing them: the real terminals must stay mounted to keep answering ConPTY queries.
 */
export function WallOverlay() {
  const open = useWall((s) => s.open);
  const allWorkspaces = useWall((s) => s.allWorkspaces);
  const tabs = useDeck((s) => s.tabs);
  const workspaces = useDeck((s) => s.workspaces);
  const panes = useDeck((s) => s.panes);
  const workspace = useDeck((s) => activeWorkspace(s));
  const rootRef = useRef<HTMLDivElement>(null);
  const items = useMemo(
    () => (open ? wallItems(tabs, workspaces, panes, allWorkspaces ? null : workspace.id) : []),
    [open, tabs, workspaces, panes, allWorkspaces, workspace.id],
  );

  // Ctrl+Shift+W toggles the wall; capture phase so the terminal never receives it.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey && e.shiftKey && !e.altKey && e.code === "KeyW")) return;
      if (e.target instanceof HTMLElement && e.target.closest("[data-shortcut-recorder]")) return;
      e.preventDefault();
      e.stopPropagation();
      if (!e.repeat) toggleWall();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  // Take focus from the terminal underneath so typing doesn't go to a hidden agent.
  useEffect(() => {
    if (open) rootRef.current?.focus();
  }, [open]);

  if (!open) return null;
  return (
    // z-20: above dockview's layers and the empty-session screen (z-10).
    <section
      ref={rootRef}
      tabIndex={-1}
      aria-label="Mur des agents"
      className="absolute inset-0 z-20 flex flex-col bg-gutter outline-none"
      onKeyDown={(e) => {
        if (e.key !== "Escape") return;
        e.stopPropagation();
        useWall.setState({ open: false });
      }}
    >
      <div className="flex h-10 shrink-0 items-center gap-3 border-b border-line bg-panel pr-2 pl-3">
        <h2 className="text-xs font-medium text-subtle">
          Mur · {allWorkspaces ? "tous les workspaces" : workspace.name}
          <span className="ml-2 tabular-nums">{items.length}</span>
        </h2>
        <label className="ml-auto flex items-center gap-2 text-xs text-muted">
          <input
            type="checkbox"
            checked={allWorkspaces}
            onChange={(e) => useWall.setState({ allWorkspaces: e.target.checked })}
            className="accent-(--color-accent)"
          />
          Tous les workspaces
        </label>
        <IconButton
          icon={X}
          label="Quitter le mur"
          hint="Quitter le mur (Échap, Ctrl+Maj+W)"
          onClick={() => useWall.setState({ open: false })}
        />
      </div>
      <div className="scroll-thin min-h-0 flex-1 overflow-y-auto p-1.5">
        {items.length === 0 ? (
          <p className="px-2 py-4 text-xs text-subtle">Aucun agent lancé.</p>
        ) : (
          <ul className="grid grid-cols-[repeat(auto-fill,minmax(300px,1fr))] gap-1.5">
            {items.map((item) => (
              <Tile key={item.pane.id} item={item} />
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}

/** Title-bar toggle for the wall. */
export function WallToggle() {
  const open = useWall((s) => s.open);
  return (
    <IconButton
      icon={LayoutGrid}
      label="Mode mur"
      hint={`${open ? "Quitter le" : "Afficher le"} mur : tous les agents d'un coup d'œil (Ctrl+Maj+W)`}
      aria-pressed={open}
      onClick={toggleWall}
    />
  );
}
