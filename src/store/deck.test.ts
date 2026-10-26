import { beforeEach, describe, expect, it } from "vitest";
import { interruptKey } from "../lib/agents";
import { applyStatus, clampFontSize, deck, FONT_SIZE, newPane, type Pane, useDeck } from "./deck";

const pane = (over: Partial<Pane> = {}): Pane => ({
  ...newPane({
    id: "p1",
    agent: "claude",
    title: "t",
    cwd: "/",
    args: [],
    worktree: false,
    completedCount: 0,
    fontSize: 13,
  }),
  ...over,
});

describe("applyStatus", () => {
  it("starts the task timer on working and keeps it while waiting", () => {
    const working = applyStatus(pane(), "working", "Edit a.ts", 100);
    expect(working.taskStartedAt).toBe(100);
    expect(working.lastAction).toBe("Edit a.ts");
    const waiting = applyStatus(working, "waiting", null, 500);
    expect(waiting.taskStartedAt).toBe(100);
    expect(waiting.lastAction).toBe("Edit a.ts");
  });
  it("counts done once and stops the timer", () => {
    const done = applyStatus(pane({ status: "working", taskStartedAt: 1 }), "done", null, 9);
    expect(done.completedCount).toBe(1);
    expect(done.taskStartedAt).toBeNull();
    expect(applyStatus(done, "done", null, 10).completedCount).toBe(1);
  });
  it("does not count error or idle as completed", () => {
    expect(applyStatus(pane({ status: "working" }), "error", null, 1).completedCount).toBe(0);
    expect(applyStatus(pane({ status: "working" }), "idle", null, 1).completedCount).toBe(0);
  });
});

describe("deck actions", () => {
  beforeEach(() =>
    useDeck.setState({ panes: {}, tabs: [{ id: "t1", name: "S", paneIds: [], layout: null }], activeTabId: "t1" }),
  );

  it("adds and removes panes from their tab", () => {
    deck.addPane(pane(), "t1");
    expect(useDeck.getState().tabs[0].paneIds).toEqual(["p1"]);
    deck.setStatus("p1", "working", null, 5);
    expect(useDeck.getState().panes.p1.taskStartedAt).toBe(5);
    deck.removePane("p1");
    expect(useDeck.getState().panes.p1).toBeUndefined();
    expect(useDeck.getState().tabs[0].paneIds).toEqual([]);
  });
  it("always keeps one tab", () => {
    deck.removeTab("t1");
    expect(useDeck.getState().tabs).toHaveLength(1);
    expect(useDeck.getState().activeTabId).toBe(useDeck.getState().tabs[0].id);
  });
});

describe("sessions archive", () => {
  beforeEach(() =>
    useDeck.setState({
      panes: {},
      tabs: [{ id: "t1", name: "Projet", paneIds: [], layout: null }],
      activeTabId: "t1",
      archived: [],
    }),
  );

  it("archives a tab with its panes and reopens it", () => {
    deck.addPane(pane({ title: "fix bug", fontSize: 18 }), "t1");
    expect(deck.archiveTab("t1", 42)).toEqual(["p1"]);
    const state = useDeck.getState();
    expect(state.panes).toEqual({});
    expect(state.tabs.map((t) => t.id)).not.toContain("t1");
    expect(state.archived[0]).toMatchObject({ id: "t1", name: "Projet", savedAt: 42 });
    expect(state.archived[0].panes[0]).toMatchObject({ id: "p1", title: "fix bug", fontSize: 18 });

    const reopened = deck.unarchive("t1");
    expect(reopened?.panes).toHaveLength(1);
    expect(useDeck.getState().archived).toEqual([]);
    expect(useDeck.getState().activeTabId).toBe(reopened?.tabId);
  });

  it("deletes an archived session", () => {
    deck.archiveTab("t1");
    deck.deleteArchived("t1");
    expect(useDeck.getState().archived).toEqual([]);
    expect(deck.unarchive("t1")).toBeNull();
  });
});

describe("zoom", () => {
  it("clamps font size", () => {
    expect(clampFontSize(2)).toBe(FONT_SIZE.min);
    expect(clampFontSize(99)).toBe(FONT_SIZE.max);
    expect(clampFontSize(14.6)).toBe(15);
    expect(clampFontSize(Number.NaN)).toBe(FONT_SIZE.default);
  });
  it("zooms within range and resets", () => {
    useDeck.setState({ panes: {}, tabs: [{ id: "t1", name: "S", paneIds: [], layout: null }] });
    deck.addPane(pane({ fontSize: 32 }), "t1");
    deck.zoom("p1", 1);
    expect(useDeck.getState().panes.p1.fontSize).toBe(32);
    deck.zoom("p1", "reset");
    expect(useDeck.getState().panes.p1.fontSize).toBe(FONT_SIZE.default);
  });
});

describe("interruptKey", () => {
  it("sends Esc to agents and Ctrl+C to a shell", () => {
    expect(interruptKey("claude")).toBe("");
    expect(interruptKey("codex")).toBe("");
    expect(interruptKey("gemini")).toBe("");
    expect(interruptKey("shell")).toBe("");
  });
});
