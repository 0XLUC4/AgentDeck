import { describe, expect, it } from "vitest";
import { applyAppearance, luminance, onAccent } from "./theme";

describe("accent contrast", () => {
  it("measures luminance", () => {
    expect(luminance("#000000")).toBe(0);
    expect(luminance("#ffffff")).toBeCloseTo(1);
  });
  it("picks readable text on the accent", () => {
    expect(onAccent("#e2b33c")).toBe("#141414"); // yellow: dark text
    expect(onAccent("#4f8ff7")).toBe("#141414");
    expect(onAccent("#7c3aed")).toBe("#ffffff"); // deep violet: white text
  });
});

describe("background tint", () => {
  const fakeRoot = () => {
    const props = new Map<string, string>();
    const style = {
      setProperty: (k: string, v: string) => props.set(k, v),
      removeProperty: (k: string) => props.delete(k),
    };
    return { root: { dataset: {}, style } as unknown as HTMLElement, props };
  };
  it("tints surfaces with the chosen hue at the theme's lightness", () => {
    const { root, props } = fakeRoot();
    applyAppearance({ theme: "dark", accent: "", background: "#2b4c8c" }, root);
    expect(props.get("--color-base")).toBe("oklch(from #2b4c8c 0.165 min(c, 0.045) h)");
    applyAppearance({ theme: "dark", accent: "", background: "" }, root);
    expect(props.has("--color-base")).toBe(false);
  });
});
