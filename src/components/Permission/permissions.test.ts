import { describe, expect, it } from "vitest";
import type { PermissionRequest } from "../../lib/types";
import { addPermission, type PendingPermissions, permissionsForPane, removePermission } from "./permissions";

const req = (requestId: string, paneId = "p1"): PermissionRequest => ({
  requestId,
  paneId,
  agent: "claude",
  toolName: "Bash",
  summary: "Bash: rm -rf build",
  detail: "rm -rf build",
  suggestions: [],
});

describe("pending permissions reducer", () => {
  it("adds, lists per pane in arrival order and removes", () => {
    let s: PendingPermissions = {};
    s = addPermission(s, req("a"));
    s = addPermission(s, req("b", "p2"));
    s = addPermission(s, req("c"));
    expect(permissionsForPane(s, "p1").map((r) => r.requestId)).toEqual(["a", "c"]);
    s = removePermission(s, "a");
    expect(Object.keys(s)).toEqual(["b", "c"]);
  });

  it("removing an unknown id keeps the same object", () => {
    const s = addPermission({}, req("a"));
    expect(removePermission(s, "zzz")).toBe(s);
  });

  it("re-adding the same id replaces it", () => {
    const s = addPermission(addPermission({}, req("a")), { ...req("a"), summary: "Edit x" });
    expect(Object.values(s)).toHaveLength(1);
    expect(s.a.summary).toBe("Edit x");
  });
});
