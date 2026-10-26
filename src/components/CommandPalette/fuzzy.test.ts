import { describe, expect, it } from "vitest";
import { fuzzyScore } from "./fuzzy";

describe("fuzzyScore", () => {
  it("matches subsequences, ignoring case, accents and spaces", () => {
    expect(fuzzyScore("SETT", "Open settings")).not.toBeNull();
    expect(fuzzyScore("resume", "Résumé")).not.toBeNull();
    expect(fuzzyScore("stop a", "Stop all")).not.toBeNull();
    expect(fuzzyScore("", "anything")).toBe(0);
  });
  it("rejects missing characters and wrong order", () => {
    expect(fuzzyScore("xyz", "Stop all")).toBeNull();
    expect(fuzzyScore("ba", "ab")).toBeNull();
  });
  it("ranks substring and word-start matches above scattered ones", () => {
    const sub = fuzzyScore("wall", "Wall mode") ?? -1;
    const scattered = fuzzyScore("wall", "Window with all lines") ?? -1;
    expect(sub).toBeGreaterThan(scattered);
    const wordStart = fuzzyScore("ns", "New session") ?? -1;
    const middle = fuzzyScore("ns", "Maximize panes") ?? -1;
    expect(wordStart).toBeGreaterThan(middle);
  });
  it("prefers shorter text on equal matches", () => {
    expect(fuzzyScore("claude", "Claude") ?? -1).toBeGreaterThan(fuzzyScore("claude", "Claude Code session") ?? -1);
  });
});
