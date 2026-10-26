import { describe, expect, it } from "vitest";
import { forwardsInput, replayGate } from "./detach";

function setup() {
  const written: string[] = [];
  return { written, gate: replayGate((d) => written.push(d)) };
}

describe("replayGate", () => {
  it("holds live output until the replay, then writes replay first", () => {
    const { written, gate } = setup();
    gate.output("c");
    gate.replay("ab");
    gate.output("d");
    expect(written.join("")).toBe("abcd");
  });

  it("drops buffered chunks already contained in the replay", () => {
    const { written, gate } = setup();
    gate.output("c");
    gate.output("d");
    gate.replay("abc");
    expect(written.join("")).toBe("abcd");
  });

  it("handles an empty scrollback and ignores a second replay", () => {
    const { written, gate } = setup();
    gate.output("x");
    gate.replay("");
    gate.replay("late");
    expect(written).toEqual(["x"]);
  });
});

describe("forwardsInput", () => {
  it("lets exactly one window own the pane", () => {
    expect(forwardsInput(false, false)).toBe(true); // main, attached
    expect(forwardsInput(true, false)).toBe(false); // stray detached window
    expect(forwardsInput(false, true)).toBe(false); // main, detached
    expect(forwardsInput(true, true)).toBe(true); // detached window
  });
});
