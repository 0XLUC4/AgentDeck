import { describe, expect, it, vi } from "vitest";

vi.mock("../lib/ipc", () => ({ writeToPane: vi.fn(async () => undefined), gitPaneDir: vi.fn(async () => "/repo") }));

const { createsLoop, reviewPrompt } = await import("./review");

describe("review pipeline", () => {
  it("refuses loops", () => {
    expect(createsLoop({}, "a", "b")).toBe(false);
    expect(createsLoop({ b: "a" }, "a", "b")).toBe(true);
    expect(createsLoop({ b: "c", c: "a" }, "a", "b")).toBe(true);
    expect(createsLoop({ b: "c" }, "a", "b")).toBe(false);
    expect(createsLoop({}, "a", "a")).toBe(true);
  });

  it("points the reviewer at the author's folder", () => {
    const text = reviewPrompt({ title: "Api", agent: "codex" }, "D:\repo", "add\nlogin");
    expect(text).toContain('git -C "D:\repo" diff HEAD');
    expect(text).toContain('"add login"');
    expect(text).not.toContain("\n");
  });
});
