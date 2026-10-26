import { describe, expect, it } from "vitest";
import { gridLayout, gridRows } from "./gridLayout";

describe("gridRows", () => {
  it.each([
    [1, [1]],
    [2, [2]],
    [3, [2, 1]],
    [4, [2, 2]],
    [5, [3, 2]],
    [6, [3, 3]],
    [7, [3, 3, 1]],
    [8, [3, 3, 2]],
    [9, [3, 3, 3]],
    [10, [4, 4, 2]],
  ])("%i panes -> %j", (count, rows) => {
    expect(gridRows(count)).toEqual(rows);
  });

  it("is empty for no pane", () => expect(gridRows(0)).toEqual([]));
});

describe("gridLayout", () => {
  it.each([1, 2, 3, 5, 7, 10])("places each of %i panes exactly once, sizes sum", (count) => {
    const ids = Array.from({ length: count }, (_, i) => `p${i}`);
    const panels = Object.fromEntries(
      ids.map((id) => [id, { id, contentComponent: "pane", params: { paneId: id }, title: "Agent" }]),
    );
    const layout = gridLayout(ids, panels, 1001, 703);

    expect(layout.grid.orientation).toBe("VERTICAL");
    expect(layout.panels).toEqual(panels);
    const rows = layout.grid.root.data as { size: number; data: { size: number; data: { views: string[] } }[] }[];
    expect(rows.map((r) => r.data.length)).toEqual(gridRows(count));
    expect(rows.reduce((sum, r) => sum + r.size, 0)).toBe(703);
    for (const row of rows) expect(row.data.reduce((sum, leaf) => sum + leaf.size, 0)).toBe(1001);
    expect(rows.flatMap((r) => r.data.flatMap((leaf) => leaf.data.views))).toEqual(ids);
  });
});
