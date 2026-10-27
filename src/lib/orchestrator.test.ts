import { beforeEach, describe, expect, it, vi } from "vitest";

const ipc = vi.hoisted(() => ({
  spawnPane: vi.fn(),
  killPane: vi.fn(async () => undefined),
  setActivePane: vi.fn(),
  resizePane: vi.fn(),
  writeToPane: vi.fn(),
  mcpRegister: vi.fn(async () => ({ args: ["--mcp-config", "cfg.json"], secret: "s" })),
  mcpRespond: vi.fn(),
  mcpSetPaused: vi.fn(),
  gitPaneDir: vi.fn(async () => "C:\\proj"),
  gitStatus: vi.fn(async () => ({ branch: "agentdeck/a", files: [{ path: "x.ts" }] })),
  gitDiff: vi.fn(async () => "diff x.ts\n"),
  gitCommit: vi.fn(async () => "abc1234"),
  gitMergeWorktree: vi.fn(async () => "merged"),
  gitListCheckpoints: vi.fn(async () => [{ sha: "f".repeat(40), ts: 0 }]),
  gitRestoreCheckpoint: vi.fn(async () => "e".repeat(40)),
}));

vi.mock("./ipc", () => ({
  ...ipc,
  onPaneOutput: vi.fn(async () => () => undefined),
  onPaneExit: vi.fn(async () => () => undefined),
  onAgentStatus: vi.fn(async () => () => undefined),
  onMcpRequest: vi.fn(async () => () => undefined),
  onMcpActivity: vi.fn(async () => () => undefined),
}));
vi.mock("./notify", () => ({ notify: vi.fn() }));

const { handleMcpRequest, resolveInside, summarizeCall } = await import("./orchestrator");
const { deck, newPane, newTab, newWorkspace, useDeck } = await import("../store/deck");
const { useTasks } = await import("../store/tasks");

const spec = (id: string, extra: object = {}) => ({
  id,
  agent: "claude" as const,
  title: id,
  cwd: "C:\\proj",
  args: [],
  worktree: false,
  completedCount: 0,
  fontSize: 13,
  ...extra,
});

const call = (tool: string, args: Record<string, unknown> = {}) =>
  handleMcpRequest({ orchestratorId: "orch", tool, args });

beforeEach(() => {
  vi.clearAllMocks();
  const ws = newWorkspace("C:\\proj");
  const other = newWorkspace("C:\\other");
  const tab = { ...newTab("S1", ws.id), paneIds: ["orch", "a"] };
  const tab2 = { ...newTab("S2", ws.id), paneIds: ["b"] };
  const foreign = { ...newTab("X", other.id), paneIds: ["x"] };
  useDeck.setState({
    workspaces: [ws, other],
    tabs: [tab, tab2, foreign],
    activeTabId: tab.id,
    panes: {
      orch: newPane(spec("orch", { orchestrator: true })),
      a: newPane(spec("a")),
      b: newPane(spec("b", { agent: "codex" })),
      x: newPane(spec("x")),
    },
  });
  useTasks.setState({}, true);
});

