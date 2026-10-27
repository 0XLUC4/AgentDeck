import { describe, expect, it } from "vitest";
import { autoAllowed, insideFolder, ruleFor, ruleMatches } from "./autoAllow";

const bash = (detail: string) => ({ toolName: "Bash", detail });

describe("auto-allow rules", () => {
  it("matches exact and prefix shell rules", () => {
    expect(ruleMatches("Bash(pnpm test)", bash("pnpm test"))).toBe(true);
    expect(ruleMatches("Bash(pnpm test)", bash("pnpm test --watch"))).toBe(false);
    expect(ruleMatches("Bash(pnpm test *)", bash("pnpm test --watch"))).toBe(true);
    expect(ruleMatches("Bash(pnpm test *)", bash("pnpm test"))).toBe(true);
    expect(ruleMatches("Bash(pnpm test *)", bash("pnpm testx"))).toBe(false);
  });

  it("never approves chained or substituted commands", () => {
    for (const cmd of [
      "pnpm test && rm -rf ~",
      "pnpm test; rm x",
      "pnpm test | sh",
      "pnpm test $(evil)",
      "pnpm test\nrm x",
    ]) {
      expect(ruleMatches("Bash(pnpm test *)", bash(cmd))).toBe(false);
    }
    expect(ruleMatches("Bash", bash("ls"))).toBe(false);
  });

  it("matches other tools by name only", () => {
    expect(ruleMatches("Read", { toolName: "Read", detail: "{}" })).toBe(true);
    expect(ruleMatches("Read(x)", { toolName: "Read", detail: "{}" })).toBe(false);
    expect(ruleMatches("Edit", { toolName: "Read", detail: "{}" })).toBe(false);
  });

  it("scopes rules to a folder", () => {
    expect(insideFolder("D:\\proj\\sub", "d:/proj")).toBe(true);
    expect(insideFolder("D:\\project", "D:\\proj")).toBe(false);
    const rules = [
      { rule: "Read", folder: "D:\\other" },
      { rule: "Bash(ls)", folder: "" },
    ];
    expect(autoAllowed({ toolName: "Read", detail: "" }, "D:\\proj", rules)).toBeUndefined();
    expect(autoAllowed(bash("ls"), "D:\\proj", rules)?.rule).toBe("Bash(ls)");
  });

  it("builds a rule from a request", () => {
    expect(ruleFor(bash("pnpm lint"))).toBe("Bash(pnpm lint)");
    expect(ruleFor(bash("a && b"))).toBeNull();
    expect(ruleFor({ toolName: "Edit", detail: "{}" })).toBe("Edit");
  });
});
