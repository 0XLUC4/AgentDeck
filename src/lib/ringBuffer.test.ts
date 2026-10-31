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
  it("drops many chunks in order and reads the tail", () => {
    const buf = new RingBuffer(4);
    for (const c of "abcdefghij".repeat(300)) buf.push(c);
    expect(buf.text()).toBe("ghij");
    expect(buf.tail(2)).toBe("ij");
    expect(buf.tail(10)).toBe("ghij");
  });
  it("clears", () => {
    const buf = new RingBuffer(3);
    buf.push("ab");
    buf.clear();
    expect(buf.text()).toBe("");
  });
});
