import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-store", () => ({ load: vi.fn(() => Promise.reject(new Error("no tauri"))) }));

const { DEFAULT_SETTINGS, mergeSettings } = await import("./settings");

describe("mergeSettings", () => {
  it("returns defaults for missing or garbage input", () => {
    expect(mergeSettings(undefined)).toEqual(DEFAULT_SETTINGS);
    expect(mergeSettings("nope")).toEqual(DEFAULT_SETTINGS);
    expect(mergeSettings({ notify: 3, agents: [] })).toEqual(DEFAULT_SETTINGS);
  });

  it("does not share nested objects with the defaults", () => {
    const s = mergeSettings(null);
    s.notify.sound = false;
    expect(DEFAULT_SETTINGS.notify.sound).toBe(true);
  });

  it("keeps valid values and replaces invalid ones field by field", () => {
    const s = mergeSettings({
      fontSize: 40,
      fontFamily: "  ",
      confirmBulk: false,
      sidebarOpen: "yes",
      notify: { sound: false, error: true, enabled: 1 },
      agents: { claude: { args: "--model opus", worktree: true }, codex: { args: 5 }, bogus: { args: "x" } },
      unknown: true,
    });
    expect(s.fontSize).toBe(32);
    expect(s.fontFamily).toBe(DEFAULT_SETTINGS.fontFamily);
    expect(s.confirmBulk).toBe(false);
    expect(s.sidebarOpen).toBe(DEFAULT_SETTINGS.sidebarOpen);
    expect(s.notify).toEqual({ ...DEFAULT_SETTINGS.notify, sound: false, error: true });
    expect(s.agents.claude).toEqual({ args: "--model opus", worktree: true });
    expect(s.agents.codex).toEqual({ args: "", worktree: false });
    expect(Object.keys(s.agents).sort()).toEqual(["claude", "codex", "gemini", "grok", "shell"]);
    expect(s).not.toHaveProperty("unknown");
  });

  it("validates auto-allow rules, budget and phone URL", () => {
    const s = mergeSettings({
      autoAllow: [{ rule: " Bash(ls) ", folder: "D:\\p" }, { rule: "" }, "Read", { rule: "Edit", folder: 3 }],
      budget: { dailyUsd: 5, paneUsd: -1, interrupt: "yes" },
      phoneUrl: " https://ntfy.sh/t ",
    });
    expect(s.autoAllow).toEqual([
      { rule: "Bash(ls)", folder: "D:\\p" },
      { rule: "Edit", folder: "" },
    ]);
    expect(s.budget).toEqual({ dailyUsd: 5, paneUsd: 0, interrupt: false });
    expect(s.phoneUrl).toBe("https://ntfy.sh/t");
  });

  it("clamps and rounds the font size", () => {
    expect(mergeSettings({ fontSize: 2 }).fontSize).toBe(8);
    expect(mergeSettings({ fontSize: 14.6 }).fontSize).toBe(15);
    expect(mergeSettings({ fontSize: Number.NaN }).fontSize).toBe(DEFAULT_SETTINGS.fontSize);
  });
});
