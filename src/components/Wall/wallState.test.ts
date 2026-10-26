import { describe, expect, it } from "vitest";
import type { PaneStatus } from "../../lib/types";
import { newPane, newTab, newWorkspace, type Pane } from "../../store/deck";
import { lastLines, stripAnsi, wallItems } from "./wallState";

describe("stripAnsi", () => {
  it("removes colors, cursor moves, OSC titles and charset switches", () => {
    expect(stripAnsi("\x1b[1;31mred\x1b[0m \x1b[2K\x1b[1Aup\x1b]0;title\x07 \x1b(Bok \x1b=")).toBe("red up ok ");
  });
});

describe("lastLines", () => {
  it("keeps the last non-blank lines", () => {
    const raw = Array.from({ length: 10 }, (_, i) => `line ${i}`).join("\r\n");
    expect(lastLines(`${raw}\r\n\r\n`, 3)).toEqual(["line 7", "line 8", "line 9"]);
  });
  it("lets a carriage return overwrite the line and drops control chars", () => {
    expect(lastLines("Loading 10%\rLoading 100%\r\nok\x07\x08 \n")).toEqual(["Loading 100%", "ok"]);
  });
});

describe("wallItems", () => {
  const ws = newWorkspace("C:\\a");
  const other = newWorkspace("C:\\b");
  const tab = newTab("S1", ws.id);
  const tab2 = newTab("S2", other.id);
  const pane = (id: string, status: PaneStatus, since: number, exited = false): Pane => ({
    ...newPane({ id, agent: "shell", title: id, cwd: "", args: [], worktree: false, completedCount: 0, fontSize: 13 }),
    status,
    statusSince: since,
    exited,
  });
  const panes = Object.fromEntries(
    [
      pane("idle", "idle", 1),
      pane("working", "working", 1),
      pane("stopped", "idle", 1, true),
      pane("waitNew", "waiting", 9),
      pane("waitOld", "waiting", 2),
      pane("error", "error", 1),
      pane("done", "done", 1),
      pane("elsewhere", "waiting", 0),
    ].map((p) => [p.id, p]),
  );
  tab.paneIds = ["idle", "working", "stopped", "waitNew", "waitOld", "error", "done"];
  tab2.paneIds = ["elsewhere"];

  it("sorts by priority then by time in status, for one workspace", () => {
    const ids = wallItems([tab, tab2], [ws, other], panes, ws.id).map((i) => i.pane.id);
    expect(ids).toEqual(["waitOld", "waitNew", "error", "done", "working", "idle", "stopped"]);
  });
  it("includes every workspace when asked", () => {
    expect(wallItems([tab, tab2], [ws, other], panes, null)[0].pane.id).toBe("elsewhere");
  });
});
