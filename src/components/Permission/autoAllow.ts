// Permission rules answered by AgentDeck without asking: "Read", "Bash(pnpm test)", "Bash(pnpm test *)".
// Shell commands only match when they are a single plain command: a rule for `pnpm test` must never
// approve `pnpm test && rm -rf ~`.
import type { PermissionRequest } from "../../lib/types";
import type { AutoAllowRule } from "../../store/settings";

const SHELL_TOOLS = new Set(["Bash", "PowerShell"]);
/** Chaining, substitution, redirection, newlines: anything that could run more than the approved command. */
const SHELL_META = /[;&|`$<>\r\n(){}]/;

const normalize = (path: string) =>
  path
    .replace(/[\\/]+/g, "/")
    .replace(/\/$/, "")
    .toLowerCase();

export function insideFolder(cwd: string, folder: string): boolean {
  if (!folder) return true;
  const base = normalize(folder);
  const path = normalize(cwd);
  return path === base || path.startsWith(`${base}/`);
}

/** Splits "Bash(pnpm test)" into tool + content; a bare "Read" has no content. Null when malformed. */
export function parseRule(rule: string): { tool: string; content: string | null } | null {
  const m = /^([\w-]+)(?:\((.+)\))?$/.exec(rule.trim());
  return m ? { tool: m[1], content: m[2]?.trim() ?? null } : null;
}

/** True when `rule` covers this request. Shell rules need content; other tools match by name only. */
export function ruleMatches(rule: string, req: Pick<PermissionRequest, "toolName" | "detail">): boolean {
  const parsed = parseRule(rule);
  if (!parsed || parsed.tool !== req.toolName) return false;
  if (!SHELL_TOOLS.has(req.toolName)) return parsed.content === null;
  if (parsed.content === null) return false; // never "any shell command"
  const command = req.detail.trim();
  if (SHELL_META.test(command)) return false;
  const { content } = parsed;
  if (content.endsWith(" *")) {
    const prefix = content.slice(0, -2).trim();
    return command === prefix || command.startsWith(`${prefix} `);
  }
  return command === content;
}

export const autoAllowed = (
  req: Pick<PermissionRequest, "toolName" | "detail">,
  cwd: string,
  rules: AutoAllowRule[],
): AutoAllowRule | undefined => rules.find((r) => insideFolder(cwd, r.folder) && ruleMatches(r.rule, req));

/** Rule the "Auto-allow here" button adds: the exact shell command, or the tool name. Null if not allowed. */
export function ruleFor(req: Pick<PermissionRequest, "toolName" | "detail">): string | null {
  if (!SHELL_TOOLS.has(req.toolName)) return req.toolName;
  const command = req.detail.trim();
  return command && !SHELL_META.test(command) ? `${req.toolName}(${command})` : null;
}
