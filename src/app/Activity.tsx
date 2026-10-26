import { Activity, Check, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { AgentLogo } from "../components/AgentLogo/AgentLogo";
import { IconButton } from "../components/IconButton/IconButton";
import { STATUS, StatusDot } from "../components/StatusBadge/StatusBadge";
import { AGENTS } from "../lib/agents";
import { formatSince, useNow } from "../lib/clock";
import {
  type ActivityItem,
  type ActivityKey,
  activityGroups,
  activityKey,
  deck,
  type Pane,
  useDeck,
} from "../store/deck";
import { focusPane } from "../store/runtime";
import { revealPane } from "./Deck";

const GROUP_LABEL: Record<ActivityKey, string> = {
  waiting: "En attente",
  error: "Erreur",
  done: "Terminé",
  working: "En cours",
  idle: "Inactif",
  exited: "Arrêté",
};

const paneName = (pane: Pane) => pane.title || AGENTS[pane.agent].name;

/** Shows the pane's workspace + session, reveals it in its layout and focuses its terminal. */
function jumpTo({ pane, tab }: ActivityItem) {
  useDeck.setState({ activeTabId: tab.id });
  revealPane(tab.id, pane.id);
  focusPane(pane.id);
  // The session becomes visible on the next render; focus the terminal after it.
  requestAnimationFrame(() =>
    document.querySelector<HTMLElement>(`[data-pane-id="${CSS.escape(pane.id)}"] .xterm-helper-textarea`)?.focus(),
  );
}

function Since({ since }: { since: number }) {
  const now = useNow();
  return <span className="shrink-0 text-xs tabular-nums text-subtle">{formatSince(now - since)}</span>;
}

function ActivityRow({ item, active }: { item: ActivityItem; active: boolean }) {
  const { pane, tab, workspace } = item;
  const key = activityKey(pane);
  const status = key === "exited" ? null : key;
  return (
    <li
      className={`group flex items-start border-b border-line transition-colors duration-150 ${
        active ? "bg-raised" : "hover:bg-hover"
      }`}
    >
      <button
        type="button"
        className="flex min-w-0 flex-1 flex-col gap-0.5 py-2 pr-2 pl-3 text-left"
        aria-current={active ? "true" : undefined}
        aria-label={`${paneName(pane)}, ${AGENTS[pane.agent].name}, ${status ? STATUS[status].label : "Arrêté"}, ${workspace.name} › ${tab.name}`}
        onClick={() => jumpTo(item)}
      >
        <span className="flex w-full items-center gap-2">
          <AgentLogo kind={pane.agent} size={14} />
          <span className={`min-w-0 flex-1 truncate font-medium ${active ? "text-fg" : "text-muted"}`}>
            {paneName(pane)}
          </span>
          <Since since={pane.statusSince} />
          {status ? (
            <StatusDot status={status} />
          ) : (
            <span className="size-2 shrink-0 rounded-full border border-line-strong" aria-hidden="true" />
          )}
        </span>
        <span className="truncate pl-[22px] text-xs text-subtle">
          {workspace.name} › {tab.name}
        </span>
        {pane.lastAction && (
          <span className="truncate pl-[22px] text-xs text-muted" title={pane.lastAction}>
            {pane.lastAction}
          </span>
        )}
      </button>
      {key === "done" && (
        <IconButton
          icon={Check}
          size="sm"
          label={`Marquer ${paneName(pane)} comme vu`}
          hint="Vu : repasse l'agent en inactif"
          className="mt-1.5 mr-1.5"
          onClick={() => deck.acknowledge(pane.id)}
        />
      )}
    </li>
  );
}

/** Right column: every agent of every workspace, the ones needing attention first. */
export function ActivityPanel({ onClose }: { onClose: () => void }) {
  const tabs = useDeck((s) => s.tabs);
  const workspaces = useDeck((s) => s.workspaces);
  const panes = useDeck((s) => s.panes);
  const activePaneId = useDeck((s) => s.activePaneId);
  const groups = useMemo(() => activityGroups(tabs, workspaces, panes), [tabs, workspaces, panes]);

  return (
    <aside className="flex w-[260px] shrink-0 flex-col border-l border-line bg-panel" aria-label="Activité des agents">
      <div className="flex h-10 shrink-0 items-center justify-between pr-2 pl-3">
        <h2 className="text-xs font-medium text-subtle">Activité</h2>
        <IconButton icon={X} label="Masquer l'activité" hint="Masquer l'activité (Ctrl+Maj+A)" onClick={onClose} />
      </div>
      <div className="scroll-thin min-h-0 flex-1 overflow-y-auto border-t border-line">
        {groups.length === 0 && <p className="px-3 py-4 text-xs text-subtle">Aucun agent lancé.</p>}
        {groups.map((group) => (
          <section key={group.key} aria-label={GROUP_LABEL[group.key]}>
            <h3 className="flex items-center border-b border-line px-3 pt-3 pb-1 text-xs text-subtle">
              {GROUP_LABEL[group.key]}
              <span className="ml-auto tabular-nums">{group.items.length}</span>
            </h3>
            <ul>
              {group.items.map((item) => (
                <ActivityRow key={item.pane.id} item={item} active={item.pane.id === activePaneId} />
              ))}
            </ul>
          </section>
        ))}
      </div>
    </aside>
  );
}

/** Title-bar toggle with the waiting count (and an error dot), visible even when the panel is closed. */
export function ActivityToggle({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  const waiting = useDeck((s) => Object.values(s.panes).filter((p) => activityKey(p) === "waiting").length);
  const failed = useDeck((s) => Object.values(s.panes).some((p) => activityKey(p) === "error"));
  const [announcement, setAnnouncement] = useState("");

  // Taskbar hint: "AgentDeck (2)" while agents wait for an answer.
  useEffect(() => {
    document.title = waiting > 0 ? `AgentDeck (${waiting})` : "AgentDeck";
  }, [waiting]);

  // Screen readers hear when an agent starts waiting, wherever it is.
  useEffect(
    () =>
      useDeck.subscribe((state, prev) => {
        if (state.panes === prev.panes) return;
        const started = Object.values(state.panes).find(
          (p) => p.status === "waiting" && prev.panes[p.id] && prev.panes[p.id].status !== "waiting",
        );
        if (started) setAnnouncement(`${AGENTS[started.agent].name} attend votre réponse : ${paneName(started)}`);
      }),
    [],
  );

  const summary = [waiting > 0 && `${waiting} en attente`, failed && "erreur"].filter(Boolean).join(", ");
  return (
    <span className="relative inline-flex">
      <IconButton
        icon={Activity}
        label={summary ? `Activité (${summary})` : "Activité"}
        hint={`${open ? "Masquer" : "Afficher"} l'activité de tous les agents (Ctrl+Maj+A)`}
        aria-pressed={open}
        onClick={onToggle}
      />
      {waiting > 0 && (
        <span
          className="pointer-events-none absolute -top-0.5 -right-0.5 min-w-3.5 rounded-full bg-wait px-1 text-center text-[10px] leading-3.5 font-semibold tabular-nums text-on-accent"
          aria-hidden="true"
        >
          {waiting}
        </span>
      )}
      {failed && (
        <span
          className="pointer-events-none absolute -bottom-0.5 -right-0.5 size-2 rounded-full bg-bad ring-2 ring-panel"
          aria-hidden="true"
        />
      )}
      <span className="sr-only" aria-live="polite">
        {announcement}
      </span>
    </span>
  );
}
