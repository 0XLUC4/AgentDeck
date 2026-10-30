import { describe, expect, it, vi } from "vitest";
import type { AgentSubagentEvent } from "../../lib/types";

vi.mock("../../lib/ipc", () => ({ onAgentStatus: vi.fn(), onAgentSubagent: vi.fn() }));
const { applySubagentEvent, clearFinished, dismissSubagent, parseSubagentPanel, subagentPanelId } = await import(
  "./subagents"
);

const ev = (event: AgentSubagentEvent["event"], extra: Partial<AgentSubagentEvent> = {}): AgentSubagentEvent => ({
  paneId: "p1",
  agentId: "a1",
  agentType: null,
  event,
  action: null,
  summary: null,
  ...extra,
});

describe("subagents", () => {
  it("tracks start, tool calls and stop, creating the subagent on an early tool call", () => {
    let s = applySubagentEvent({}, ev("PreToolUse", { action: "Read a.ts" }), 1);
    s = applySubagentEvent(s, ev("SubagentStart", { agentType: "Explore" }), 2);
    s = applySubagentEvent(s, ev("PreToolUse", { action: "Grep foo" }), 3);
    s = applySubagentEvent(s, ev("SubagentStop", { summary: "found it" }), 4);
    expect(s.p1).toEqual([
      {
        id: "a1",
        type: "Explore",
        startedAt: 1,
        endedAt: 4,
        summary: "found it",
        actions: [
          { at: 1, text: "Read a.ts" },
          { at: 3, text: "Grep foo" },
        ],
      },
    ]);
  });

  it("clears finished subagents only, and dismisses one", () => {
    let s = applySubagentEvent({}, ev("SubagentStart"), 1);
    s = applySubagentEvent(s, ev("SubagentStart", { agentId: "a2" }), 1);
    s = applySubagentEvent(s, ev("SubagentStop"), 2);
    expect(clearFinished(s, "p1").p1.map((a) => a.id)).toEqual(["a2"]);
    expect(dismissSubagent(dismissSubagent(s, "p1", "a1"), "p1", "a2")).toEqual({});
    expect(clearFinished(s, "other")).toBe(s);
  });

  it("round-trips panel ids, agent ids with colons included", () => {
    expect(parseSubagentPanel(subagentPanelId("p1", "x:y"))).toEqual({ paneId: "p1", agentId: "x:y" });
    expect(parseSubagentPanel("browser:p1")).toBeNull();
  });
});
