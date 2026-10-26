import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-store", () => ({ load: vi.fn(() => Promise.reject(new Error("no tauri"))) }));
vi.mock("../lib/ipc", () => ({}));

const { BUILTIN_TEMPLATES, instantiate, joinPath, parseTemplates, relativePath, sessionToTemplate } = await import(
  "./templates"
);
const { newPane } = await import("./deck");
const { DEFAULT_SETTINGS } = await import("./settings");

const spec = (id: string, cwd: string, agent: "claude" | "codex" = "claude") =>
  newPane({ id, agent, title: id, cwd, args: ["--x"], worktree: true, completedCount: 4, fontSize: 15 });

describe("template paths", () => {
  it("keeps only paths inside the workspace, relative", () => {
    expect(relativePath("D:\\proj\\web\\app", "D:\\proj")).toBe("web/app");
    expect(relativePath("d:/PROJ/", "D:\\proj")).toBe("");
    expect(relativePath("D:\\project2", "D:\\proj")).toBeNull();
    expect(relativePath("C:\\other", "D:\\proj")).toBeNull();
    expect(relativePath("D:\\proj", "")).toBeNull();
  });

  it("joins with the workspace's separator", () => {
    expect(joinPath("D:\\ws\\", "web/app")).toBe("D:\\ws\\web\\app");
    expect(joinPath("/home/me/ws", "api")).toBe("/home/me/ws/api");
    expect(joinPath("D:\\ws", "")).toBe("D:\\ws");
  });
});

describe("template serialization", () => {
  const layout = { grid: { root: { data: [{ views: ["a"] }, { views: ["b"] }] } }, panels: { a: { id: "a" }, b: {} } };
  const panes = { a: spec("a", "D:\\proj\\web"), b: spec("b", "C:\\elsewhere", "codex") };
  const template = sessionToTemplate(
    "My template",
    { paneIds: ["a", "b", "gone"], layout: layout as never },
    panes,
    "D:\\proj",
  );

  it("stores relative folders and no absolute path outside the workspace", () => {
    expect(template.panes).toEqual([
      { key: "a", agent: "claude", title: "a", cwd: "web", args: ["--x"], worktree: true, fontSize: 15 },
      { key: "b", agent: "codex", title: "b", cwd: null, args: ["--x"], worktree: true, fontSize: 15 },
    ]);
    expect(JSON.stringify(template)).not.toContain("elsewhere");
    expect(parseTemplates(JSON.parse(JSON.stringify([template, { bogus: 1 }])))).toEqual([template]);
  });

  it("instantiates in another workspace with fresh ids and a remapped layout", () => {
    const { layout: out, panes: specs } = instantiate(template, "E:\\other", "", DEFAULT_SETTINGS);
    expect(specs.map((s) => s.cwd)).toEqual(["E:\\other\\web", "E:\\other"]);
    expect(specs.every((s) => s.id !== "a" && s.id !== "b" && s.completedCount === 0)).toBe(true);
    const json = JSON.stringify(out);
    expect(json).toContain(specs[0].id);
    expect(json).toContain(specs[1].id);
    expect(json).not.toContain('"a"');
  });

  it("uses the fallback folder and agent defaults for builtins", () => {
    const defaults = {
      ...DEFAULT_SETTINGS,
      agents: { ...DEFAULT_SETTINGS.agents, claude: { args: "--model opus", worktree: true } },
    };
    const { layout, panes: specs } = instantiate(BUILTIN_TEMPLATES[0], "", "D:\\last", defaults);
    expect(layout).toBeNull();
    expect(specs.map((s) => [s.title, s.cwd, s.args, s.worktree])).toEqual([
      ["Front", "D:\\last", ["--model", "opus"], true],
      ["Back", "D:\\last", ["--model", "opus"], true],
      ["Tests", "D:\\last", ["--model", "opus"], true],
    ]);
  });
});
