import { Activity, Check, EyeOff, SquareTerminal, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { AgentLogo } from "../components/AgentLogo/AgentLogo";
import { IconButton } from "../components/IconButton/IconButton";
import { OrchestratorCrown } from "../components/Orchestrator/OrchestratorButton";
import { PermissionCard } from "../components/Permission/PermissionCard";
import { permissionsForPane, usePermissions } from "../components/Permission/permissions";
import { STATUS, StatusDot } from "../components/StatusBadge/StatusBadge";
import { AGENTS } from "../lib/agents";
import { formatSince, useNow } from "../lib/clock";
import type { PermissionRequest } from "../lib/types";
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
import { TaskDetail } from "./TaskDetail";

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
function jumpTo({ pane, tab }: Pick<ActivityItem, "pane" | "tab">) {
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

function ActivityRow({
  item,
  active,
  requests,
  onDetail,
}: {
  item: ActivityItem;
  active: boolean;
  requests: PermissionRequest[];
  onDetail: () => void;
}) {
  const { pane, tab, workspace } = item;
  const key = activityKey(pane);
  const status = key === "exited" ? null : key;
  return (
    <li
      className={`group border-b border-line transition-colors duration-150 ${active ? "bg-raised" : "hover:bg-hover"}`}
    >
      <div className="flex items-start">
        <button
          type="button"
          className="flex min-w-0 flex-1 flex-col gap-0.5 py-2 pr-2 pl-3 text-left"
          aria-current={active ? "true" : undefined}
          aria-label={`${paneName(pane)}, ${AGENTS[pane.agent].name}, ${status ? STATUS[status].label : "Arrêté"}, ${workspace.name} › ${tab.name}`}
          onClick={() => jumpTo(item)}
        >
          <span className="flex w-full items-center gap-2">
            <AgentLogo kind={pane.agent} size={14} />
            {pane.orchestrator && <OrchestratorCrown size={12} />}
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
          <span className="mt-1.5 mr-1.5 flex flex-col items-center">
            <IconButton
              icon={Check}
              size="sm"
              label={`Voir ce que ${paneName(pane)} a terminé`}
              hint="Voir ce qu'il a terminé"
              onClick={onDetail}
            />
            <IconButton
              icon={EyeOff}
              size="sm"
              label={`Ignorer ${paneName(pane)}`}
              hint="Ignorer : repasse l'agent en inactif sans ouvrir le détail"
              className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
              onClick={() => deck.acknowledge(pane.id)}
            />
          </span>
        )}
        {/* Agents without a permission hook (Codex, Gemini, Grok, Claude prompts we can't answer): answer in the terminal. */}
        {key === "waiting" && requests.length === 0 && (
          <span className="mt-1.5 mr-1.5">
            <IconButton
              icon={SquareTerminal}
              size="sm"
              label={`Aller au terminal de ${paneName(pane)}`}
              hint="Aller au terminal pour répondre"
              onClick={() => jumpTo(item)}
            />
          </span>
        )}
      </div>
      {requests.length > 0 && (
        <div className="flex flex-col gap-1.5 px-2 pb-2">
          {requests.map((r) => (
            <PermissionCard key={r.requestId} request={r} onJump={() => jumpTo(item)} />
          ))}
        </div>
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
  const pending = usePermissions((s) => s);
  const groups = useMemo(() => activityGroups(tabs, workspaces, panes), [tabs, workspaces, panes]);
  const [detailId, setDetailId] = useState<string | null>(null);
  const detail = groups.flatMap((g) => g.items).find((i) => i.pane.id === detailId);

  return (
    <aside className="flex w-[260px] shrink-0 flex-col border-l border-line bg-panel" aria-label="Activité des agents">
      <div className="flex h-10 shrink-0 items-center justify-between pr-2 pl-3">
        <h2 className="text-xs font-medium text-subtle">Activité</h2>
        <IconButton icon={X} label="Masquer l'activité" hint="Masquer l'activité (Ctrl+Maj+A)" onClick={onClose} />
      </div>
      <div className="relative flex min-h-0 flex-1 flex-col border-t border-line">
        {detail && <TaskDetail item={detail} onJump={() => jumpTo(detail)} onClose={() => setDetailId(null)} />}
        <div className="scroll-thin min-h-0 flex-1 overflow-y-auto">
          {groups.length === 0 && <p className="px-3 py-4 text-xs text-subtle">Aucun agent lancé.</p>}
          {groups.map((group) => (
            <section key={group.key} aria-label={GROUP_LABEL[group.key]}>
              <h3 className="flex items-center border-b border-line px-3 pt-3 pb-1 text-xs text-subtle">
                {GROUP_LABEL[group.key]}
                <span className="ml-auto tabular-nums">{group.items.length}</span>
              </h3>
              <ul>
                {group.items.map((item) => (
                  <ActivityRow
                    key={item.pane.id}
                    item={item}
                    active={item.pane.id === activePaneId}
                    requests={permissionsForPane(pending, item.pane.id)}
                    onDetail={() => setDetailId(item.pane.id)}
                  />
                ))}
              </ul>
            </section>
          ))}
        </div>
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

/** Shown while the activity panel is closed: the oldest pending permission, answerable in place. */
export function PermissionBanner() {
  const pending = usePermissions((s) => s);
  const requests = Object.values(pending);
  const request = requests[0];
  const pane = useDeck((s) => (request ? s.panes[request.paneId] : undefined));
  const tab = useDeck((s) => (request ? s.tabs.find((t) => t.paneIds.includes(request.paneId)) : undefined));
  if (!request || !pane) return null;
  const more = requests.length - 1;
  return (
    <section
      aria-label={`${AGENTS[pane.agent].name} demande l'autorisation`}
      className="fixed top-12 right-4 z-40 flex w-[340px] flex-col gap-1.5 rounded-lg border border-line-strong bg-raised p-2.5 shadow-lg"
    >
      <p className="flex items-center gap-2 text-xs text-muted">
        <AgentLogo kind={pane.agent} size={14} />
        <span className="min-w-0 flex-1 truncate">
          <span className="font-medium text-fg">{paneName(pane)}</span> demande l'autorisation
        </span>
        {more > 0 && <span className="shrink-0 tabular-nums text-subtle">+{more}</span>}
      </p>
      <PermissionCard
        key={request.requestId}
        request={request}
        onJump={tab ? () => jumpTo({ pane, tab }) : undefined}
      />
    </section>
  );
}
