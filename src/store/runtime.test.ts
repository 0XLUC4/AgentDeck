import { beforeEach, describe, expect, it, vi } from "vitest";

// Fake backend: spawn succeeds, kill fires the pane's exit event.
const exitHandlers = new Map<string, () => void>();
const ipc = vi.hoisted(() => ({
  spawnPane: vi.fn(),
  killPane: vi.fn(),
  setActivePane: vi.fn(),
  resizePane: vi.fn(),
  writeToPane: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  ...ipc,
  onPaneOutput: vi.fn(async () => () => undefined),
  onPaneExit: vi.fn(async (id: string, cb: () => void) => {
    exitHandlers.set(id, cb);
    return () => exitHandlers.delete(id);
  }),
  onAgentStatus: vi.fn(async () => () => undefined),
}));
vi.mock("../lib/notify", () => ({ notify: vi.fn() }));

const { closePane, createPane, focusPane, restartPane } = await import("./runtime");
const { useDeck } = await import("./deck");

const spec = (id: string) => ({
  id,
  agent: "claude" as const,
  title: "t",
  cwd: "/",
  args: [],
  worktree: false,
  completedCount: 0,
  fontSize: 13,
});

beforeEach(() => {
  vi.clearAllMocks();
  ipc.spawnPane.mockResolvedValue(undefined);
  ipc.setActivePane.mockResolvedValue(undefined);
  ipc.resizePane.mockResolvedValue(undefined);
  ipc.killPane.mockImplementation(async (id: string) => exitHandlers.get(id)?.());
  useDeck.setState({
    workspaces: [{ id: "w1", name: "W", path: "" }],
    panes: {},
    tabs: [{ id: "t1", name: "S", workspaceId: "w1", paneIds: [], layout: null }],
    activeTabId: "t1",
    activePaneId: null,
  });
});

describe("runtime", () => {
  it("dedupes concurrent restarts of the same pane", async () => {
    await createPane(spec("p1"), "t1");
    expect(ipc.spawnPane).toHaveBeenCalledTimes(1);
    const first = restartPane("p1");
    const second = restartPane("p1");
    expect(second).toBe(first);
    await Promise.all([first, second]);
    expect(ipc.spawnPane).toHaveBeenCalledTimes(2);
    expect(useDeck.getState().panes.p1.status).not.toBe("error");
    // Once settled, a new restart runs again.
    await restartPane("p1");
    expect(ipc.spawnPane).toHaveBeenCalledTimes(3);
  });

  it("clears the backend active pane when closing it", async () => {
    await createPane(spec("p2"), "t1");
    focusPane("p2");
    await closePane("p2");
    expect(ipc.setActivePane).toHaveBeenLastCalledWith(null);
    expect(useDeck.getState().activePaneId).toBeNull();
  });

  it("does not spawn or keep listeners for a pane closed while subscribing", async () => {
    const creating = createPane(spec("p3"), "t1");
    void closePane("p3");
    await creating;
    expect(ipc.spawnPane).not.toHaveBeenCalled();
    expect(exitHandlers.has("p3")).toBe(false);
  });
});
