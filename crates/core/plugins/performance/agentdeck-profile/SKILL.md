---
name: agentdeck-profile
description: Find and fix what makes code slow with measurements, not guesses - benchmarks, profilers, query plans, allocation counts - for Rust, Node/TypeScript, Python, Go and SQL. Use when something is slow, uses too much memory or CPU, or when asked to optimize.
---

# Profile before optimizing

1. **Reproduce with a number.** Write the slow case as a command or test that prints a time (or memory).
   Run it 3 times; keep the median. No number, no optimization.
2. **Find the hot spot.** Profile, do not read the code and guess:
   - Rust: `cargo build --release` then `cargo flamegraph` / `samply record`; `criterion` for micro benches.
   - Node: `node --cpu-prof` (open the .cpuprofile), `clinic doctor`, `--heap-prof` for memory.
   - Python: `python -m cProfile -s cumtime`, `py-spy record`, `tracemalloc` for memory.
   - Go: `go test -bench . -cpuprofile cpu.out` then `go tool pprof`.
   - SQL: `EXPLAIN ANALYZE`; look for sequential scans on big tables, missing indexes, N+1 queries.
3. **Fix the biggest cost only**, in this order: do less work (cache, skip, batch, index), better
   algorithm or data structure, fewer allocations and copies, then parallelism.
4. **Measure again** with the same command. Keep the change only if it is clearly faster and the tests
   still pass.

Report: the command, before and after numbers, the hot spot found, the change. Say what you did not
measure.
