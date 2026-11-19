import { readFileSync } from "node:fs";

function words(text) {
  return text.normalize("NFC").toLowerCase().match(/[\p{L}\p{N}]+/gu) ?? [];
}

function edits(reference, actual) {
  // Matching outer words contribute no edits. Score only the differing span.
  let start = 0;
  let refEnd = reference.length;
  let actualEnd = actual.length;
  while (start < refEnd && start < actualEnd && reference[start] === actual[start]) start++;
  while (refEnd > start && actualEnd > start && reference[refEnd - 1] === actual[actualEnd - 1]) {
    refEnd--;
    actualEnd--;
  }
  if (start === refEnd) return actualEnd - start;
  if (start === actualEnd) return refEnd - start;
  reference = reference.slice(start, refEnd);
  actual = actual.slice(start, actualEnd);
  let prev = Array.from({ length: actual.length + 1 }, (_, i) => i);
  for (const [i, word] of reference.entries()) {
    const next = [i + 1];
    for (const [j, heard] of actual.entries()) {
      next.push(Math.min(prev[j + 1] + 1, next[j] + 1, prev[j] + Number(word !== heard)));
    }
    prev = next;
  }
  return prev[actual.length];
}

function score(path) {
  const times = [];
  const perClip = new Map();
  let errors = 0;
  let totalWords = 0;
  for (const [i, line] of readFileSync(path, "utf8").split(/\r?\n/).entries()) {
    if (!line.trim()) continue;
    let row;
    try {
      row = JSON.parse(line);
    } catch {
      throw new Error(`invalid JSON on line ${i + 1}`);
    }
    if (!row || typeof row.clip !== "string" || !row.clip.trim() ||
        typeof row.expected !== "string" || typeof row.actual !== "string" ||
        !Number.isFinite(row.latencyMs) || row.latencyMs < 0) {
      throw new Error(`invalid clip, transcript or latency on line ${i + 1}`);
    }
    const reference = words(row.expected);
    const distance = edits(reference, words(row.actual));
    errors += distance;
    totalWords += reference.length;
    times.push(row.latencyMs);
    if (distance) perClip.set(row.clip, (perClip.get(row.clip) ?? 0) + distance);
  }
  if (!times.length) throw new Error("no measurements");
  times.sort((a, b) => a - b);
  const mid = Math.floor(times.length / 2);
  return {
    runs: times.length, wordEdits: errors, referenceWords: totalWords,
    wordErrorRate: totalWords ? errors / totalWords : null,
    medianMs: (times[mid] + times[Math.floor((times.length - 1) / 2)]) / 2,
    p90Ms: times[Math.ceil(times.length * 0.9) - 1],
    editsByClip: Object.fromEntries(perClip),
  };
}

try {
  if (process.argv.length !== 3) throw new Error("usage: node score.mjs results.jsonl");
  console.log(JSON.stringify(score(process.argv[2]), null, 2));
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
