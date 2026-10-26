import { describe, expect, it } from "vitest";
import type { AgentStatusEvent } from "../lib/types";
import { applyTaskEvent, MAX_ACTIONS, MAX_HISTORY, type TaskLog } from "./tasks";

type Ev = Pick<AgentStatusEvent, "status" | "event" | "lastAction" | "prompt" | "summary">;
const ev = (e: Partial<Ev> & Pick<Ev, "status" | "event">): Ev => ({ lastAction: null, ...e });
const prompt = (text: string) => ev({ status: "working", event: "UserPromptSubmit", prompt: text });
const tool = (text: string) => ev({ status: "working", event: "PreToolUse", lastAction: text });
const stop = (summary?: string) => ev({ status: "done", event: "Stop", summary });

const run = (events: Ev[], log: TaskLog = { current: null, history: [] }) =>
  events.reduce((l, e, i) => applyTaskEvent(l, e, i * 1000), log);

describe("applyTaskEvent", () => {
  it("records prompt, deduped actions and summary, then archives", () => {
    const log = run([prompt("fix it"), tool("Read a.ts"), tool("Read a.ts"), tool("Edit a.ts"), stop("Fixed.")]);
    expect(log.current).toBeNull();
    expect(log.history).toHaveLength(1);
    const [task] = log.history;
    expect(task).toMatchObject({ prompt: "fix it", summary: "Fixed.", status: "done", startedAt: 0, endedAt: 4000 });
    expect(task.actions.map((a) => a.text)).toEqual(["Read a.ts", "Edit a.ts"]);
  });

  it("starts on the first working event after done", () => {
    const log = run([prompt("a"), stop("A"), tool("Bash: ls")]);
    expect(log.current?.actions.map((a) => a.text)).toEqual(["Bash: ls"]);
    expect(log.current?.prompt).toBeNull();
  });

  it("ignores events with no running task that do not start one", () => {
    expect(run([ev({ status: "idle", event: "SessionStart" })])).toEqual({ current: null, history: [] });
  });

  it("creates a task from a lone end event (Codex notify)", () => {
    const log = applyTaskEvent(
      undefined,
      ev({ status: "done", event: "agent-turn-complete", prompt: "p", summary: "s" }),
      5000,
      2000,
    );
    expect(log.history[0]).toMatchObject({ prompt: "p", summary: "s", startedAt: 2000, endedAt: 5000 });
  });

  it("uses the error as summary on failure", () => {
    const log = run([prompt("a"), ev({ status: "error", event: "StopFailure", lastAction: "rate_limit" })]);
    expect(log.history[0]).toMatchObject({ status: "error", summary: "rate_limit", actions: [] });
  });

  it("caps actions and history", () => {
    const many = Array.from({ length: MAX_ACTIONS + 5 }, (_, i) => tool(`t${i}`));
    const log = run([prompt("a"), ...many]);
    expect(log.current?.actions).toHaveLength(MAX_ACTIONS);
    expect(log.current?.actions[MAX_ACTIONS - 1]?.text).toBe(`t${MAX_ACTIONS + 4}`);

    const turns = Array.from({ length: MAX_HISTORY + 3 }, (_, i) => [prompt(`p${i}`), stop()]).flat();
    const full = run(turns);
    expect(full.history).toHaveLength(MAX_HISTORY);
    expect(full.history[0].prompt).toBe(`p${MAX_HISTORY + 2}`);
  });

  it("archives an interrupted task when a new prompt arrives", () => {
    const log = run([prompt("a"), tool("x"), prompt("b")]);
    expect(log.current?.prompt).toBe("b");
    expect(log.history[0]).toMatchObject({ prompt: "a", status: "working", endedAt: 2000 });
  });
});
