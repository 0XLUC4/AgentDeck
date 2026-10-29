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

const { closePane, createPane, focusPane, reportSize, restartPane } = await import("./runtime");
const { deck, useDeck } = await import("./deck");
const deckSetDone = (id: string) => deck.setStatus(id, "done", null);

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

  it("acknowledges a finished pane when it gets focus", async () => {
    await createPane(spec("p4"), "t1");
    useDeck.setState({ activePaneId: "p4" });
    deckSetDone("p4");
    focusPane("p4"); // already active: still counts as seen
    expect(useDeck.getState().panes.p4.status).toBe("idle");
  });

  it("does not spawn or keep listeners for a pane closed while subscribing", async () => {
    const creating = createPane(spec("p3"), "t1");
    void closePane("p3");
    await creating;
    expect(ipc.spawnPane).not.toHaveBeenCalled();
    expect(exitHandlers.has("p3")).toBe(false);
  });

  it("closes the pane when the agent exits by itself, but not on restart", async () => {
    await createPane(spec("p6"), "t1");
    await restartPane("p6");
    expect(useDeck.getState().panes.p6).toBeDefined();
    exitHandlers.get("p6")?.(); // e.g. Ctrl+C confirmed in Claude Code
    expect(useDeck.getState().panes.p6).toBeUndefined();
    expect(ipc.killPane).toHaveBeenLastCalledWith("p6");
  });

  it("adopts a live pane without spawning and repaints it on the first size report", async () => {
    await createPane(spec("p9"), "t1", true);
    expect(ipc.spawnPane).not.toHaveBeenCalled();
    reportSize("p9", 100, 30);
    await vi.waitFor(() => expect(ipc.resizePane).toHaveBeenCalledTimes(2));
    expect(ipc.resizePane.mock.calls).toEqual([
      ["p9", 99, 30],
      ["p9", 100, 30],
    ]);
  });

  it("treats 'pane id already in use' as adopt, not as an error", async () => {
    ipc.spawnPane.mockRejectedValueOnce("pane id already in use");
    await createPane(spec("p5"), "t1");
    const pane = useDeck.getState().panes.p5;
    expect(pane.status).not.toBe("error");
    expect(pane.exited).toBe(false);
  });
});
