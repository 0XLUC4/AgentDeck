import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-store", () => ({ load: vi.fn(() => Promise.reject(new Error("no tauri"))) }));
vi.mock("./ipc", () => ({ getVoiceConfig: vi.fn() }));

const { cleanForSpeech, speechText } = await import("./tts");

describe("TTS text cleanup", () => {
  it("strips markdown and code", () => {
    const md =
      "## Fait\n\n- Ajouté `parseArgs` dans **src/lib**\n- Voir [la doc](https://x.dev)\n\n```ts\nconst a = 1;\n```\nFini.";
    expect(cleanForSpeech(md)).toBe("Fait Ajouté parseArgs dans src/lib Voir la doc Fini.");
  });

  it("drops an unterminated code block and bare URLs", () => {
    expect(cleanForSpeech("Voir https://example.com ici\n```\ncode")).toBe("Voir ici");
  });

  it("cuts long text at a word boundary", () => {
    const text = cleanForSpeech("mot ".repeat(100));
    expect(text.length).toBeLessThanOrEqual(201);
    expect(text.endsWith("mot…")).toBe(true);
  });

  it("builds the sentence", () => {
    expect(speechText("Codex", "Tests **verts**.")).toBe("Codex a terminé : Tests verts.");
    expect(speechText("Codex", "```\nx\n```")).toBe("Codex a terminé");
    expect(speechText("Codex", null)).toBe("Codex a terminé");
  });
});