describe("handleMcpRequest", () => {
  it("lists the workspace's agents, never itself or other workspaces", async () => {
    const list = (await call("list_agents")) as { paneId: string; session: string }[];
    expect(list.map((a) => [a.paneId, a.session])).toEqual([
      ["a", "S1"],
      ["b", "S2"],
    ]);
  });

  it("refuses non-orchestrators, itself and foreign panes", async () => {
    await expect(handleMcpRequest({ orchestratorId: "a", tool: "list_agents", args: {} })).rejects.toThrow("not found");
    await expect(call("get_status", { paneId: "orch" })).rejects.toThrow("yourself");
    await expect(call("send_prompt", { paneId: "x", text: "hi" })).rejects.toThrow("not found");
    expect(ipc.writeToPane).not.toHaveBeenCalled();
  });

  it("sends prompts, but never to an agent waiting for a permission", async () => {
    await call("send_prompt", { paneId: "a", text: "line1\nline2", submit: true });
    expect(ipc.writeToPane).toHaveBeenCalledWith("a", "\x1b[200~line1\nline2\x1b[201~");
    expect(ipc.writeToPane).toHaveBeenLastCalledWith("a", "\r");
    await call("send_prompt", { paneId: "a", text: "draft", submit: false });
    expect(ipc.writeToPane).toHaveBeenLastCalledWith("a", "draft");
    deck.setStatus("b", "waiting", null);
    await expect(call("send_prompt", { paneId: "b", text: "1" })).rejects.toThrow("waiting");
    await expect(call("interrupt", { paneId: "b" })).rejects.toThrow("waiting");
    expect(ipc.writeToPane).toHaveBeenCalledTimes(3);
  });

  it("returns the last finished task", async () => {
    useTasks.setState({
      a: {
        current: null,
        history: [
          {
            prompt: "fix",
            startedAt: 1000,
            endedAt: 4000,
            summary: "fixed",
            status: "done",
            actions: Array.from({ length: 40 }, (_, i) => ({ at: i, text: `act${i}` })),
          },
        ],
      },
    });
    const r = (await call("get_result", { paneId: "a" })) as {
      result: { summary: string; actions: string[]; durationSec: number };
    };
    expect(r.result.summary).toBe("fixed");
    expect(r.result.actions).toHaveLength(30);
    expect(r.result.actions[29]).toBe("act39");
    expect(r.result.durationSec).toBe(3);
    expect(await call("get_result", { paneId: "b" })).toMatchObject({ result: null });
  });

  it("reports statuses for wait_for and spawns into the orchestrator's session", async () => {
    deck.setStatus("b", "done", null);
    expect(await call("wait_for", { paneIds: ["a", "b"] })).toEqual({ statuses: { a: "idle", b: "done" } });
    const r = (await call("spawn_agent", { agent: "codex", title: "Tests", cwd: "sub", args: [] })) as {
      paneId: string;
    };
    const pane = useDeck.getState().panes[r.paneId];
    expect(pane.cwd).toBe("C:\\proj\\sub");
    expect(useDeck.getState().tabs[0].paneIds).toContain(r.paneId);
    expect(ipc.spawnPane).toHaveBeenCalledOnce();
    await expect(call("spawn_agent", { agent: "codex", title: "t", cwd: "C:\\other" })).rejects.toThrow("stay inside");
  });
});

describe("git tools", () => {
  it("returns the diff with restore points", async () => {
    const r = (await call("get_diff", { paneId: "a" })) as { diff: string; checkpoints: unknown[] };
    expect(r.diff).toBe("diff x.ts\n");
    expect(r.checkpoints).toHaveLength(1);
  });

  it("commits before merging when given a message, refuses while working", async () => {
    await call("merge_worktree", { paneId: "a", commitMessage: "feat: x" });
    expect(ipc.gitCommit).toHaveBeenCalledWith("C:\\proj", "feat: x");
    deck.updatePane("a", (p) => ({ ...p, status: "working" }));
    await expect(call("merge_worktree", { paneId: "a" })).rejects.toThrow("still working");
    await expect(call("restore_checkpoint", { paneId: "a" })).rejects.toThrow("interrupt");
  });

  it("restores the latest checkpoint by default and closes agents", async () => {
    await call("restore_checkpoint", { paneId: "b" });
    expect(ipc.gitRestoreCheckpoint).toHaveBeenCalledWith("C:\\proj", "b", "f".repeat(40));
    await call("close_agent", { paneId: "b" });
    expect(useDeck.getState().panes.b).toBeUndefined();
    await expect(call("close_agent", { paneId: "x" })).rejects.toThrow("not found");
  });
});

describe("resolveInside", () => {
  it("keeps paths inside the root", () => {
    expect(resolveInside("C:\\proj", "")).toBe("C:\\proj");
    expect(resolveInside("C:\\proj", "C:/proj/a")).toBe("C:/proj/a");
    expect(() => resolveInside("C:\\proj", "C:\\project2")).toThrow();
    expect(() => resolveInside("C:\\proj", "a\\..\\..\\x")).toThrow("..");
    expect(resolveInside("/home/u/p", "src")).toBe("/home/u/p/src");
  });
});

describe("summarizeCall", () => {
  it("names targets by title", () => {
    const { panes } = useDeck.getState();
    expect(summarizeCall("send_prompt", { paneId: "b", text: "go  now" }, panes)).toBe("b (codex): go now");
    expect(summarizeCall("wait_for", { paneIds: ["a", "zzzzzzzzzz"], until: "done" }, panes)).toBe(
      "a (claude), zzzzzzzz → done",
    );
  });
});
