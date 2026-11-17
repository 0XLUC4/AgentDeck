---
name: agentdeck-speech-qa
description: Measure dictation latency and transcription accuracy on a repeatable local corpus. Use when changing microphone capture, speech recognition, dictation cleanup, model selection, or GPU acceleration.
---

# Test dictation

1. Pick the same recordings for both builds. Include short commands, a pause in the middle of a sentence, quiet word endings, agent names, code vocabulary, and silence. Use synthetic recordings or clips provided for this test.
2. Record the expected words before transcribing. Include at least one command where a single missing word changes the meaning. Check its destination pane and submit behavior separately.
3. Keep model, language, and audio identical. Measure cold startup separately. For warm runs, time the interval from releasing push-to-talk until text appears. If only inference is timed, label it as inference. Record which backend and device the engine actually selected.
4. Save one JSON object per clip per repetition in a JSONL file per build. Use the same number of repetitions for each clip in both builds.

   ```json
   {"clip":"short-fr","expected":"lance les tests","actual":"lance les tests","latencyMs":85}
   ```

5. Run `node scripts/score.mjs results.jsonl` for each build. Compare word error rate and warm latency, then inspect the affected clips. The scorer ignores case and punctuation. Review technical identifiers, negations, accents, and routing separately. A silent clip has an empty expected string and any words it produces count as insertions.
