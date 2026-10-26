import {
  type DockviewApi,
  DockviewReact,
  type DockviewTheme,
  type IDockviewPanelHeaderProps,
  themeDark,
} from "dockview-react";
import "dockview-react/dist/styles/dockview.css";
import { useEffect, useState } from "react";
import { EmptyPaneHeader, PaneHeader } from "../components/TerminalPane/PaneHeader";
import { PanePanel } from "../components/TerminalPane/PanePanel";
import { deck, type Tab, useDeck } from "../store/deck";

const components = { pane: PanePanel };

/** Dockview api of each mounted session tab, so other parts of the UI can reveal a pane. */
const apis = new Map<string, DockviewApi>();

/** Makes a pane visible in its session: leaves another pane's maximized state, selects it if stacked. */
export function revealPane(tabId: string, paneId: string): void {
  const api = apis.get(tabId);
  const panel = api?.getPanel(paneId);
  if (!api || !panel) return;
  if (api.hasMaximizedGroup() && !panel.api.isMaximized()) api.exitMaximizedGroup();
  panel.api.setActive();
}

const theme: DockviewTheme = {
  ...themeDark,
  name: "agentdeck",
  className: `${themeDark.className} dockview-theme-agentdeck`,
  gap: 6,
};

/** Our pane header doubles as the dockview tab, so there is one row of chrome per pane and it stays draggable. */
function PaneTab({ api, containerApi }: IDockviewPanelHeaderProps) {
  const pane = useDeck((s) => s.panes[api.id]);
  const active = useDeck((s) => s.activePaneId === api.id);
  const [maximized, setMaximized] = useState(false);
  const [stacked, setStacked] = useState(false);

  useEffect(() => {
    const update = () => {
      setMaximized(api.isMaximized());
      setStacked(api.group.panels.length > 1);
    };
    update();
    const subs = [containerApi.onDidMaximizedGroupChange(update), containerApi.onDidLayoutChange(update)];
    return () => {
      for (const sub of subs) sub.dispose();
    };
  }, [api, containerApi]);

  if (!pane) return <EmptyPaneHeader />;
  return (
    <PaneHeader
      pane={pane}
      active={active}
      maximized={maximized}
      compact={stacked}
      onToggleMaximize={() => (api.isMaximized() ? api.exitMaximized() : api.maximize())}
    />
  );
}

/** One dockview grid per session tab. The store's paneIds are the source of truth. */
export function Deck({ tab }: { tab: Tab }) {
  const [api, setApi] = useState<DockviewApi | null>(null);

  useEffect(() => {
    if (!api) return;
    const { panes } = useDeck.getState();
    for (const id of tab.paneIds) {
      if (!api.getPanel(id)) {
        // New panes go side by side (not as tabs of the same group).
        const last = api.panels[api.panels.length - 1];
        api.addPanel({
          id,
          component: "pane",
          params: { paneId: id },
          title: panes[id]?.title || "Agent",
          position: last ? { referencePanel: last.id, direction: "right" } : undefined,
        });
      }
    }
    for (const panel of api.panels) {
      if (!tab.paneIds.includes(panel.id)) panel.api.close();
    }
  }, [api, tab.paneIds]);

  useEffect(() => {
    if (!api) return;
    apis.set(tab.id, api);
    return () => {
      apis.delete(tab.id);
    };
  }, [api, tab.id]);

  useEffect(() => {
    if (!api) return;
    const sub = api.onDidLayoutChange(() => deck.setTabLayout(tab.id, api.toJSON()));
    return () => sub.dispose();
  }, [api, tab.id]);

  // Esc restores a maximized pane, unless the key is meant for a terminal (agents use Esc).
  useEffect(() => {
    if (!api) return;
    const onKey = (e: KeyboardEvent) => {
      const inTerminal = e.target instanceof Element && e.target.closest(".xterm");
      if (e.key === "Escape" && !inTerminal && useDeck.getState().activeTabId === tab.id && api.hasMaximizedGroup()) {
        api.exitMaximizedGroup();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [api, tab.id]);

  return (
    <DockviewReact
      className="h-full"
      theme={theme}
      components={components}
      defaultTabComponent={PaneTab}
      singleTabMode="fullwidth"
      disableFloatingGroups
      defaultRenderer="always"
      onReady={(e) => {
        if (tab.layout) {
          try {
            e.api.fromJSON(tab.layout);
          } catch (err) {
            console.warn("layout restore failed; using default layout", err);
          }
        }
        setApi(e.api);
      }}
    />
  );
}
