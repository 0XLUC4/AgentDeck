import { beforeEach, describe, expect, it } from "vitest";
import { interruptKey } from "../lib/agents";
import {
  activeWorkspace,
  applyStatus,
  clampFontSize,
  deck,
  FONT_SIZE,
  folderName,
  newPane,
  type Pane,
  sessionStatus,
  useDeck,
  workspaceForPath,
} from "./deck";

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

const W1 = { id: "w1", name: "Par défaut", path: "" };

describe("deck actions", () => {
  beforeEach(() =>
    useDeck.setState({
      workspaces: [W1],
      panes: {},
      tabs: [{ id: "t1", name: "S", workspaceId: "w1", paneIds: [], layout: null }],
      activeTabId: "t1",
    }),
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
      workspaces: [W1],
      panes: {},
      tabs: [{ id: "t1", name: "Projet", workspaceId: "w1", paneIds: [], layout: null }],
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
    expect(state.archived[0]).toMatchObject({ id: "t1", name: "Projet", workspaceId: "w1", savedAt: 42 });
    expect(state.archived[0].panes[0]).toMatchObject({ id: "p1", title: "fix bug", fontSize: 18 });

    const reopened = deck.unarchive("t1");
    expect(reopened?.panes).toHaveLength(1);
    expect(useDeck.getState().archived).toEqual([]);
    expect(useDeck.getState().activeTabId).toBe(reopened?.tabId);
    expect(useDeck.getState().tabs.find((t) => t.id === reopened?.tabId)?.workspaceId).toBe("w1");
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
    useDeck.setState({ panes: {}, tabs: [{ id: "t1", name: "S", workspaceId: "w1", paneIds: [], layout: null }] });
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

describe("workspaces", () => {
  beforeEach(() =>
    useDeck.setState({
      workspaces: [W1],
      panes: {},
      tabs: [{ id: "t1", name: "S", workspaceId: "w1", paneIds: [], layout: null }],
      activeTabId: "t1",
      activePaneId: null,
      archived: [],
    }),
  );

  it("links a folder as a workspace with one session and shows it", () => {
    const ws = deck.linkWorkspace("D:\\code\\my-app\\");
    expect(ws.name).toBe("my-app");
    expect(ws.path).toBe("D:\\code\\my-app\\");
    const state = useDeck.getState();
    expect(state.tabs.filter((t) => t.workspaceId === ws.id)).toHaveLength(1);
    expect(activeWorkspace().id).toBe(ws.id);
  });

  it("switches workspace through its tabs", () => {
    const ws = deck.linkWorkspace("/home/me/api");
    deck.selectWorkspace("w1");
    expect(useDeck.getState().activeTabId).toBe("t1");
    deck.selectWorkspace(ws.id);
    expect(activeWorkspace().id).toBe(ws.id);
  });

  it("new sessions go to the active workspace", () => {
    const ws = deck.linkWorkspace("/srv/site");
    const id = deck.addTab();
    expect(useDeck.getState().tabs.find((t) => t.id === id)?.workspaceId).toBe(ws.id);
  });

  it("keeps one open session per workspace", () => {
    const ws = deck.linkWorkspace("/srv/site");
    const only = useDeck.getState().activeTabId;
    deck.removeTab(only);
    const left = useDeck.getState().tabs.filter((t) => t.workspaceId === ws.id);
    expect(left).toHaveLength(1);
    expect(useDeck.getState().activeTabId).toBe(left[0].id);
  });

  it("unlinks a workspace with its sessions and returns its panes to kill", () => {
    const ws = deck.linkWorkspace("/srv/site");
    deck.addPane(pane({ id: "p9" }), useDeck.getState().activeTabId);
    deck.archiveTab(deck.addTab());
    expect(deck.unlinkWorkspace(ws.id)).toEqual(["p9"]);
    const state = useDeck.getState();
    expect(state.workspaces.map((w) => w.id)).toEqual(["w1"]);
    expect(state.tabs.every((t) => t.workspaceId === "w1")).toBe(true);
    expect(state.archived).toEqual([]);
    expect(state.panes.p9).toBeUndefined();
    expect(state.activeTabId).toBe("t1");
  });

  it("recreates a default workspace when the last one is unlinked", () => {
    deck.unlinkWorkspace("w1");
    const state = useDeck.getState();
    expect(state.workspaces).toHaveLength(1);
    expect(state.workspaces[0].path).toBe("");
    expect(state.tabs).toHaveLength(1);
    expect(state.tabs[0].workspaceId).toBe(state.workspaces[0].id);
    expect(state.activeTabId).toBe(state.tabs[0].id);
  });

  it("stores git info and finds a workspace by path regardless of case and slashes", () => {
    const ws = deck.linkWorkspace("C:\\Code\\App", "Mon app", { branch: "main" });
    expect(ws).toMatchObject({ name: "Mon app", git: { branch: "main" } });
    const all = useDeck.getState().workspaces;
    expect(workspaceForPath("c:/code/app/", all)?.id).toBe(ws.id);
    expect(workspaceForPath("C:\\Code\\Other", all)).toBeUndefined();
    expect(workspaceForPath("", all)).toBeUndefined();
  });

  it("renames a workspace", () => {
    deck.renameWorkspace("w1", "Perso");
    expect(useDeck.getState().workspaces[0].name).toBe("Perso");
  });
});

describe("helpers", () => {
  it("takes the last folder name of any path", () => {
    expect(folderName("C:\\Users\\me\\proj")).toBe("proj");
    expect(folderName("/a/b/")).toBe("b");
    expect(folderName("")).toBe("");
  });

  it("shows the most urgent pane status for a session", () => {
    const panes = {
      a: pane({ id: "a", status: "working" }),
      b: pane({ id: "b", status: "waiting" }),
      c: pane({ id: "c", status: "done" }),
    };
    expect(sessionStatus(["a", "b", "c"], panes)).toBe("waiting");
    expect(sessionStatus(["a", "c"], panes)).toBe("working");
    expect(sessionStatus([], panes)).toBeNull();
  });
});
