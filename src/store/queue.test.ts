import { afterEach, describe, expect, it, vi } from "vitest";

const ipc = vi.hoisted(() => ({ writeToPane: vi.fn(async () => undefined) }));
vi.mock("../lib/ipc", () => ipc);

const { ADVANCE_DELAY_MS, moveItem, queues, readyForNext, shouldAdvance, startQueueRunner, useQueues } = await import(
  "./queue"
);
const { deck, newPane } = await import("./deck");

const q = (n: number, paused = false) => ({
  items: Array.from({ length: n }, (_, i) => ({ id: `${i}`, text: `p${i}` })),
  paused,
});

describe("queue advance logic", () => {
  it("advances only on a transition to done with a non-empty, running queue", () => {
    expect(shouldAdvance("working", "done", q(1))).toBe(true);
    expect(shouldAdvance(undefined, "done", q(1))).toBe(true);
    expect(shouldAdvance("done", "done", q(1))).toBe(false);
    expect(shouldAdvance("working", "waiting", q(1))).toBe(false);
    expect(shouldAdvance("working", "done", q(0))).toBe(false);
    expect(shouldAdvance("working", "done", q(2, true))).toBe(false);
    expect(shouldAdvance("working", "done", undefined)).toBe(false);
  });

  it("sends only to a live pane that is not busy again", () => {
    const pane = newPane({
      id: "a",
      agent: "claude",
      title: "",
      cwd: "",
      args: [],
      worktree: false,
      completedCount: 0,
      fontSize: 13,
    });
    expect(readyForNext({ ...pane, status: "done" })).toBe(true);
    expect(readyForNext({ ...pane, status: "idle" })).toBe(true);
    expect(readyForNext({ ...pane, status: "working" })).toBe(false);
    expect(readyForNext({ ...pane, status: "done", exited: true })).toBe(false);
    expect(readyForNext(undefined)).toBe(false);
  });

  it("reorders items, clamped", () => {
    expect(moveItem(["a", "b", "c"], 0, 1)).toEqual(["b", "a", "c"]);
    expect(moveItem(["a", "b", "c"], 2, -5)).toEqual(["c", "a", "b"]);
    expect(moveItem(["a", "b"], 1, 1)).toEqual(["a", "b"]);
    expect(moveItem(["a"], -1, 1)).toEqual(["a"]);
  });
});

describe("queue runner", () => {
  afterEach(() => vi.useRealTimers());

  it("sends the next prompt 1.5 s after the pane is done", async () => {
    vi.useFakeTimers();
    const stop = startQueueRunner();
    const tabId = deck.addTab();
    deck.addPane(
      newPane({
        id: "p",
        agent: "claude",
        title: "",
        cwd: "",
        args: [],
        worktree: false,
        completedCount: 0,
        fontSize: 13,
      }),
      tabId,
    );
    queues.add("p", "first");
    queues.add("p", "multi\nline");
    deck.setStatus("p", "working", null);
    deck.setStatus("p", "done", null);
    expect(ipc.writeToPane).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(ADVANCE_DELAY_MS);
    expect(ipc.writeToPane).toHaveBeenCalledWith("p", "first\r");
    expect(useQueues.getState().p.items.map((i) => i.text)).toEqual(["multi\nline"]);

    // Paused: nothing is sent.
    queues.setPaused("p", true);
    deck.setStatus("p", "working", null);
    deck.setStatus("p", "done", null);
    await vi.advanceTimersByTimeAsync(ADVANCE_DELAY_MS);
    expect(ipc.writeToPane).toHaveBeenCalledTimes(1);

    queues.setPaused("p", false);
    deck.setStatus("p", "working", null);
    deck.setStatus("p", "done", null);
    await vi.advanceTimersByTimeAsync(ADVANCE_DELAY_MS);
    expect(ipc.writeToPane).toHaveBeenLastCalledWith("p", "\x1b[200~multi\nline\x1b[201~\r");

    deck.removePane("p");
    expect(useQueues.getState().p).toBeUndefined();
    stop();
  });
});
