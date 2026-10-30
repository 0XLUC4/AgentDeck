import {
  type DockviewApi,
  DockviewReact,
  type DockviewTheme,
  type IDockviewPanelHeaderProps,
  themeDark,
} from "dockview-react";
import "dockview-react/dist/styles/dockview.css";
import { useEffect, useState } from "react";
import { BrowserHeader, BrowserPanel, browserPanelId, paneOfBrowserPanel } from "../components/Browser/BrowserPanel";
import { SubagentHeader, SubagentPanel } from "../components/Subagents/SubagentPanel";
import { parseSubagentPanel, subagentPanelId, useSubagents } from "../components/Subagents/subagents";
import { EmptyPaneHeader, PaneHeader } from "../components/TerminalPane/PaneHeader";
import { PanePanel } from "../components/TerminalPane/PanePanel";
import { gridLayout } from "../lib/gridLayout";
import { deck, type Tab, useDeck } from "../store/deck";
import { useSettings } from "../store/settings";

const components = { pane: PanePanel, browser: BrowserPanel, subagent: SubagentPanel };

/** Dock panels of a session: each pane, followed by its browser panel and its subagent viewers. */
function dockPanels(paneIds: string[]): string[] {
  const { panes } = useDeck.getState();
  const subagents = useSettings.getState().showSubagents ? useSubagents.getState() : {};
  return paneIds.flatMap((id) => [
    id,
    ...(panes[id]?.browser ? [browserPanelId(id)] : []),
    ...(subagents[id] ?? []).map((a) => subagentPanelId(id, a.id)),
  ]);
}

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

/** Maximizes a pane in its session, or restores it if already maximized. */
export function toggleMaximizePane(tabId: string, paneId: string): void {
  const panel = apis.get(tabId)?.getPanel(paneId);
  if (!panel) return;
  if (panel.api.isMaximized()) panel.api.exitMaximized();
  else panel.api.maximize();
}

/** Re-lays the session's panes on a grid (store order), keeping the panels mounted, then saves the layout. */
export function arrangePanesInGrid(tabId: string): void {
  const api = apis.get(tabId);
  const tab = useDeck.getState().tabs.find((t) => t.id === tabId);
  if (!api || !tab) return;
  if (api.hasMaximizedGroup()) api.exitMaximizedGroup();
  const { panels } = api.toJSON();
  const ids = dockPanels(tab.paneIds).filter((id) => panels[id]);
  if (ids.length === 0) return;
  const activeId = api.activePanel?.id;
  // A hidden container can report 0: sizes are only proportions, so any non-zero size works.
  api.fromJSON(gridLayout(ids, panels, api.width || 1200, api.height || 800), { reuseExistingPanels: true });
  if (activeId) api.getPanel(activeId)?.api.setActive();
  deck.setTabLayout(tabId, api.toJSON());
}

const theme: DockviewTheme = {
  ...themeDark,
  name: "agentdeck",
  className: `${themeDark.className} dockview-theme-agentdeck`,
  gap: 6,
};

/** Our pane header doubles as the dockview tab, so there is one row of chrome per pane and it stays draggable. */
function PaneTab(props: IDockviewPanelHeaderProps) {
  const browserOf = paneOfBrowserPanel(props.api.id);
  const subagent = parseSubagentPanel(props.api.id);
  if (browserOf) return <BrowserHeader paneId={browserOf} />;
  return subagent ? <SubagentHeader {...subagent} /> : <AgentTab {...props} />;
}

function AgentTab({ api, containerApi }: IDockviewPanelHeaderProps) {
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
  // Joined to a string so the sync below only reruns when the set of browsers changes.
  const browsers = useDeck((s) => tab.paneIds.filter((id) => s.panes[id]?.browser).join(","));
  const showSubagents = useSettings((s) => s.showSubagents);
  const subagents = useSubagents((s) => tab.paneIds.flatMap((id) => (s[id] ?? []).map((a) => a.id)).join(","));

  // Registered before the pane sync below, which may re-grid through `apis`.
  useEffect(() => {
    if (!api) return;
    apis.set(tab.id, api);
    return () => {
      apis.delete(tab.id);
    };
  }, [api, tab.id]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: browsers/subagents/showSubagents only trigger the resync; dockPanels reads the stores
  useEffect(() => {
    if (!api) return;
    const { panes } = useDeck.getState();
    const wanted = dockPanels(tab.paneIds);
    let changed = false;
    for (const id of wanted) {
      if (!api.getPanel(id)) {
        changed = true;
        const browserOf = paneOfBrowserPanel(id);
        const subagent = parseSubagentPanel(id);
        const paneId = browserOf ?? subagent?.paneId;
        // New panes go side by side (not as tabs of the same group); a browser or subagent goes right of its agent.
        const last = api.panels[api.panels.length - 1];
        const reference = paneId && api.getPanel(paneId) ? paneId : last?.id;
        api.addPanel({
          id,
          component: browserOf ? "browser" : subagent ? "subagent" : "pane",
          params: subagent ?? { paneId: paneId ?? id },
          title: browserOf ? "Browser" : subagent ? "Sub-agent" : panes[id]?.title || "Agent",
          position: reference ? { referencePanel: reference, direction: "right" } : undefined,
        });
      }
    }
    for (const panel of api.panels) {
      if (!wanted.includes(panel.id)) {
        changed = true;
        panel.api.close();
      }
    }
    // Auto grid also applies to restored layouts, so a session always opens as a grid.
    if (useSettings.getState().autoGrid && (changed || tab.paneIds.length > 1)) arrangePanesInGrid(tab.id);
  }, [api, tab.paneIds, tab.id, browsers, subagents, showSubagents]);

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
      // Pointer drag instead of native HTML5 drag: in the webview, HTML5 drag on the header grabbed text
      // and swallowed clicks on the header buttons.
      dndStrategy="pointer"
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
