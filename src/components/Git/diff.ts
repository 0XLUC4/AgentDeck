// Unified diff line classification for the Git panel's diff viewer.

export type DiffLineKind = "add" | "del" | "hunk" | "meta" | "context";

/** Kind of each line of `git diff` output. Inside a hunk, "--- x" is a deleted line, not a file header. */
export function classifyDiff(lines: string[]): DiffLineKind[] {
  let inHunk = false;
  return lines.map((line) => {
    if (line.startsWith("@@")) {
      inHunk = true;
      return "hunk";
    }
    if (line.startsWith("diff ")) inHunk = false;
    if (!inHunk) return "meta"; // diff --git, index, ---/+++ headers, Binary files ...
    if (line.startsWith("+")) return "add";
    if (line.startsWith("-")) return "del";
    if (line.startsWith("\\")) return "meta"; // \ No newline at end of file
    return "context";
  });
}

/** Lines of a diff, without the trailing empty line git's final newline produces. */
export const diffLines = (text: string) => (text ? text.replace(/\n$/, "").split("\n") : []);
