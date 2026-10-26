import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-store", () => ({ load: vi.fn(() => Promise.reject(new Error("no tauri"))) }));
vi.mock("./ipc", () => ({ getVoiceConfig: vi.fn() }));

const { cleanForSpeech, speechText } = await import("./tts");

describe("TTS text cleanup", () => {
  it("strips markdown and code", () => {
    const md =
      "## Done\n\n- Added `parseArgs` in **src/lib**\n- See [the docs](https://x.dev)\n\n```ts\nconst a = 1;\n```\nFinished.";
    expect(cleanForSpeech(md)).toBe("Done Added parseArgs in src/lib See the docs Finished.");
  });

  it("drops an unterminated code block and bare URLs", () => {
    expect(cleanForSpeech("See https://example.com here\n```\ncode")).toBe("See here");
  });

  it("cuts long text at a word boundary", () => {
    const text = cleanForSpeech("word ".repeat(100));
    expect(text.length).toBeLessThanOrEqual(201);
    expect(text.endsWith("word…")).toBe(true);
  });

  it("builds the sentence", () => {
    expect(speechText("Codex", "Tests **green**.")).toBe("Codex finished: Tests green.");
    expect(speechText("Codex", "```\nx\n```")).toBe("Codex finished");
    expect(speechText("Codex", null)).toBe("Codex finished");
  });
});
