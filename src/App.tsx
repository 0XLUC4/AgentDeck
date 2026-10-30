import { Plus } from "lucide-react";
import { useEffect, useState } from "react";
import { ActivityPanel, jumpToNextWaiting, PermissionBanner } from "./app/Activity";
import { Deck } from "./app/Deck";
import { Sidebar } from "./app/Sidebar";
import { TabBar } from "./app/TabBar";
import { TitleBar } from "./app/TitleBar";
import { Toasts } from "./app/Toasts";
import { AgentLogo } from "./components/AgentLogo/AgentLogo";
import { AgentPicker } from "./components/AgentPicker/AgentPicker";
import { BroadcastHost } from "./components/Broadcast/Broadcast";
import { CommandPalette } from "./components/CommandPalette/CommandPalette";
import { startDetachHost } from "./components/Detached/host";
import { startJournalRecorder } from "./components/Journal/recorder";
import { LinkWorkspaceDialog } from "./components/LinkWorkspace/LinkWorkspaceDialog";
import { listenPermissions } from "./components/Permission/permissions";
import { SettingsDialog } from "./components/Settings/SettingsDialog";
import { TemplatePickerHost } from "./components/Templates/TemplatePicker";
import { WallOverlay } from "./components/Wall/Wall";
import { startBudgetWatch } from "./lib/budget";
import { onTick } from "./lib/clock";
import { listenOrchestrator } from "./lib/orchestrator";
import { useTraySync } from "./lib/tray";
import { startTaskSpeech } from "./lib/tts";
import { checkForUpdateSilently } from "./lib/updater";
import { listenVoiceTranscripts } from "./lib/voiceListener";
import { activeWorkspace, useDeck } from "./store/deck";
import { restoreAndAutosave } from "./store/persist";
import { startQueueRunner } from "./store/queue";
import { startReviewRunner } from "./store/review";
import { deriveStatuses, listenAgentStatus } from "./store/runtime";
import { settingsReady, useSettings } from "./store/settings";

// Module-level so StrictMode's double effects don't restore or subscribe twice.
const ready = restoreAndAutosave();
listenAgentStatus().catch((err) => console.warn("agent-status listener failed", err));
startJournalRecorder();
listenVoiceTranscripts().catch((err) => console.warn("voice-transcript listener failed", err));
startQueueRunner();
startReviewRunner();
startBudgetWatch();
startTaskSpeech();
listenPermissions().catch((err) => console.warn("agent-permission listener failed", err));
listenOrchestrator().catch((err) => console.warn("orchestrator listener failed", err));
checkForUpdateSilently();
startDetachHost();

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
          <h2 className="text-[15px] font-medium text-fg">Empty session</h2>
          <p className="text-muted">
            Start Claude Code, Codex, Gemini CLI, Grok Build or a shell
            {workspace.path ? (
              <>
                {" "}
                in <span className="font-mono text-xs text-fg">{workspace.path}</span>
              </>
            ) : null}
            . Each agent opens in its own terminal; drag the headers to rearrange.
          </p>
        </div>
        <button
          type="button"
          onClick={onAddAgent}
          className="inline-flex h-9 items-center gap-2 rounded-md bg-accent px-4 font-medium text-on-accent transition-colors duration-150 hover:bg-accent-hover"
        >
          <Plus size={16} strokeWidth={1.5} aria-hidden="true" />
          Add agent
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
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [activityOpen, setActivityOpen] = useState(false);
  const tabs = useDeck((s) => s.tabs);
  const activeTabId = useDeck((s) => s.activeTabId);
  const activeEmpty = useDeck((s) => s.tabs.find((t) => t.id === s.activeTabId)?.paneIds.length === 0);
  useTraySync();

  useEffect(() => {
    // Settings first: terminals read the font at mount; panels start as configured.
    void Promise.all([ready, settingsReady]).then(() => {
      const { sidebarOpen, activityOpen } = useSettings.getState();
      setSidebarOpen(sidebarOpen);
      setActivityOpen(activityOpen);
      setHydrated(true);
    });
    return onTick(() => deriveStatuses());
  }, []);

  // Ctrl+Shift+A toggles the activity panel, Ctrl+, opens Settings; capture phase so the terminal never receives them.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const activity = e.ctrlKey && e.shiftKey && !e.altKey && e.code === "KeyA";
      const settings = e.ctrlKey && !e.shiftKey && !e.altKey && e.key === ",";
      const nextWaiting = e.ctrlKey && e.shiftKey && !e.altKey && e.code === "KeyJ";
      // The voice shortcut recorder needs every combo.
      const recording = e.target instanceof HTMLElement && e.target.closest("[data-shortcut-recorder]");
      if ((!activity && !settings && !nextWaiting) || recording) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.repeat) return;
      if (activity) setActivityOpen((open) => !open);
      else if (nextWaiting) jumpToNextWaiting();
      else setSettingsOpen(true);
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
        onOpenSettings={() => setSettingsOpen(true)}
      />
      <div className="flex min-h-0 flex-1">
        {sidebarOpen && <Sidebar onLink={() => setLinking(true)} />}
        <main className="flex min-w-0 flex-1 flex-col">
          <TabBar onAddAgent={() => setPicking(true)} />
          <div className="relative isolate min-h-0 flex-1">
            {hydrated &&
              tabs.map((tab) => (
                // Every session of every workspace stays mounted (invisible, same size): terminals keep their
                // state and answer ConPTY's startup cursor query even in the background.
                // Not `visibility: hidden`: dockview sets `visibility: visible` on sashes and headers, which leaked
                // them into other sessions. Opacity and inert apply to the whole subtree.
                <div
                  key={tab.id}
                  className={`absolute inset-0 p-1.5 ${tab.id === activeTabId ? "" : "-z-10 opacity-0"}`}
                  aria-hidden={tab.id !== activeTabId}
                  inert={tab.id !== activeTabId}
                >
                  <Deck tab={tab} />
                </div>
              ))}
            {hydrated && activeEmpty && <EmptySession onAddAgent={() => setPicking(true)} />}
            {hydrated && <WallOverlay />}
          </div>
        </main>
        {activityOpen && <ActivityPanel onClose={() => setActivityOpen(false)} />}
      </div>
      {picking && <AgentPicker onClose={() => setPicking(false)} />}
      {linking && <LinkWorkspaceDialog onClose={() => setLinking(false)} />}
      {settingsOpen && <SettingsDialog onClose={() => setSettingsOpen(false)} />}
      <CommandPalette
        onAddAgent={() => setPicking(true)}
        onLinkWorkspace={() => setLinking(true)}
        onOpenSettings={() => setSettingsOpen(true)}
        onToggleSidebar={() => setSidebarOpen((open) => !open)}
        onToggleActivity={() => setActivityOpen((open) => !open)}
      />
      {!activityOpen && <PermissionBanner />}
      <BroadcastHost />
      <TemplatePickerHost />
      <Toasts />
    </div>
  );
}
