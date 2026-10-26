import { describe, expect, it } from "vitest";
import { classifyDiff, diffLines } from "./diff";

describe("classifyDiff", () => {
  it("classifies every kind of unified diff line", () => {
    const diff = [
      "diff --git a/a.txt b/a.txt",
      "index 5626abf..f719efd 100644",
      "--- a/a.txt",
      "+++ b/a.txt",
      "@@ -1,3 +1,3 @@",
      " same",
      "-old",
      "+new",
      "--- deleted line that started with two dashes",
      "+++ added line that started with two pluses",
      "\\ No newline at end of file",
      "diff --git a/b.png b/b.png",
      "--- a/b.png",
      "Binary files differ",
    ];
    expect(classifyDiff(diff)).toEqual([
      "meta",
      "meta",
      "meta",
      "meta",
      "hunk",
      "context",
      "del",
      "add",
      "del",
      "add",
      "meta",
      "meta",
      "meta",
      "meta",
    ]);
  });

  it("splits lines without the trailing newline", () => {
    expect(diffLines("a\nb\n")).toEqual(["a", "b"]);
    expect(diffLines("a\n\n")).toEqual(["a", ""]);
    expect(diffLines("")).toEqual([]);
  });
});
