import { describe, expect, it } from "vitest";
import { splitArgs } from "./splitArgs";

describe("splitArgs", () => {
  it("splits on whitespace", () => expect(splitArgs("  --model  sonnet -v ")).toEqual(["--model", "sonnet", "-v"]));
  it("groups quoted words", () =>
    expect(splitArgs(`-p "hello world" 'a b' ""`)).toEqual(["-p", "hello world", "a b", ""]));
  it("returns nothing for blank input", () => expect(splitArgs("   ")).toEqual([]));
});
