import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-store", () => ({ load: vi.fn() }));
vi.mock("./runtime", () => ({ createPane: vi.fn() }));

const { hydrate, migrate, planRestore } = await import("./persist");
const { useDeck } = await import("./deck");

const session = { id: "s1", name: "Session 1", savedAt: 1, layout: null, panes: [] };

describe("migrate", () => {
  it("moves v1 sessions into a 'Default' workspace", () => {
    const saved = migrate({
      version: 1,
      tabs: [session],
      activeTabId: "s1",
      lastCwd: "D:/proj",
      archived: [{ ...session, id: "old" }],
    });
    expect(saved?.version).toBe(2);
    expect(saved?.workspaces).toEqual([{ id: expect.any(String), name: "Default", path: "" }]);
    const wsId = saved?.workspaces[0].id;
    expect(saved?.tabs[0].workspaceId).toBe(wsId);
    expect(saved?.archived[0]).toMatchObject({ id: "old", workspaceId: wsId });
    expect(saved?.lastCwd).toBe("D:/proj");
  });

  it("handles v1 without archive", () => {
    expect(migrate({ version: 1, tabs: [session], activeTabId: "s1", lastCwd: "" })?.archived).toEqual([]);
  });

  it("ignores empty or unknown data", () => {
    expect(migrate(undefined)).toBeNull();
    expect(migrate({ version: 1, tabs: [], activeTabId: "", lastCwd: "" })).toBeNull();
    expect(migrate({ version: 99 })).toBeNull();
  });
});

describe("hydrate", () => {
  it("restores workspaces, gives every workspace a tab and repairs orphan sessions", () => {
    const toSpawn = hydrate({
      version: 2,
      workspaces: [
        { id: "w1", name: "A", path: "/a" },
        { id: "w2", name: "B", path: "/b" },
      ],
      tabs: [{ ...session, workspaceId: "gone" }],
      activeTabId: "missing",
      lastCwd: "/a",
      archived: [],
    });
    const state = useDeck.getState();
    expect(toSpawn[0].workspaceId).toBe("w1");
    expect(state.tabs.map((t) => t.workspaceId).sort()).toEqual(["w1", "w2"]);
    expect(state.activeTabId).toBe("s1");
  });
});

describe("planRestore", () => {
  const pane = (id: string) => ({
    id,
    agent: "claude" as const,
    title: "t",
    cwd: "/",
    args: [],
    worktree: false,
    completedCount: 0,
    fontSize: 13,
  });
  const tabs = [
    { ...session, workspaceId: "w", panes: [pane("a"), pane("b")] },
    { ...session, id: "s2", workspaceId: "w", panes: [pane("c")] },
  ];

  it("adopts live restored panes, kills live orphans, spawns the rest", () => {
    const { adopt, orphans } = planRestore(tabs, ["b", "zombie", "c"]);
    expect([...adopt].sort()).toEqual(["b", "c"]);
    expect(orphans).toEqual(["zombie"]);
    expect(adopt.has("a")).toBe(false); // not alive: spawned as usual
  });

  it("spawns everything on a cold start", () => {
    expect(planRestore(tabs, [])).toEqual({ adopt: new Set(), orphans: [] });
  });

  it("kills every live pane when nothing is restored", () => {
    expect(planRestore([], ["x"]).orphans).toEqual(["x"]);
  });
});
