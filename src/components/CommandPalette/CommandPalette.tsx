import {
  Activity,
  Archive,
  Crown,
  ExternalLink,
  Folder,
  FolderPlus,
  Gauge,
  Grid2x2,
  History,
  Layers,
  LayoutGrid,
  LayoutTemplate,
  type LucideIcon,
  Maximize2,
  Megaphone,
  NotebookText,
  PanelLeft,
  Pause,
  Plus,
  RotateCcw,
  Search,
  Settings,
  SquarePlus,
  SquareStop,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { type KeyboardEvent as ReactKeyboardEvent, type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { arrangePanesInGrid, toggleMaximizePane } from "../../app/Deck";
import { confirmRestartAll, confirmStopAll } from "../../app/TitleBar";
import { AGENTS } from "../../lib/agents";
import { checkpointPane } from "../../lib/checkpoints";
import { openDialog } from "../../lib/dialogs";
import { createOrchestrator, setOrchestratorPaused, useOrchestrator } from "../../lib/orchestrator";
import { activityKey, deck, useDeck } from "../../store/deck";
import { interruptAll, reopenSession } from "../../store/runtime";
import { AgentLogo } from "../AgentLogo/AgentLogo";
import { openBroadcast } from "../Broadcast/Broadcast";
import { detachPane } from "../Detached/host";
import { STATUS, StatusDot } from "../StatusBadge/StatusBadge";
import { openTemplatePicker } from "../Templates/TemplatePicker";
import { currentPane, jumpToPane } from "../Wall/jump";
import { toggleWall, useWall } from "../Wall/wallState";
import { fuzzyScore } from "./fuzzy";

/** Callbacks owned by App (dialogs and panels it renders). */
export interface PaletteCallbacks {
  onAddAgent: () => void;
  onLinkWorkspace: () => void;
  onOpenSettings: () => void;
  onToggleSidebar: () => void;
  onToggleActivity: () => void;
}

type Section = "Recent" | "Agents" | "Sessions" | "Workspaces" | "Actions";
const SECTIONS: Section[] = ["Recent", "Agents", "Sessions", "Workspaces", "Actions"];

interface Item {
  id: string;
  section: Exclude<Section, "Recent">;
  label: string;
  detail?: string;
  icon: ReactNode;
  /** Status dot (agents) or shortcut hint, shown on the right. */
  trailing?: ReactNode;
  run: () => void;
}

/** Recently run item ids, most recent first. In memory: resets with the app. */
let recent: string[] = [];

const icon = (Icon: LucideIcon) => <Icon size={15} strokeWidth={1.5} aria-hidden="true" className="text-subtle" />;
const kbd = (keys: string) => <kbd className="font-sans text-xs text-subtle">{keys}</kbd>;

function onCurrentPane(fn: (tabId: string, paneId: string) => void) {
  const target = currentPane();
  if (target) fn(target.tabId, target.paneId);
}

function actionItems(cb: PaletteCallbacks): Item[] {
  const actions: [string, string, LucideIcon, () => void, string?][] = [
    ["add-agent", "Add agent", Plus, cb.onAddAgent],
    ["new-session", "New session", SquarePlus, () => deck.addTab()],
    ["link-workspace", "Link workspace", FolderPlus, cb.onLinkWorkspace],
    ["interrupt-all", "Interrupt all", Pause, interruptAll],
    ["stop-all", "Stop all", SquareStop, confirmStopAll],
    ["restart-all", "Restart all", RotateCcw, confirmRestartAll],
    ["settings", "Open settings", Settings, cb.onOpenSettings, "Ctrl+,"],
    ["sidebar", "Toggle sidebar", PanelLeft, cb.onToggleSidebar],
    ["activity", "Toggle activity", Activity, cb.onToggleActivity, "Ctrl+Shift+A"],
    ["wall", "Wall mode", LayoutGrid, toggleWall, "Ctrl+Shift+W"],
    ["maximize", "Maximize/restore active pane", Maximize2, () => onCurrentPane(toggleMaximizePane)],
    ["grid", "Arrange panes in a grid", Grid2x2, () => arrangePanesInGrid(useDeck.getState().activeTabId)],
    ["zoom-in", "Zoom +", ZoomIn, () => onCurrentPane((_, id) => deck.zoom(id, 1)), "Ctrl+="],
    ["zoom-out", "Zoom -", ZoomOut, () => onCurrentPane((_, id) => deck.zoom(id, -1)), "Ctrl+-"],
    ["broadcast", "Broadcast prompt", Megaphone, openBroadcast, "Ctrl+Shift+B"],
    ["templates", "New session from template", LayoutTemplate, openTemplatePicker],
    [
      "checkpoint",
      "Create restore point (active pane)",
      History,
      () => onCurrentPane((_, id) => void checkpointPane(id)),
    ],
    ["detach", "Detach active pane", ExternalLink, () => onCurrentPane((_, id) => void detachPane(id))],
    ["usage", "Usage", Gauge, () => openDialog("usage")],
    ["history", "History", History, () => openDialog("history"), "Ctrl+Shift+F"],
    ["journal", "Today's journal", NotebookText, () => openDialog("journal")],
    ["orchestrator", "New orchestrator", Crown, createOrchestrator],
    [
      "orchestrator-pause",
      "Pause/resume orchestrator",
      Pause,
      () => void setOrchestratorPaused(!useOrchestrator.getState().paused),
    ],
  ];
  return actions.map(([id, label, Icon, run, shortcut]) => ({
    id: `action:${id}`,
    section: "Actions",
    label,
    detail: id.endsWith("-all") ? "Active session" : undefined,
    icon: icon(Icon),
    trailing: shortcut ? kbd(shortcut) : undefined,
    run,
  }));
}

/** Every jump target and action, in section order. Reads the store once when the palette opens. */
function buildItems(cb: PaletteCallbacks): Item[] {
  const { tabs, workspaces, panes, archived } = useDeck.getState();
  const wsName = (id: string) => workspaces.find((w) => w.id === id)?.name ?? "";
  const agents: Item[] = tabs.flatMap((tab) =>
    tab.paneIds.flatMap((id) => {
      const pane = panes[id];
      if (!pane) return [];
      const key = activityKey(pane);
      return [
        {
          id: `pane:${id}`,
          section: "Agents" as const,
          label: pane.title || AGENTS[pane.agent].name,
          detail: `${wsName(tab.workspaceId)} › ${tab.name}`,
          icon: <AgentLogo kind={pane.agent} size={15} />,
          trailing:
            key === "exited" ? (
              <span className="size-2 shrink-0 rounded-full border border-line-strong" title="Stopped" />
            ) : (
              <span title={STATUS[key].label} className="inline-flex">
                <StatusDot status={key} />
              </span>
            ),
          run: () => jumpToPane(tab.id, id),
        },
      ];
    }),
  );
  const sessions: Item[] = [
    ...tabs.map((tab) => ({
      id: `tab:${tab.id}`,
      section: "Sessions" as const,
      label: tab.name,
      detail: wsName(tab.workspaceId),
      icon: icon(Layers),
      run: () => {
        useWall.setState({ open: false });
        useDeck.setState({ activeTabId: tab.id });
      },
    })),
    ...archived.map((session) => ({
      id: `archived:${session.id}`,
      section: "Sessions" as const,
      label: session.name,
      detail: `${wsName(session.workspaceId)} · archived`,
      icon: icon(Archive),
      run: () => {
        useWall.setState({ open: false });
        reopenSession(session.id);
      },
    })),
  ];
  const workspaceItems: Item[] = workspaces.map((ws) => ({
    id: `ws:${ws.id}`,
    section: "Workspaces",
    label: ws.name,
    detail: ws.path || undefined,
    icon: icon(Folder),
    run: () => deck.selectWorkspace(ws.id),
  }));
  return [...agents, ...sessions, ...workspaceItems, ...actionItems(cb)];
}

/** Items grouped for display: recents first when the query is empty, fuzzy-ranked within sections otherwise. */
function filterItems(items: Item[], query: string): { section: Section; items: Item[] }[] {
  if (!query.trim()) {
    const recents = recent.flatMap((id) => items.filter((i) => i.id === id));
    const rest = items.filter((i) => !recent.includes(i.id));
    return [
      { section: "Recent" as Section, items: recents },
      ...SECTIONS.slice(1).map((section) => ({ section, items: rest.filter((i) => i.section === section) })),
    ].filter((g) => g.items.length > 0);
  }
  const scored = items.flatMap((item) => {
    const onLabel = fuzzyScore(query, item.label);
    const score = onLabel ?? (fuzzyScore(query, `${item.label} ${item.detail ?? ""}`) ?? Number.NaN) - 5;
    return Number.isNaN(score) ? [] : [{ item, score }];
  });
  return SECTIONS.slice(1)
    .map((section) => ({
      section,
      items: scored
        .filter((s) => s.item.section === section)
        .sort((a, b) => b.score - a.score)
        .map((s) => s.item),
    }))
    .filter((g) => g.items.length > 0);
}

const optionId = (index: number) => `palette-option-${index}`;

function PaletteDialog({ callbacks, onClose }: { callbacks: PaletteCallbacks; onClose: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const items = useMemo(() => buildItems(callbacks), [callbacks]);
  const groups = useMemo(() => filterItems(items, query), [items, query]);
  const flat = useMemo(() => groups.flatMap((g) => g.items), [groups]);
  const sectionStarts = useMemo(() => {
    let index = 0;
    return groups.map((g) => {
      const start = index;
      index += g.items.length;
      return start;
    });
  }, [groups]);

  useEffect(() => {
    dialogRef.current?.showModal();
  }, []);

  useEffect(() => {
    document.getElementById(optionId(active))?.scrollIntoView({ block: "nearest" });
  }, [active]);

  function run(item: Item | undefined) {
    if (!item) return;
    recent = [item.id, ...recent.filter((id) => id !== item.id)].slice(0, 5);
    onClose();
    item.run();
  }

  function onKeyDown(e: ReactKeyboardEvent) {
    const count = flat.length;
    if (e.key === "Escape") {
      // Keeps Deck's Esc (restore maximized pane) from also firing.
      e.stopPropagation();
      return;
    }
    if (count === 0) return;
    let next: number | null = null;
    if (e.key === "ArrowDown") next = (active + 1) % count;
    else if (e.key === "ArrowUp") next = (active - 1 + count) % count;
    else if (e.key === "Home" && e.ctrlKey) next = 0;
    else if (e.key === "End" && e.ctrlKey) next = count - 1;
    else if (e.key === "Tab") {
      // Tab / Shift+Tab: first item of the next / previous section.
      const current = sectionStarts.filter((start) => start <= active).length - 1;
      const target = (current + (e.shiftKey ? -1 : 1) + groups.length) % groups.length;
      next = sectionStarts[target];
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(flat[active]);
      return;
    }
    if (next === null) return;
    e.preventDefault();
    setActive(next);
  }

  return (
    <dialog
      ref={dialogRef}
      onClose={onClose}
      aria-label="Command palette"
      className="mx-auto mt-[12vh] w-[min(38rem,92vw)] overflow-hidden rounded-xl border border-line bg-panel p-0 text-fg backdrop:bg-gutter/70"
    >
      <div className="flex items-center gap-2 border-b border-line px-3">
        <Search size={16} strokeWidth={1.5} aria-hidden="true" className="shrink-0 text-subtle" />
        <input
          autoFocus
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-listbox"
          aria-autocomplete="list"
          aria-activedescendant={flat.length > 0 ? optionId(active) : undefined}
          aria-label="Search for an agent, session, workspace or action"
          placeholder="Agent, session, workspace, action…"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(0);
          }}
          onKeyDown={onKeyDown}
          className="h-11 min-w-0 flex-1 bg-transparent text-fg outline-none placeholder:text-subtle"
        />
        {kbd("Esc")}
      </div>
      <div
        id="palette-listbox"
        role="listbox"
        aria-label="Results"
        className="scroll-thin max-h-[60vh] overflow-y-auto py-1"
      >
        {flat.length === 0 && <p className="px-3 py-4 text-xs text-subtle">No results.</p>}
        {groups.map((group, g) => (
          // biome-ignore lint/a11y/useSemanticElements: ARIA listbox groups; a fieldset is not valid inside a listbox
          <div key={group.section} role="group" aria-labelledby={`palette-section-${g}`}>
            <div id={`palette-section-${g}`} role="presentation" className="px-3 pt-2 pb-1 text-xs text-subtle">
              {group.section}
            </div>
            {group.items.map((item, i) => {
              const index = sectionStarts[g] + i;
              const selected = index === active;
              return (
                // biome-ignore lint/a11y/useKeyWithClickEvents: keyboard goes through the combobox (aria-activedescendant)
                <div
                  key={item.id}
                  id={optionId(index)}
                  role="option"
                  aria-selected={selected}
                  tabIndex={-1}
                  className={`mx-1 flex h-9 cursor-default items-center gap-2.5 rounded-md px-2 ${
                    selected ? "bg-raised text-fg" : "text-muted"
                  }`}
                  onMouseMove={() => setActive(index)}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => run(item)}
                >
                  <span className="flex size-4 shrink-0 items-center justify-center">{item.icon}</span>
                  <span className="truncate">{item.label}</span>
                  {item.detail && <span className="min-w-0 flex-1 truncate text-xs text-subtle">{item.detail}</span>}
                  <span className="ml-auto flex shrink-0 items-center">{item.trailing}</span>
                </div>
              );
            })}
          </div>
        ))}
      </div>
    </dialog>
  );
}

/** Ctrl+K / Ctrl+Shift+P: jump to any agent, session or workspace, or run any action. */
export function CommandPalette(callbacks: PaletteCallbacks) {
  const [open, setOpen] = useState(false);

  // Capture phase so the terminal never receives the shortcut.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const palette =
        e.ctrlKey && !e.altKey && ((!e.shiftKey && e.code === "KeyK") || (e.shiftKey && e.code === "KeyP"));
      if (!palette) return;
      if (e.target instanceof HTMLElement && e.target.closest("[data-shortcut-recorder]")) return;
      e.preventDefault();
      e.stopPropagation();
      if (!e.repeat) setOpen((o) => !o);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  return open ? <PaletteDialog callbacks={callbacks} onClose={() => setOpen(false)} /> : null;
}
