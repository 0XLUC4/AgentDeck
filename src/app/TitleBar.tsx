import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ChevronRight,
  Copy,
  Megaphone,
  Minus,
  PanelLeft,
  Pause,
  RotateCcw,
  Settings,
  Square,
  SquareStop,
  X,
} from "lucide-react";
import { useEffect, useState } from "react";
import { openBroadcast } from "../components/Broadcast/Broadcast";
import { HistoryButton } from "../components/History/HistoryDialog";
import { IconButton } from "../components/IconButton/IconButton";
import { JournalButton } from "../components/Journal/JournalDialog";
import { UsageButton } from "../components/Usage/UsageDialog";
import { WallToggle } from "../components/Wall/Wall";
import { activeTab, activeWorkspace, useDeck } from "../store/deck";
import { interruptAll, restartAll, stopAll } from "../store/runtime";
import { useSettings } from "../store/settings";
import { ActivityToggle } from "./Activity";

const appWindow = getCurrentWindow();
const warn = (what: string) => (err: unknown) => console.warn(`${what} failed`, err);

/** Runs a bulk action on the active session, asking first unless disabled in Réglages. */
function confirmBulk(question: (count: number) => string, action: () => Promise<unknown>) {
  const count = activeTab()?.paneIds.length ?? 0;
  if (count > 0 && (!useSettings.getState().confirmBulk || window.confirm(question(count)))) void action();
}

export const confirmStopAll = () => confirmBulk((n) => `Arrêter les ${n} agent(s) de cette session ?`, stopAll);
export const confirmRestartAll = () => confirmBulk((n) => `Relancer les ${n} agent(s) de cette session ?`, restartAll);

/** Logo: three stacked cards, the front one in the accent color. */
function AppMark() {
  return (
    <svg viewBox="0 0 20 20" className="pointer-events-none size-[18px] shrink-0" aria-hidden="true">
      <rect x="5" y="2" width="12" height="9" rx="2" fill="var(--color-line-strong)" />
      <rect x="3.5" y="5" width="13" height="10" rx="2.2" fill="var(--color-subtle)" />
      <rect x="2" y="8.5" width="14" height="9.5" rx="2.4" fill="var(--color-accent)" />
      <path
        d="M5 12.2l2 1.6-2 1.6"
        stroke="var(--color-on-accent)"
        strokeWidth="1.4"
        fill="none"
        strokeLinecap="round"
      />
    </svg>
  );
}

function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const sync = () => {
      appWindow.isMaximized().then(setMaximized, warn("isMaximized"));
    };
    sync();
    const unlisten = appWindow.onResized(sync);
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  const base =
    "inline-flex h-full w-[46px] items-center justify-center text-muted transition-colors duration-150 focus-visible:-outline-offset-2";
  return (
    <div className="flex h-full shrink-0">
      <button
        type="button"
        aria-label="Réduire"
        title="Réduire"
        className={`${base} hover:bg-hover hover:text-fg`}
        onClick={() => appWindow.minimize().catch(warn("minimize"))}
      >
        <Minus size={16} strokeWidth={1.5} aria-hidden="true" />
      </button>
      <button
        type="button"
        aria-label={maximized ? "Restaurer la fenêtre" : "Agrandir la fenêtre"}
        title={maximized ? "Restaurer" : "Agrandir"}
        className={`${base} hover:bg-hover hover:text-fg`}
        onClick={() => appWindow.toggleMaximize().catch(warn("toggleMaximize"))}
      >
        {maximized ? (
          <Copy size={13} strokeWidth={1.5} className="-scale-x-100" aria-hidden="true" />
        ) : (
          <Square size={13} strokeWidth={1.5} aria-hidden="true" />
        )}
      </button>
      <button
        type="button"
        aria-label="Fermer AgentDeck"
        title="Fermer"
        className={`${base} hover:bg-close hover:text-fg`}
        onClick={() => appWindow.close().catch(warn("close"))}
      >
        <X size={16} strokeWidth={1.5} aria-hidden="true" />
      </button>
    </div>
  );
}

/** Custom window title bar: identity, where you are, global session controls, window buttons. */
export function TitleBar({
  sidebarOpen,
  onToggleSidebar,
  activityOpen,
  onToggleActivity,
  onOpenSettings,
}: {
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  activityOpen: boolean;
  onToggleActivity: () => void;
  onOpenSettings: () => void;
}) {
  const workspaceName = useDeck((s) => activeWorkspace(s).name);
  const sessionName = useDeck((s) => s.tabs.find((t) => t.id === s.activeTabId)?.name ?? "");

  return (
    <header
      data-tauri-drag-region
      className="flex h-9 shrink-0 select-none items-center border-b border-line bg-panel pl-2"
    >
      <IconButton
        icon={PanelLeft}
        label={sidebarOpen ? "Masquer les workspaces" : "Afficher les workspaces"}
        aria-pressed={sidebarOpen}
        onClick={onToggleSidebar}
      />
      <div data-tauri-drag-region className="ml-2 flex min-w-0 items-center gap-2">
        <AppMark />
        <span data-tauri-drag-region className="font-semibold tracking-tight text-fg">
          AgentDeck
        </span>
        <nav
          data-tauri-drag-region
          aria-label="Emplacement"
          className="flex min-w-0 items-center gap-1 pl-2 text-muted"
        >
          <ChevronRight size={14} className="pointer-events-none shrink-0 text-subtle" aria-hidden="true" />
          <span data-tauri-drag-region className="truncate">
            {workspaceName}
          </span>
          <ChevronRight size={14} className="pointer-events-none shrink-0 text-subtle" aria-hidden="true" />
          <span data-tauri-drag-region className="truncate text-fg" aria-current="page">
            {sessionName}
          </span>
        </nav>
      </div>

      <div data-tauri-drag-region className="h-full min-w-6 flex-1" />

      <div className="flex items-center gap-0.5 pr-2" role="toolbar" aria-label="Contrôles de la session">
        <IconButton
          icon={Megaphone}
          label="Diffuser un prompt"
          hint="Diffuser un prompt à plusieurs agents (Ctrl+Maj+B)"
          onClick={openBroadcast}
        />
        <IconButton
          icon={Pause}
          label="Interrompre tout"
          hint="Interrompre tout : envoie Échap (Ctrl+C pour un shell) à chaque agent de la session. Les agents restent ouverts."
          onClick={interruptAll}
        />
        <IconButton
          icon={RotateCcw}
          label="Tout relancer"
          hint="Relancer tous les agents de la session"
          onClick={confirmRestartAll}
        />
        <IconButton
          icon={SquareStop}
          tone="danger"
          label="Tout arrêter"
          hint="Arrêter tous les processus de la session"
          onClick={confirmStopAll}
        />
      </div>
      <span className="h-5 w-px bg-line" aria-hidden="true" />
      <div className="flex items-center gap-0.5 px-2">
        <WallToggle />
        <UsageButton />
        <HistoryButton />
        <JournalButton />
        <ActivityToggle open={activityOpen} onToggle={onToggleActivity} />
        <IconButton icon={Settings} label="Réglages" hint="Réglages (Ctrl+,)" onClick={onOpenSettings} />
      </div>
      <span className="h-5 w-px bg-line" aria-hidden="true" />
      <WindowControls />
    </header>
  );
}
