---
name: agentdeck-profile
description: Find and fix what makes code slow or memory-hungry with measurements, not guesses - benchmarks, profilers, flamegraphs, query plans, allocation counts - for Rust, Node/TypeScript/Bun, Python, Go, C#, SQL and startup or build times. Use when something is slow, uses too much memory or CPU, freezes, or when asked to optimize.
---

# Profile before optimizing

## 1. Reproduce with a number

Write the slow case as a command or test that prints a time (or memory). Realistic data size, release
build, warm run. Run it at least 3 times (`hyperfine 'cmd'` does it for you); keep the median and the
spread. No number, no optimization. Set a target ("under 200 ms").

## 2. Find the hot spot

Profile, do not read the code and guess. The slow part is rarely where you think.

| Stack | Time | Memory |
|---|---|---|
| Rust | `cargo build --release` + `samply record` / `cargo flamegraph`; `criterion` or `divan` benches; `debug = true` in `[profile.release]` for symbols | `dhat`, `heaptrack`, count allocations |
| Node / TS | `node --cpu-prof`, `0x`, `clinic flame`/`doctor`; `--inspect` + DevTools | `--heap-prof`, heap snapshots, `process.memoryUsage()` |
| Bun | `bun --inspect`, `mitata` benches | `Bun.gc(true)` + `heapStats()` |
| Python | `py-spy record -o out.svg -- python app.py`, `python -m cProfile -s cumtime`, `scalene` (CPU + memory) | `tracemalloc`, `memray` |
| Go | `go test -bench . -benchmem -cpuprofile cpu.out` + `go tool pprof -http :8080` | `-memprofile`, `pprof` alloc_space |
| .NET | `dotnet-trace`, `dotnet-counters`, BenchmarkDotNet | `dotnet-gcdump` |
| SQL | `EXPLAIN (ANALYZE, BUFFERS)` (Postgres), `EXPLAIN QUERY PLAN` (SQLite), slow query log | rows read vs returned |

Read the flamegraph from the widest boxes down. Also look at what is *not* CPU: waiting on I/O, locks,
network, `sleep`, sequential `await` that could run together.

## 3. Fix the biggest cost only, in this order

1. **Do less work**: skip it, cache it, compute once outside the loop, batch I/O and queries
   (kill N+1), paginate, stream instead of loading everything.
2. **Better algorithm or data structure**: O(n²) nested loops → hash map/set; sorted + binary search;
   right collection (`Vec` vs `HashMap` vs `BTreeMap`, `Set` vs array `includes`).
3. **Database**: index the columns in `WHERE`/`JOIN`/`ORDER BY`, select only needed columns,
   one query instead of many, connection pool, transactions for bulk writes.
4. **Fewer allocations and copies**: reuse buffers, `with_capacity`, borrow instead of clone, avoid
   string building in hot loops, avoid JSON round trips.
5. **I/O**: buffered reads/writes, parallel independent requests (`Promise.all`, `join!`), timeouts.
6. **Parallelism last**: `rayon`, worker threads, process pools; only for CPU-bound work, after the
   above.

Startup slow: measure imports/modules loaded, lazy-load what is not needed at start.
Build slow: `cargo build --timings`, `tsc --extendedDiagnostics`, cache in CI.

## 4. Measure again

Same command, same data. Keep the change only if it is clearly faster (beyond the spread) and the tests
still pass. Revert micro-optimizations that do not show up in the numbers: they cost readability.

## Report

The command, before and after numbers (median), the hot spot found, the change. Say what you did not
measure (other inputs, other machines, concurrency).
