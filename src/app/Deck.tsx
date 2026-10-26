import {
  type DockviewApi,
  DockviewDefaultTab,
  DockviewReact,
  type IDockviewPanelHeaderProps,
  themeDark,
} from "dockview-react";
import "dockview-react/dist/styles/dockview.css";
import { useEffect, useState } from "react";
import { confirmClose } from "../components/TerminalPane/PaneHeader";
import { PanePanel } from "../components/TerminalPane/PanePanel";
import { deck, type Tab, useDeck } from "../store/deck";

const components = { pane: PanePanel };

// Dockview's tab close goes through the store so the process is killed too.
function PaneTab(props: IDockviewPanelHeaderProps) {
  return (
    <DockviewDefaultTab
      {...props}
      closeActionOverride={() => {
        const pane = useDeck.getState().panes[props.api.id];
        if (pane) confirmClose(pane);
      }}
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
      theme={themeDark}
      components={components}
      defaultTabComponent={PaneTab}
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
