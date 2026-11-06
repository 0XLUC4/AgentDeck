import { describe, expect, it } from "vitest";
import { onViewport } from "./wakeTerminal";

const box = (left: number, top: number, width: number, height: number) => ({
  left,
  top,
  width,
  height,
  right: left + width,
  bottom: top + height,
});

describe("onViewport", () => {
  it("accepts a pane inside the window", () => expect(onViewport(box(10, 10, 300, 200), 1400, 900)).toBe(true));
  it("rejects a zero-size pane (minimized)", () => expect(onViewport(box(0, 0, 0, 0), 1400, 900)).toBe(false));
  it("rejects an off-screen tab", () => expect(onViewport(box(-5000, 10, 300, 200), 1400, 900)).toBe(false));
  it("rejects a pane past the right edge", () => expect(onViewport(box(1400, 10, 300, 200), 1400, 900)).toBe(false));
});
