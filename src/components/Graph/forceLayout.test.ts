import { describe, expect, it } from "vitest";
import { initialPositions, step } from "./forceLayout";

describe("forceLayout", () => {
  it("pulls linked notes together and pushes the others away", () => {
    const nodes = initialPositions(4);
    const edges: [number, number][] = [[0, 1]];
    for (let i = 0; i < 300; i++) step(nodes, edges, 1 - i / 300);
    const dist = (a: number, b: number) => Math.hypot(nodes[a].x - nodes[b].x, nodes[a].y - nodes[b].y);
    expect(dist(0, 1)).toBeLessThan(dist(2, 3));
    expect(nodes.every((n) => Number.isFinite(n.x) && Number.isFinite(n.y))).toBe(true);
  });
});
