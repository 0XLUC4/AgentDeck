import { describe, expect, it } from "vitest";
import { fuzzyScore } from "./fuzzy";

describe("fuzzyScore", () => {
  it("matches subsequences, ignoring case, accents and spaces", () => {
    expect(fuzzyScore("REGL", "Ouvrir les réglages")).not.toBeNull();
    expect(fuzzyScore("tout arr", "Tout arrêter")).not.toBeNull();
    expect(fuzzyScore("", "anything")).toBe(0);
  });
  it("rejects missing characters and wrong order", () => {
    expect(fuzzyScore("xyz", "Tout arrêter")).toBeNull();
    expect(fuzzyScore("ba", "ab")).toBeNull();
  });
  it("ranks substring and word-start matches above scattered ones", () => {
    const sub = fuzzyScore("mur", "Mode mur") ?? -1;
    const scattered = fuzzyScore("mur", "Mode multiple ouvert") ?? -1;
    expect(sub).toBeGreaterThan(scattered);
    const wordStart = fuzzyScore("ns", "Nouvelle session") ?? -1;
    const middle = fuzzyScore("ns", "Agrandir pane actif") ?? -1;
    expect(wordStart).toBeGreaterThan(middle);
  });
  it("prefers shorter text on equal matches", () => {
    expect(fuzzyScore("claude", "Claude") ?? -1).toBeGreaterThan(fuzzyScore("claude", "Claude Code session") ?? -1);
  });
});
