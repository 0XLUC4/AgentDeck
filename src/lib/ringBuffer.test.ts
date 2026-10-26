import { describe, expect, it } from "vitest";
import { RingBuffer } from "./ringBuffer";

describe("RingBuffer", () => {
  it("keeps everything under the cap", () => {
    const buf = new RingBuffer(10);
    buf.push("abc");
    buf.push("def");
    expect(buf.text()).toBe("abcdef");
  });
  it("drops oldest chunks past the cap", () => {
    const buf = new RingBuffer(5);
    buf.push("abc");
    buf.push("de");
    buf.push("fg");
    expect(buf.text()).toBe("defg");
  });
  it("keeps the tail of an oversized chunk", () => {
    const buf = new RingBuffer(3);
    buf.push("abcdef");
    expect(buf.text()).toBe("def");
    buf.push("g");
    expect(buf.text()).toBe("g"); // whole chunks are dropped
  });
  it("clears", () => {
    const buf = new RingBuffer(3);
    buf.push("ab");
    buf.clear();
    expect(buf.text()).toBe("");
  });
});
