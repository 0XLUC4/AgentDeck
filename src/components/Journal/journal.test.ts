import { describe, expect, it } from "vitest";
import { groupJournal, type JournalEntry, journalMarkdown, parseEntries, toJournalEntry } from "./journal";

const at = (h: number, m = 0) => new Date(2026, 9, 26, h, m).getTime();

const entry = (over: Partial<JournalEntry>): JournalEntry => ({
  paneId: "p1",
  title: "Claude Code",
  agent: "claude",
  cwd: "D:\\proj",
  workspaceName: "proj",
  workspacePath: "D:\\proj",
  prompt: "Fix the bug",
  summary: "Fixed.",
  actions: ["Edit a.ts"],
  startedAt: at(10),
  endedAt: at(10, 12),
  status: "done",
  ...over,
});

describe("journal grouping", () => {
  it("groups by workspace then agent, in agent order, tasks by start", () => {
    const groups = groupJournal([
      entry({ agent: "codex", startedAt: at(11), endedAt: at(11, 5), prompt: "B" }),
      entry({ startedAt: at(9), endedAt: at(9, 30), prompt: "A" }),
      entry({ workspacePath: "D:/proj/", prompt: "C", startedAt: at(12), endedAt: at(12, 1) }),
      entry({ workspaceName: "", workspacePath: "", cwd: "C:\\tmp\\scratch", prompt: "D" }),
    ]);
    expect(groups.map((g) => g.name)).toEqual(["proj", "scratch"]);
    expect(groups[0].agents.map((a) => a.agent)).toEqual(["claude", "codex"]);
    expect(groups[0].agents[0].tasks.map((t) => t.prompt)).toEqual(["A", "C"]);
  });

  it("keeps workspaces that only have commits", () => {
    const commit = { hash: "abc1234", time: "2026-10-26T14:05:00+01:00", author: "L", subject: "feat: x" };
    const groups = groupJournal([], [{ name: "other", path: "E:\\other", commits: [commit] }]);
    expect(groups).toHaveLength(1);
    expect(groups[0].commits).toHaveLength(1);
  });

  it("drops malformed day-file entries", () => {
    expect(parseEntries([entry({}), { foo: 1 }, null, entry({ agent: "nope" as never })])).toHaveLength(1);
  });

  it("builds an entry from a pane task", () => {
    const e = toJournalEntry({ id: "x", title: "T", agent: "codex", cwd: "C:\\a" }, undefined, {
      prompt: "p",
      startedAt: 1,
      actions: [{ at: 1, text: "Run" }],
      summary: null,
      endedAt: 5,
      status: "done",
    });
    expect(e).toMatchObject({ paneId: "x", workspacePath: "", actions: ["Run"], endedAt: 5 });
  });
});

describe("journal markdown", () => {
  it("renders headings, tasks, interrupted status, commits and usage", () => {
    const groups = groupJournal(
      [entry({}), entry({ status: "working", prompt: "multi\nline   prompt", summary: null, actions: [] })],
      [
        {
          name: "proj",
          path: "D:\\proj",
          commits: [{ hash: "abc1234", time: new Date(at(15)).toISOString(), author: "L", subject: "fix: y" }],
        },
      ],
    );
    const md = journalMarkdown("2026-10-26", groups, {
      total: { input: 0, output: 0, cacheWrite: 0, cacheRead: 0, tokens: 12_000, cost: 0.31, unpricedTokens: 0 },
      byAgent: [{ key: "claude", tokens: 12_000, cost: 0.31 }],
    });
    expect(md).toContain("# Journal for 10/26/2026");
    expect(md).toContain("**Usage**: 12k tokens · $0.31 (Claude Code $0.31)");
    expect(md).toContain("## proj");
    expect(md).toContain("### Claude Code - 2 task(s)");
    expect(md).toContain("- 10:00 → 10:12 (12 min) - Fix the bug");
    expect(md).toContain("  - Result: Fixed.");
    expect(md).toContain("[interrupted] - multi line prompt");
    expect(md).toContain("### Commits - 1");
    expect(md).toContain("- `abc1234` 15:00 fix: y");
    expect(md.endsWith("\n")).toBe(true);
  });

  it("says when the day is empty", () => {
    expect(journalMarkdown("2026-10-26", [], null)).toContain("No tasks");
  });
});
