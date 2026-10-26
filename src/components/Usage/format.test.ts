import { describe, expect, it } from "vitest";
import { formatCost, formatTokens, usageLabel } from "./format";

const nbsp = (s: string) => s.replace(/[  ]/g, " ");

describe("usage format", () => {
  it("compacts token counts", () => {
    expect(formatTokens(950)).toBe("950");
    expect(formatTokens(1234)).toBe("1,2k");
    expect(formatTokens(12_345)).toBe("12k");
    expect(formatTokens(1_234_567)).toBe("1,2M");
    expect(formatTokens(5_400_000_000)).toBe("5,4Md");
  });

  it("formats cost in French", () => {
    expect(formatCost(0.314)).toBe("0,31 $");
    expect(nbsp(formatCost(2061.04))).toBe("2 061,04 $");
  });

  it("omits cost when nothing is priced", () => {
    const base = { input: 0, output: 0, cacheWrite: 0, cacheRead: 0, unpricedTokens: 0 };
    expect(usageLabel({ ...base, tokens: 12_000, cost: 0.31 })).toBe("12k tok · 0,31 $");
    expect(usageLabel({ ...base, tokens: 500, cost: 0, unpricedTokens: 500 })).toBe("500 tok");
  });
});
