"""Run from the repository root: python crates/core/benches/plugin_catalog.py BASE_REV

Requires tiktoken. Counts description text independently using o200k_base.
The retrieval check measures vocabulary coverage, not model behavior.
"""

import argparse
import math
import re
import subprocess
from collections import Counter
from pathlib import Path

import tiktoken


QUERIES = {
    "3d": ("Render a Blender scene", "Export a model as glTF"),
    "backend": ("Make a webhook idempotent", "Migrate a database schema"),
    "debug": ("Trace a panic", "Investigate a flaky test"),
    "engineering": ("Keep this refactor minimal", "Self-review a production change"),
    "form-flow": ("Test checkout with a double submit", "Test a multi-step login flow"),
    "frontend": ("Build a React component", "Style a loading state"),
    "git": ("Resolve a merge conflict", "Rebase my branch"),
    "mobile-check": ("Measure tap targets on a phone", "Check responsive layout overflow"),
    "plan-first": ("Plan a new project", "Split a migration between agents"),
    "profile": ("Profile CPU and memory", "Benchmark slow startup"),
    "project-memory": ("Update AGENTS.md", "Onboard a contributor to an unfamiliar repository"),
    "review": ("Review a pull request for defects", "Inspect another agent's diff"),
    "security": ("Audit authorization for IDOR", "Check for SSRF"),
    "speech-qa": ("Measure dictation latency", "Compare transcription accuracy"),
    "terse": ("Use fewer tokens", "Give me a TL;DR"),
    "tests-first": ("Use TDD for this bug", "Write a failing test first"),
    "upgrade": ("Upgrade our framework version", "Update vulnerable dependencies"),
    "web-check": ("Inspect failed requests on the page", "Check browser console errors"),
    "web-perf": ("Trace LCP and CLS", "Investigate janky scrolling"),
    "web-research": ("Find current prices online", "Compare primary sources for recent news"),
}


# Indirect requests exercise wording beyond the direct prompts. Both sets are regression fixtures.
PARAPHRASES = {
    "3d": ("Why does my mesh render black in Godot?",),
    "backend": ("Handle duplicate deliveries from the payment provider's webhook",),
    "debug": ("Yesterday's change made this test hang intermittently",),
    "engineering": ("Simplify this feature without touching unrelated files",),
    "form-flow": ("Exercise checkout offline and verify the saved order",),
    "frontend": ("Design an empty state for a Vue screen",),
    "git": ("Undo my last commit while keeping local edits",),
    "mobile-check": ("On phones the submit button is too small to tap",),
    "plan-first": ("Map the steps and dependencies for a cross-team migration",),
    "profile": ("Which function is consuming the most CPU?",),
    "project-memory": ("Record the build command for the next agent in CLAUDE.md",),
    "review": ("Identify regressions introduced by this diff",),
    "security": ("Can one account read another user's files?",),
    "speech-qa": ("Did changing the microphone lose words at the end?",),
    "terse": ("Be brief and keep the exact error",),
    "tests-first": ("Demonstrate the bug with a red test before patching it",),
    "upgrade": ("Move our runtime to the next major version",),
    "web-check": ("Open the changed page and check the console",),
    "web-perf": ("What caused the poor Largest Contentful Paint?",),
    "web-research": ("Look up whether those API docs are still current",),
}


def terms(text):
    stopwords = {"a", "an", "and", "are", "as", "at", "by", "for", "from", "in",
                 "is", "it", "of", "on", "or", "s", "the", "to", "with"}
    return set(re.findall(r"[a-z0-9]+", text.lower())) - stopwords


def retrieval(descriptions, fixtures):
    docs = {key: terms(key + " " + value) for key, value in descriptions.items()}
    frequency = Counter(word for words in docs.values() for word in words)
    misses = []
    for expected, queries in fixtures.items():
        for query in queries:
            wanted = terms(query)
            scores = {
                key: sum(math.log((1 + len(docs)) / frequency[w]) for w in sorted(wanted & words))
                for key, words in docs.items()
            }
            winners = [key for key in docs if scores[key] == max(scores.values())]
            if winners != [expected]:
                misses.append(f"{query} -> {', '.join(winners)} (expected {expected})")
    return misses


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", help="Git revision containing the comparison catalog")
    base = parser.parse_args().base
    encoder = tiktoken.get_encoding("o200k_base")
    catalogs = [{}, {}]
    totals = [0, 0]
    print("skill                     description tokens    full SKILL.md tokens")
    for path in sorted(Path("crates/core/plugins").glob("*/*/SKILL.md")):
        key = path.parent.name.removeprefix("agentdeck-")
        original = subprocess.check_output(["git", "show", f"{base}:{path.as_posix()}"])
        counts, full = [], []
        for i, data in enumerate([original, path.read_bytes()]):
            body = data.decode("utf-8").replace("\r\n", "\n")
            description = next(s.removeprefix("description: ") for s in body.splitlines()
                               if s.startswith("description: "))
            catalogs[i][key] = description
            counts.append(len(encoder.encode(description)))
            full.append(len(encoder.encode(body)))
            totals[i] += counts[-1]
        print(f"{key:24} {counts[0]:4} -> {counts[1]:3}          {full[0]:5} -> {full[1]:4}")
    print(f"Description totals: {totals[0]} -> {totals[1]}")
    for suite, cases in [("direct", QUERIES), ("paraphrases", PARAPHRASES)]:
        if set(catalogs[1]) != set(cases):
            raise SystemExit(f"Update {suite} fixtures to cover the catalog")
        n = sum(map(len, cases.values()))
        for label, catalog in zip(["base", "working tree"], catalogs):
            misses = retrieval(catalog, cases)
            print(f"{suite}, {label}: {n - len(misses)}/{n} unique first-place matches")
            for miss in misses:
                print(f"  {miss}")


if __name__ == "__main__":
    main()
