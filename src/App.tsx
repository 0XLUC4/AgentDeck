import { useEffect, useState } from "react";
import { Deck } from "./app/Deck";
import { TabBar } from "./app/TabBar";
import { Toasts } from "./app/Toasts";
import { AgentPicker } from "./components/AgentPicker/AgentPicker";
import { onTick } from "./lib/clock";
import { useDeck } from "./store/deck";
import { restoreAndAutosave } from "./store/persist";
import { deriveStatuses, listenAgentStatus } from "./store/runtime";

// Module-level so StrictMode's double effects don't restore or subscribe twice.
const ready = restoreAndAutosave();
listenAgentStatus().catch((err) => console.warn("agent-status listener failed", err));

export default function App() {
  const [hydrated, setHydrated] = useState(false);
  const [picking, setPicking] = useState(false);
  const tabs = useDeck((s) => s.tabs);
  const activeTabId = useDeck((s) => s.activeTabId);
  const activeEmpty = useDeck((s) => s.tabs.find((t) => t.id === s.activeTabId)?.paneIds.length === 0);

  useEffect(() => {
    void ready.then(() => setHydrated(true));
    return onTick(() => deriveStatuses());
  }, []);

  return (
    <main className="flex h-full flex-col bg-zinc-950 text-zinc-100">
      <TabBar onAddAgent={() => setPicking(true)} />
      <div className="relative min-h-0 flex-1">
        {hydrated &&
          tabs.map((tab) => (
            // Inactive tabs stay mounted (invisible, same size) so terminals keep their state.
            <div key={tab.id} className={`absolute inset-0 ${tab.id === activeTabId ? "" : "invisible"}`}>
              <Deck tab={tab} />
            </div>
          ))}
        {hydrated && activeEmpty && (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-4 bg-zinc-950">
            <p className="text-zinc-400">Aucun agent dans cette session.</p>
            <button
              type="button"
              onClick={() => setPicking(true)}
              className="rounded-xl bg-blue-600 px-8 py-4 text-xl font-semibold hover:bg-blue-500"
            >
              + Ajouter un agent
            </button>
          </div>
        )}
      </div>
      {picking && <AgentPicker onClose={() => setPicking(false)} />}
      <Toasts />
    </main>
  );
}
