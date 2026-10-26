/** Splits a CLI argument string on whitespace; "double" or 'single' quotes group words. */
export function splitArgs(input: string): string[] {
  const matches = input.matchAll(/"([^"]*)"|'([^']*)'|(\S+)/g);
  return Array.from(matches, (m) => m[1] ?? m[2] ?? m[3] ?? "");
}
