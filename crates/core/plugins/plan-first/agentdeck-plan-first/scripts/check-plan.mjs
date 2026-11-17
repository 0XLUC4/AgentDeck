#!/usr/bin/env node
// Checks a linked plan folder: broken links and anchors, files the index does not link, files with no
// way back to the index, and placeholders left over. Usage: node check-plan.mjs <plan folder>
// Exit code 1 when there are errors; warnings alone exit 0.
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";

const root = resolve(process.argv[2] ?? ".");
if (!existsSync(root) || !statSync(root).isDirectory()) {
  console.error(`not a folder: ${root}`);
  process.exit(2);
}

/** Every .md file under `dir`, as absolute paths. */
function markdownFiles(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name.startsWith(".") ? [] : markdownFiles(path);
    return entry.isFile() && entry.name.endsWith(".md") ? [path] : [];
  });
}

/** Text without fenced code blocks and inline code (links and placeholders in code are examples). */
function prose(text) {
  return text.replace(/^(```|~~~)[^\n]*\n[\s\S]*?^\1[^\n]*$/gm, (block) => block.replace(/[^\n]/g, "")).replace(/`[^`\n]*`/g, "");
}

/** GitHub heading anchor: lower case, punctuation dropped, spaces to dashes. */
function slug(heading) {
  return heading
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, "")
    .replace(/\s/g, "-");
}

function anchors(text) {
  const found = new Set();
  const seen = new Map();
  for (const [, title] of prose(text).matchAll(/^#{1,6}\s+(.+?)\s*#*\s*$/gm)) {
    const base = slug(title);
    const count = seen.get(base) ?? 0;
    seen.set(base, count + 1);
    found.add(count ? `${base}-${count}` : base);
  }
  for (const [, id] of text.matchAll(/<a\s+(?:id|name)="([^"]+)"/g)) found.add(id);
  return found;
}

const HTML_TAGS = new Set(["a", "b", "i", "br", "kbd", "sub", "sup", "details", "summary", "img", "p", "div", "span"]);
const VAGUE = /\b(TBD|TODO|FIXME|handle edge cases|add validation|update as needed|as appropriate|etc\.)/i;

const files = markdownFiles(root);
const texts = new Map(files.map((f) => [f, readFileSync(f, "utf8")]));
const headings = new Map();
const index = join(root, "README.md");
const incoming = new Map(files.map((f) => [f, new Set()]));
const errors = [];
const warnings = [];
const at = (file, line) => `${relative(root, file).replaceAll("\\", "/")}:${line}`;

if (!texts.has(index)) errors.push("README.md (the index) is missing");

for (const [file, text] of texts) {
  const lines = prose(text).split("\n");
  lines.forEach((line, i) => {
    for (const [, target] of line.matchAll(/\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g)) {
      if (/^[a-z][a-z0-9+.-]*:/i.test(target)) continue; // http:, mailto:...
      const [path, anchor] = target.split("#");
      let linked;
      let fragment;
      try {
        linked = path ? resolve(dirname(file), decodeURIComponent(path)) : file;
        fragment = decodeURIComponent(anchor ?? "");
      } catch {
        errors.push(`${at(file, i + 1)} invalid URL encoding: ${target}`);
        continue;
      }
      if (!existsSync(linked)) {
        errors.push(`${at(file, i + 1)} broken link: ${target}`);
        continue;
      }
      if (linked !== file) incoming.get(linked)?.add(file);
      if (fragment && texts.has(linked)) {
        if (!headings.has(linked)) headings.set(linked, anchors(texts.get(linked)));
        if (!headings.get(linked).has(fragment)) errors.push(`${at(file, i + 1)} no heading for anchor: ${target}`);
      }
    }
    for (const [placeholder, name] of line.matchAll(/<([a-z][a-z0-9 ,._/|-]*)>/gi)) {
      if (!HTML_TAGS.has(name.toLowerCase())) errors.push(`${at(file, i + 1)} placeholder left: ${placeholder}`);
    }
    const vague = line.match(VAGUE);
    if (vague) warnings.push(`${at(file, i + 1)} vague: "${vague[0]}", write the concrete thing`);
  });
}

for (const file of files) {
  if (file === index) continue;
  const from = incoming.get(file);
  if (!from.has(index)) errors.push(`${at(file, 1)} not linked from README.md`);
  if ([...from].filter((f) => f !== index).length === 0) warnings.push(`${at(file, 1)} no other plan file links here`);
  const head = texts.get(file).split("\n").slice(0, 3).join("\n");
  if (!/\]\((\.\.\/)*README\.md\)/.test(head)) warnings.push(`${at(file, 1)} no breadcrumb back to README.md`);
}

for (const e of errors) console.log(`error   ${e}`);
for (const w of warnings) console.log(`warning ${w}`);
console.log(`${files.length} files, ${errors.length} errors, ${warnings.length} warnings`);
process.exit(errors.length ? 1 : 0);
