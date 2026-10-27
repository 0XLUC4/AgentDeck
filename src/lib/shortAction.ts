// Few-word version of a pane's last action for the header ("Editing deck.ts", "Running pnpm test").
// The full text (from hooks_server::describe_tool) stays in the tooltip and the task log.

const base = (path: string) => path.split(/[/]/).filter(Boolean).pop() ?? path;
const words = (text: string, n: number) => text.split(/\s+/).filter(Boolean).slice(0, n).join(" ");

const VERBS: Partial<Record<string, (rest: string) => string>> = {
  Read: (rest) => `Reading ${base(rest)}`,
  Edit: (rest) => `Editing ${base(rest)}`,
  MultiEdit: (rest) => `Editing ${base(rest)}`,
  Write: (rest) => `Writing ${base(rest)}`,
  NotebookEdit: (rest) => `Editing ${base(rest)}`,
  Grep: (rest) => `Searching ${words(rest, 3)}`,
  Glob: (rest) => `Finding ${words(rest, 3)}`,
  WebFetch: (rest) => `Opening ${rest.replace(/^https?:\/\//, "").split("/")[0]}`,
  WebSearch: (rest) => `Searching web: ${words(rest, 3)}`,
};

export function shortAction(action: string | null): string {
  if (!action) return "";
  if (action.startsWith("Bash: ")) return `Running ${words(action.slice(6), 3)}`;
  const space = action.indexOf(" ");
  const tool = space < 0 ? action : action.slice(0, space);
  const rest = space < 0 ? "" : action.slice(space + 1);
  // mcp__server__tool_name -> "tool name"
  const mcp = /^mcp__[^_]+(?:_[^_]+)*?__(.+)$/.exec(tool);
  if (mcp) return mcp[1].replace(/_/g, " ");
  const verb = VERBS[tool];
  return verb && rest ? verb(rest) : words(action, 4);
}
