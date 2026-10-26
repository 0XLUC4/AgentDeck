import { useState } from "react";
import { activeTab, deck, useDeck } from "../store/deck";
import { archiveSession, interruptAll, restartAll, stopAll } from "../store/runtime";
import { SessionsMenu } from "./SessionsMenu";

const isWorking = (paneIds: string[]) => paneIds.some((id) => useDeck.getState().panes[id]?.status === "working");

function closeTab(tabId: string) {
  const tab = useDeck.getState().tabs.find((t) => t.id === tabId);
  if (!tab) return;
  if (
    isWorking(tab.paneIds) &&
    !window.confirm(`Des agents de « ${tab.name} » travaillent encore. Archiver quand même ?`)
  )
    return;
  archiveSession(tabId);
}

function confirmStopAll() {
  const count = activeTab()?.paneIds.length ?? 0;
  if (count > 0 && window.confirm(`Arrêter les ${count} agent(s) de cette session ?`)) void stopAll();
}

const ctrl = "shrink-0 rounded-md px-2 py-1 text-sm text-zinc-300 hover:bg-zinc-800 hover:text-zinc-100";

/** Top bar: session tabs (double-click/F2 to rename, × to archive), sessions menu, global controls. */
export function TabBar({ onAddAgent }: { onAddAgent: () => void }) {
  const tabs = useDeck((s) => s.tabs);
  const activeTabId = useDeck((s) => s.activeTabId);
  const [editing, setEditing] = useState<string | null>(null);

  return (
    <nav
      className="flex h-10 shrink-0 items-center gap-1 border-b border-zinc-800 bg-zinc-900 px-2"
      aria-label="Sessions"
    >
      <div role="tablist" className="flex min-w-0 items-center gap-1 overflow-x-auto">
        {tabs.map((tab) => (
          <div
            key={tab.id}
            className={`flex items-center rounded-md text-sm ${tab.id === activeTabId ? "bg-zinc-700" : "hover:bg-zinc-800"}`}
          >
            {editing === tab.id ? (
              <input
                // biome-ignore lint/a11y/noAutofocus: the user just asked to rename this tab
                autoFocus
                defaultValue={tab.name}
                aria-label="Nom de la session"
                className="w-28 rounded bg-zinc-950 px-2 py-1"
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
              <button
                type="button"
                role="tab"
                aria-selected={tab.id === activeTabId}
                title="Double-clic pour renommer"
                className="px-3 py-1"
                onClick={() => useDeck.setState({ activeTabId: tab.id })}
                onDoubleClick={() => setEditing(tab.id)}
                onKeyDown={(e) => {
                  if (e.key === "F2") setEditing(tab.id);
                }}
              >
                {tab.name}
              </button>
            )}
            <button
              type="button"
              aria-label={`Fermer et archiver la session ${tab.name}`}
              title="Fermer (la session reste dans le menu Sessions)"
              className="px-1.5 text-zinc-400 hover:text-zinc-100"
              onClick={() => closeTab(tab.id)}
            >
              ×
            </button>
          </div>
        ))}
      </div>
      <button
        type="button"
        aria-label="Nouvelle session"
        title="Nouvelle session"
        className={ctrl}
        onClick={() => deck.addTab()}
      >
        +
      </button>
      <SessionsMenu />

      <div className="ml-auto flex items-center gap-1">
        <button
          type="button"
          className={ctrl}
          onClick={interruptAll}
          title="Envoie Échap (Ctrl+C pour un shell) à chaque agent de la session : arrête la tâche en cours, l'agent reste ouvert. Ne fige pas le processus."
        >
          ⏸ Interrompre tout
        </button>
        <button type="button" className={ctrl} onClick={confirmStopAll} title="Tue tous les processus de la session">
          ⏹ Tout arrêter
        </button>
        <button type="button" className={ctrl} onClick={() => void restartAll()} title="Relance tous les agents">
          ↻ Tout relancer
        </button>
        <button
          type="button"
          onClick={onAddAgent}
          className="shrink-0 rounded-md bg-blue-600 px-3 py-1 text-sm font-medium hover:bg-blue-500"
        >
          + Ajouter un agent
        </button>
      </div>
    </nav>
  );
}
