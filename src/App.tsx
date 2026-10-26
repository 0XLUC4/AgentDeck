import { Plus } from "lucide-react";
import { useEffect, useState } from "react";
import { ActivityPanel } from "./app/Activity";
import { Deck } from "./app/Deck";
import { Sidebar } from "./app/Sidebar";
import { TabBar } from "./app/TabBar";
import { TitleBar } from "./app/TitleBar";
import { Toasts } from "./app/Toasts";
import { AgentLogo } from "./components/AgentLogo/AgentLogo";
import { AgentPicker } from "./components/AgentPicker/AgentPicker";
import { LinkWorkspaceDialog } from "./components/LinkWorkspace/LinkWorkspaceDialog";
import { onTick } from "./lib/clock";
import { activeWorkspace, useDeck } from "./store/deck";
import { restoreAndAutosave } from "./store/persist";
import { deriveStatuses, listenAgentStatus } from "./store/runtime";

// Module-level so StrictMode's double effects don't restore or subscribe twice.
const ready = restoreAndAutosave();
listenAgentStatus().catch((err) => console.warn("agent-status listener failed", err));

const ACTIVITY_KEY = "agentdeck.activityOpen";

// Storage can be unavailable (private mode, blocked): the panel then just starts closed.
function readActivityOpen(): boolean {
  try {
    return localStorage.getItem(ACTIVITY_KEY) === "1";
  } catch {
    return false;
  }
}

function saveActivityOpen(open: boolean): void {
  try {
    localStorage.setItem(ACTIVITY_KEY, open ? "1" : "0");
  } catch {
    // not persisted; harmless
  }
}

function EmptySession({ onAddAgent }: { onAddAgent: () => void }) {
  const workspace = useDeck((s) => activeWorkspace(s));
  return (
    // z-10: dockview's own positioned layers would otherwise sit above and swallow clicks.
    <div className="absolute inset-0 z-10 flex items-center justify-center bg-gutter p-6">
      <div className="flex max-w-sm flex-col items-center gap-5 text-center">
        <div className="flex gap-3 opacity-80" aria-hidden="true">
          {(["claude", "codex", "gemini", "grok", "shell"] as const).map((kind) => (
            <AgentLogo key={kind} kind={kind} size={20} />
          ))}
        </div>
        <div className="flex flex-col gap-1.5">
          <h2 className="text-[15px] font-medium text-fg">Session vide</h2>
          <p className="text-muted">
            Lancez Claude Code, Codex, Gemini CLI, Grok Build ou un shell
            {workspace.path ? (
              <>
                {" "}
                dans <span className="font-mono text-xs text-fg">{workspace.path}</span>
              </>
            ) : null}
            . Chaque agent s'ouvre dans son propre terminal ; glissez les en-têtes pour réorganiser.
          </p>
        </div>
        <button
          type="button"
          onClick={onAddAgent}
          className="inline-flex h-9 items-center gap-2 rounded-md bg-accent px-4 font-medium text-on-accent transition-colors duration-150 hover:bg-accent-hover"
        >
          <Plus size={16} strokeWidth={1.5} aria-hidden="true" />
          Ajouter un agent
        </button>
      </div>
    </div>
  );
}

export default function App() {
  const [hydrated, setHydrated] = useState(false);
  const [picking, setPicking] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [linking, setLinking] = useState(false);
  const [activityOpen, setActivityOpen] = useState(readActivityOpen);
  const tabs = useDeck((s) => s.tabs);
  const activeTabId = useDeck((s) => s.activeTabId);
  const activeEmpty = useDeck((s) => s.tabs.find((t) => t.id === s.activeTabId)?.paneIds.length === 0);

  useEffect(() => {
    void ready.then(() => setHydrated(true));
    return onTick(() => deriveStatuses());
  }, []);

  useEffect(() => saveActivityOpen(activityOpen), [activityOpen]);

  // Ctrl+Shift+A toggles the activity panel; capture phase so the terminal never receives it.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.shiftKey && !e.altKey && e.code === "KeyA") {
        e.preventDefault();
        e.stopPropagation();
        if (e.type === "keydown" && !e.repeat) setActivityOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  return (
    <div className="flex h-full flex-col bg-gutter text-fg">
      <TitleBar
        sidebarOpen={sidebarOpen}
        onToggleSidebar={() => setSidebarOpen(!sidebarOpen)}
        activityOpen={activityOpen}
        onToggleActivity={() => setActivityOpen(!activityOpen)}
      />
      <div className="flex min-h-0 flex-1">
        {sidebarOpen && <Sidebar onLink={() => setLinking(true)} />}
        <main className="flex min-w-0 flex-1 flex-col">
          <TabBar onAddAgent={() => setPicking(true)} />
          <div className="relative min-h-0 flex-1">
            {hydrated &&
              tabs.map((tab) => (
                // Every session of every workspace stays mounted (invisible, same size): terminals keep their
                // state and answer ConPTY's startup cursor query even in the background.
                <div
                  key={tab.id}
                  className={`absolute inset-0 p-1.5 ${tab.id === activeTabId ? "" : "invisible"}`}
                  aria-hidden={tab.id !== activeTabId}
                >
                  <Deck tab={tab} />
                </div>
              ))}
            {hydrated && activeEmpty && <EmptySession onAddAgent={() => setPicking(true)} />}
          </div>
        </main>
        {activityOpen && <ActivityPanel onClose={() => setActivityOpen(false)} />}
      </div>
      {picking && <AgentPicker onClose={() => setPicking(false)} />}
      {linking && <LinkWorkspaceDialog onClose={() => setLinking(false)} />}
      <Toasts />
    </div>
  );
}
