import type { IDockviewPanelProps } from "dockview-react";
import { useEffect, useState } from "react";
import { AGENTS } from "../../lib/agents";
import { useDeck } from "../../store/deck";
import { focusPane, restartPane } from "../../store/runtime";
import { PaneHeader } from "./PaneHeader";
import { TerminalPane } from "./TerminalPane";

export interface PanePanelParams {
  paneId: string;
}

/** Dockview panel: info header + terminal + exit bar. */
export function PanePanel({ api, containerApi, params }: IDockviewPanelProps<PanePanelParams>) {
  const pane = useDeck((s) => s.panes[params.paneId]);
  const isActive = useDeck((s) => s.activePaneId === params.paneId);
  const [maximized, setMaximized] = useState(false);
  const title = pane?.title ?? "";

  useEffect(() => api.setTitle(title || "Agent"), [api, title]);
  useEffect(() => {
    const sub = containerApi.onDidMaximizedGroupChange(() => setMaximized(api.isMaximized()));
    return () => sub.dispose();
  }, [api, containerApi]);

  if (!pane) return null;
  const color = AGENTS[pane.agent].color;
  return (
    <section
      className={`flex h-full flex-col border-2 ${pane.status === "waiting" ? "pane-waiting" : ""}`}
      style={{ borderColor: isActive ? color : `color-mix(in srgb, ${color} 30%, transparent)` }}
      onFocusCapture={() => focusPane(pane.id)}
      onMouseDown={() => focusPane(pane.id)}
      aria-label={`${AGENTS[pane.agent].name} — ${pane.title}`}
    >
      <PaneHeader
        pane={pane}
        maximized={maximized}
        onToggleMaximize={() => (api.isMaximized() ? api.exitMaximized() : api.maximize())}
      />
      <TerminalPane key={pane.generation} paneId={pane.id} fontSize={pane.fontSize} />
      {pane.exited && (
        <div role="status" className="flex items-center gap-3 bg-zinc-900 px-3 py-1.5 text-sm text-zinc-300">
          [processus terminé — appuyez sur R pour relancer]
          <button
            type="button"
            onClick={() => void restartPane(pane.id)}
            className="rounded bg-zinc-700 px-2 py-0.5 hover:bg-zinc-600"
          >
            Relancer
          </button>
        </div>
      )}
    </section>
  );
}
