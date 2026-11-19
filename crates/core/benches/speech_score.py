"""Compare the bundled dictation scorer with a Git revision on synthetic transcripts."""

import json
import random
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def main(base):
    script = Path("crates/core/plugins/speech-qa/agentdeck-speech-qa/scripts/score.mjs")
    rng = random.Random(19)
    rows = []
    for i in range(240):
        reference = rng.choices(["open", "notes", "save", "build", "again"], k=180)
        heard = reference.copy()
        if i % 4 == 1:
            heard[90] = "never"
        elif i % 4 == 2:
            del heard[:5]
        elif i % 4 == 3:
            heard.append("now")
        rows.append((" ".join(reference), " ".join(heard)))
    rows.extend([("", ""), ("", "hello"), ("stop", ""), ("CAFÉ!", "cafe\u0301"),
                 ("ab cd ef", "ef cd ab"), ("one one one", "one")])
    with tempfile.TemporaryDirectory() as tmp:
        old = Path(tmp, "baseline.mjs")
        old.write_bytes(subprocess.check_output(["git", "show", f"{base}:{script.as_posix()}"]))
        corpus = Path(tmp, "clips.jsonl")
        corpus.write_text("\n".join(json.dumps(dict(clip=str(i), expected=a, actual=b, latencyMs=i))
                                    for i, (a, b) in enumerate(rows)), encoding="utf-8")
        times = {s: [] for s in (old, script)}
        expected = None
        for run in range(8):
            for scorer in ((old, script) if run % 2 == 0 else (script, old)):
                start = time.perf_counter()
                result = json.loads(subprocess.check_output(["node", scorer, corpus]))
                elapsed = (time.perf_counter() - start) * 1000
                if expected is None:
                    expected = result
                if result != expected:
                    raise SystemExit("Scorers disagree on corpus results")
                if run:
                    times[scorer].append(elapsed)
        for label, samples in zip(("base", "working tree"), times.values()):
            print(f"{label}: median {statistics.median(samples):.1f} ms "
                  f"(range {min(samples):.1f}-{max(samples):.1f})")
        print(f"{len(rows)} clips, identical scores")


if __name__ == "__main__":
    main(sys.argv[1])
