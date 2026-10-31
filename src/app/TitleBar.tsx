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
import appIcon from "../../src-tauri/icons/64x64.png";
import { openBroadcast } from "../components/Broadcast/Broadcast";
import { HistoryButton } from "../components/History/HistoryDialog";
import { IconButton } from "../components/IconButton/IconButton";
import { JournalButton } from "../components/Journal/JournalDialog";
import { UpdateButton } from "../components/Update/UpdateButton";
import { UsageButton } from "../components/Usage/UsageDialog";
import { WallToggle } from "../components/Wall/Wall";
import { activeTab, activeWorkspace, useDeck } from "../store/deck";
import { interruptAll, restartAll, stopAll } from "../store/runtime";
import { useSettings } from "../store/settings";
import { ActivityToggle } from "./Activity";

const appWindow = getCurrentWindow();
const warn = (what: string) => (err: unknown) => console.warn(`${what} failed`, err);

/** Runs a bulk action on the active session, asking first unless disabled in Settings. */
function confirmBulk(question: (count: number) => string, action: () => Promise<unknown>) {
  const count = activeTab()?.paneIds.length ?? 0;
  if (count > 0 && (!useSettings.getState().confirmBulk || window.confirm(question(count)))) void action();
}

export const confirmStopAll = () => confirmBulk((n) => `Stop the ${n} agent(s) in this session?`, stopAll);
export const confirmRestartAll = () => confirmBulk((n) => `Restart the ${n} agent(s) in this session?`, restartAll);

/** Logo: the app icon itself, so it never follows the theme or accent. */
function AppMark() {
  return <img src={appIcon} alt="" className="pointer-events-none size-[18px] shrink-0" draggable={false} />;
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
        aria-label="Minimize"
        title="Minimize"
        className={`${base} hover:bg-hover hover:text-fg`}
        onClick={() => appWindow.minimize().catch(warn("minimize"))}
      >
        <Minus size={16} strokeWidth={1.5} aria-hidden="true" />
      </button>
      <button
        type="button"
        aria-label={maximized ? "Restore window" : "Maximize window"}
        title={maximized ? "Restore" : "Maximize"}
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
        aria-label="Close AgentDeck"
        title="Close"
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
        label={sidebarOpen ? "Hide workspaces" : "Show workspaces"}
        aria-pressed={sidebarOpen}
        onClick={onToggleSidebar}
      />
      <div data-tauri-drag-region className="ml-2 flex min-w-0 items-center gap-2">
        <AppMark />
        <span data-tauri-drag-region className="font-semibold tracking-tight text-fg">
          AgentDeck
        </span>
        <nav data-tauri-drag-region aria-label="Location" className="flex min-w-0 items-center gap-1 pl-2 text-muted">
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

      <div className="flex items-center gap-0.5 pr-2" role="toolbar" aria-label="Session controls">
        <IconButton
          icon={Megaphone}
          label="Broadcast a prompt"
          hint="Broadcast a prompt to several agents (Ctrl+Shift+B)"
          onClick={openBroadcast}
        />
        <IconButton
          icon={Pause}
          label="Interrupt all"
          hint="Interrupt all: sends Esc (Ctrl+C for a shell) to every agent in the session. Agents stay open."
          onClick={interruptAll}
        />
        <IconButton
          icon={RotateCcw}
          label="Restart all"
          hint="Restart every agent in the session"
          onClick={confirmRestartAll}
        />
        <IconButton
          icon={SquareStop}
          tone="danger"
          label="Stop all"
          hint="Stop every process in the session"
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
        <IconButton icon={Settings} label="Settings" hint="Settings (Ctrl+,)" onClick={onOpenSettings} />
        <UpdateButton />
      </div>
      <span className="h-5 w-px bg-line" aria-hidden="true" />
      <WindowControls />
    </header>
  );
}
