import { describe, expect, it } from "vitest";
import { formatShortcut, toTauriShortcut } from "./shortcut";

const press = (code: string, mods: Partial<Record<"ctrlKey" | "shiftKey" | "altKey" | "metaKey", boolean>> = {}) => ({
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  metaKey: false,
  code,
  ...mods,
});

describe("toTauriShortcut", () => {
  it("converts common combos", () => {
    expect(toTauriShortcut(press("Space", { ctrlKey: true }))).toBe("ctrl+space");
    expect(toTauriShortcut(press("F9", { ctrlKey: true, shiftKey: true }))).toBe("ctrl+shift+f9");
    expect(toTauriShortcut(press("KeyA", { altKey: true, metaKey: true }))).toBe("alt+super+a");
    expect(toTauriShortcut(press("Digit1", { ctrlKey: true }))).toBe("ctrl+1");
    expect(toTauriShortcut(press("F13"))).toBe("f13");
  });

  it("orders modifiers consistently", () => {
    expect(toTauriShortcut(press("KeyK", { shiftKey: true, ctrlKey: true, altKey: true }))).toBe("ctrl+alt+shift+k");
  });

  it("ignores modifier-only presses", () => {
    expect(toTauriShortcut(press("ControlLeft", { ctrlKey: true }))).toBeNull();
    expect(toTauriShortcut(press("ShiftRight", { shiftKey: true }))).toBeNull();
    expect(toTauriShortcut(press("MetaLeft"))).toBeNull();
    expect(toTauriShortcut(press(""))).toBeNull();
  });
});

describe("formatShortcut", () => {
  it("renders readable labels", () => {
    expect(formatShortcut("ctrl+space")).toBe("Ctrl + Space");
    expect(formatShortcut("ctrl+shift+f9")).toBe("Ctrl + Shift + F9");
    expect(formatShortcut("CommandOrControl+Shift+A")).toBe("Ctrl + Shift + A");
  });
});
