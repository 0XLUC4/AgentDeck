// Auto grid: a session's panes in rows of equal height, cols = ceil(sqrt(n)); the last row's panes stretch.
import { type GroupviewPanelState, Orientation, type SerializedDockview } from "dockview-react";

/** Number of panes in each row, top to bottom. 3 -> [2, 1], 5 -> [3, 2], 6 -> [3, 3]. */
export function gridRows(count: number): number[] {
  if (count <= 0) return [];
  const cols = Math.ceil(Math.sqrt(count));
  const rows = Math.ceil(count / cols);
  return Array.from({ length: rows }, (_, i) => Math.min(cols, count - i * cols));
}

/** Splits `total` px into `parts` integer sizes that sum exactly to `total`. */
function split(total: number, parts: number): number[] {
  const base = Math.floor(total / parts);
  return Array.from({ length: parts }, (_, i) => (i === parts - 1 ? total - base * (parts - 1) : base));
}

/**
 * Dockview layout placing `paneIds` (in order, row by row) on a grid, one panel per group.
 * `panels` must hold the serialized state of every pane (kept as is: component, params, title).
 */
export function gridLayout(
  paneIds: string[],
  panels: Record<string, GroupviewPanelState>,
  width: number,
  height: number,
): SerializedDockview {
  const rowCounts = gridRows(paneIds.length);
  const rowHeights = split(height, rowCounts.length);
  let next = 0;
  // Gridview sizes: a VERTICAL root's children are sized by height, their HORIZONTAL children by width.
  const rows = rowCounts.map((count, r) => {
    const widths = split(width, count);
    return {
      type: "branch" as const,
      size: rowHeights[r],
      data: widths.map((size) => {
        const id = paneIds[next++];
        return { type: "leaf" as const, size, data: { id: `grid-${id}`, views: [id], activeView: id } };
      }),
    };
  });
  return {
    grid: { root: { type: "branch", size: width, data: rows }, width, height, orientation: Orientation.VERTICAL },
    panels: Object.fromEntries(paneIds.map((id) => [id, panels[id]])),
  };
}
