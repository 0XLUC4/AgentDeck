import { useNow } from "../../lib/clock";
import type { Pane } from "../../store/deck";
import { isBusy } from "../../store/runtime";

/**
 * Soft white line around a pane while its agent is really doing something (working and printing), with a light
 * running all the way around it.
 * Drawn in two halves that meet: the header's (top) and the terminal's (rest). Dockview renders the
 * terminal in its own layer, so a border on the pane itself would end up hidden behind it.
 */
export function WorkingOutline({ pane, part }: { pane: Pane; part: "top" | "body" }) {
  // A plain shell has no task: its output is just commands the user runs.
  if (pane.agent === "shell" || pane.status !== "working" || pane.exited) return null;
  return <BusyLine paneId={pane.id} part={part} />;
}

// Split out so only working panes re-render on the clock tick.
function BusyLine({ paneId, part }: { paneId: string; part: "top" | "body" }) {
  const now = useNow();
  if (!isBusy(paneId, now)) return null;
  return (
    <div className={`pane-outline pane-outline-${part}`} aria-hidden="true">
      {/* Third light of the loop (left edge); the others are ::before/::after, see index.css. */}
      {part === "body" && <span className="pane-sweep-left" />}
    </div>
  );
}
