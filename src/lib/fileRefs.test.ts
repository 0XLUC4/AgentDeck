import { describe, expect, it } from "vitest";
import { fileRefs, imageExt } from "./fileRefs";

describe("fileRefs", () => {
  it("quotes paths with spaces for Claude", () => {
    expect(fileRefs("claude", ["C:\\a.png", "C:\\my pics\\b.png"])).toBe('C:\\a.png "C:\\my pics\\b.png" ');
  });
  it("uses @ and escaped spaces for Gemini", () => {
    expect(fileRefs("gemini", ["C:\\my pics\\b.png"])).toBe("@C:\\my\\ pics\\b.png ");
  });
  it("is empty without paths", () => {
    expect(fileRefs("codex", [])).toBe("");
  });
  it("maps image types only", () => {
    expect(imageExt("image/jpeg")).toBe("jpg");
    expect(imageExt("text/plain")).toBeUndefined();
  });
});
