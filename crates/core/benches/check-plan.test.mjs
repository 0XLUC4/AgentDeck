import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const checker = fileURLToPath(new URL("../plugins/plan-first/agentdeck-plan-first/scripts/check-plan.mjs", import.meta.url));

function check(markdown) {
  const dir = mkdtempSync(join(tmpdir(), "agentdeck-plan-"));
  try {
    writeFileSync(join(dir, "README.md"), markdown);
    return spawnSync(process.execPath, [checker, dir], { encoding: "utf8" });
  } finally {
    rmSync(dir, { recursive: true });
  }
}

test("linked inline-code headings, including duplicates", () => {
  const result = check("# Index\n[One](#run-build) [Two](#run-build-1)\n## Run `build`\n## Run `build`\n");
  assert.equal(result.status, 0, result.stdout);
});

test("example anchors cannot satisfy real links", () => {
  const result = check('# Index\n[Go](#sample)\n~~~html\n<a id="sample"></a>\n~~~\n');
  assert.equal(result.status, 1, result.stdout);
  assert.match(result.stdout, /no heading for anchor/);
});

test("documented todo status is valid while an unfinished instruction is reported", () => {
  const result = check("# Index\n| S1 | todo |\n**Build** — status: todo\nTODO: choose a storage engine\n");
  assert.equal(result.status, 0);
  assert.match(result.stdout, /README\.md:4 vague/);
  assert.match(result.stdout, /0 errors, 1 warnings/);
});
