import { beforeEach, describe, expect, it, vi } from "vitest";

const ipc = vi.hoisted(() => ({
  spawnPane: vi.fn(),
  killPane: vi.fn(),
  setActivePane: vi.fn(),
  resizePane: vi.fn(),
  writeToPane: vi.fn(),
  mcpRegister: vi.fn(async () => ({ args: ["--mcp-config", "cfg.json"], secret: "s" })),
  mcpRespond: vi.fn(),
  mcpSetPaused: vi.fn(),
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
    await expect(handleMcpRequest({ orchestratorId: "a", tool: "list_agents", args: {} })).rejects.toThrow(
      "introuvable",
    );
    await expect(call("get_status", { paneId: "orch" })).rejects.toThrow("toi-même");
    await expect(call("send_prompt", { paneId: "x", text: "hi" })).rejects.toThrow("introuvable");
    expect(ipc.writeToPane).not.toHaveBeenCalled();
  });

  it("sends prompts, but never to an agent waiting for a permission", async () => {
    await call("send_prompt", { paneId: "a", text: "line1\nline2", submit: true });
    expect(ipc.writeToPane).toHaveBeenLastCalledWith("a", "\x1b[200~line1\nline2\x1b[201~\r");
    await call("send_prompt", { paneId: "a", text: "draft", submit: false });
    expect(ipc.writeToPane).toHaveBeenLastCalledWith("a", "draft");
    deck.setStatus("b", "waiting", null);
    await expect(call("send_prompt", { paneId: "b", text: "1" })).rejects.toThrow("attend");
    await expect(call("interrupt", { paneId: "b" })).rejects.toThrow("attend");
    expect(ipc.writeToPane).toHaveBeenCalledTimes(2);
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
    await expect(call("spawn_agent", { agent: "codex", title: "t", cwd: "C:\\other" })).rejects.toThrow("rester");
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
    expect(summarizeCall("send_prompt", { paneId: "b", text: "go  now" }, panes)).toBe("b (codex) : go now");
    expect(summarizeCall("wait_for", { paneIds: ["a", "zzzzzzzzzz"], until: "done" }, panes)).toBe(
      "a (claude), zzzzzzzz → done",
    );
  });
});
