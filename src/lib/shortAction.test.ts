import { describe, expect, it } from "vitest";
import { shortAction } from "./shortAction";

describe("shortAction", () => {
  it("turns hook actions into a few words", () => {
    expect(shortAction("Edit src/store/deck.ts")).toBe("Editing deck.ts");
    expect(shortAction("Bash: pnpm test --run src/x")).toBe("Running pnpm test --run");
    expect(shortAction("WebFetch https://docs.rs/tauri/latest")).toBe("Opening docs.rs");
    expect(shortAction("mcp__agentdeck__send_prompt")).toBe("send prompt");
    expect(shortAction("TodoWrite")).toBe("TodoWrite");
    expect(shortAction(null)).toBe("");
  });
});
