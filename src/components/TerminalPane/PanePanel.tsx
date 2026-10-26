import type { IDockviewPanelProps } from "dockview-react";
import { RotateCcw } from "lucide-react";
import { useEffect } from "react";
import { AGENTS } from "../../lib/agents";
import { useDeck } from "../../store/deck";
import { focusPane, restartPane } from "../../store/runtime";
import { TerminalPane } from "./TerminalPane";

export interface PanePanelParams {
  paneId: string;
}

/** Dockview panel body: terminal + exit bar. The header is the dockview tab (PaneTab in Deck.tsx). */
export function PanePanel({ api, params }: IDockviewPanelProps<PanePanelParams>) {
  const pane = useDeck((s) => s.panes[params.paneId]);
  const title = pane?.title ?? "";

  useEffect(() => api.setTitle(title || "Agent"), [api, title]);

  if (!pane) return null;
  return (
    <section
      className={`flex h-full flex-col bg-base`}
      onFocusCapture={() => focusPane(pane.id)}
      onMouseDown={() => focusPane(pane.id)}
      aria-label={`${AGENTS[pane.agent].name} : ${pane.title}`}
    >
      <TerminalPane key={pane.generation} paneId={pane.id} fontSize={pane.fontSize} />
      {pane.exited && (
        <div
          role="status"
          className="flex shrink-0 items-center gap-3 border-t border-line bg-panel px-3 py-1.5 text-xs text-muted"
        >
          <span className="size-1.5 rounded-full bg-subtle" aria-hidden="true" />
          Processus terminé. Appuyez sur R dans le terminal pour relancer.
          <button
            type="button"
            onClick={() => void restartPane(pane.id)}
            className="ml-auto inline-flex items-center gap-1.5 rounded-md border border-line-strong px-2 py-0.5 font-medium text-fg transition-colors hover:bg-hover"
          >
            <RotateCcw size={13} strokeWidth={1.5} aria-hidden="true" />
            Relancer
          </button>
        </div>
      )}
    </section>
  );
}
