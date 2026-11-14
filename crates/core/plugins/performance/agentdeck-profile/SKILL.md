---
name: agentdeck-profile
description: Make code faster or leaner with measurements, not guesses - reproducible benchmark, profiler and flamegraph to find the hot spot, biggest cost fixed first, before/after numbers beyond the noise - for Rust, Node/TypeScript/Bun, Python, Go, .NET, SQL, startup and build times. Use when something is slow, uses too much memory or CPU, freezes, or when asked to optimize, before changing any code for speed.
---

# Profile before optimizing

**Iron law: no optimization without a before number, and no claim without an after number beyond
the noise.**

## 1. A number you can reproduce

Turn the slow case into one command or benchmark: realistic data size, optimized build, warm runs.
Compare commands with `hyperfine --warmup 3 -N '<old>' '<new>'` (`--export-json` to keep it). Note
the mean and spread; set a target ("under 200 ms"). Same machine, same build, other heavy apps
closed; a laptop on battery throttles.

## 2. Find the hot spot

Profile; do not read the code and guess. The cost is rarely where you expect.

| Stack | CPU | Memory |
|---|---|---|
| Rust | `debug = true` in `[profile.release]`, then `samply record target/release/app` (Windows, macOS, Linux); `cargo flamegraph`; benches with `divan` or `criterion` + `std::hint::black_box` | `dhat`, `divan` allocation counters, heaptrack |
| Node / TS | `node --cpu-prof app.js` → open `.cpuprofile` in DevTools; `--inspect` | `--heap-prof`, heap snapshots, `process.memoryUsage()` |
| Bun | `bun --cpu-prof`, `bun --inspect`; `mitata` benches | `Bun.gc(true)` + `heapStats()` |
| Python | `py-spy record -o out.svg -- python app.py` (`--native` for C extensions, `--pid` to attach); `scalene app.py` (line level); `python -m cProfile -s cumtime` | `memray run` + `memray flamegraph`, `tracemalloc` |
| Go | `go test -bench . -benchmem -count 10 -cpuprofile cpu.out` → `go tool pprof -http :8080 cpu.out`; `benchstat old.txt new.txt` | `-memprofile`, `pprof -sample_index=alloc_space` |
| .NET | `dotnet-trace collect`, `dotnet-counters`, BenchmarkDotNet | `dotnet-gcdump` |
| SQL | `EXPLAIN (ANALYZE, BUFFERS)` (Postgres), `EXPLAIN QUERY PLAN` (SQLite), slow query log | rows read vs rows returned |

Read a flamegraph from the widest boxes down. Look also at what is not CPU: I/O waits, locks,
network, `sleep`, sequential `await` that could run together, N+1 queries.

## 3. Fix the biggest cost first, in this order

1. **Do less**: skip it, cache it, hoist it out of the loop, batch I/O and queries (kill N+1),
   paginate, stream instead of loading everything.
2. **Better algorithm or structure**: nested loops over growing data → hash map/set; sort + binary
   search; the right collection.
3. **Database**: index the `WHERE`/`JOIN`/`ORDER BY` columns, select only needed columns, one query
   instead of many, a transaction for bulk writes, a connection pool.
4. **Fewer allocations and copies**: reuse buffers, `with_capacity`, borrow instead of clone, no
   string building or JSON round trips in hot loops.
5. **I/O**: buffered reads and writes, independent requests in parallel (`Promise.all`, `join!`),
   timeouts.
6. **Parallelism last**, for CPU-bound work only: `rayon`, worker threads, process pools.

Startup: measure what loads at start (`node --cpu-prof`, `python -X importtime`), lazy-load the rest.
Build: `cargo build --timings`, `tsc --extendedDiagnostics`, caching in CI.

## 4. Measure again

Same command, same data. Keep the change only if the gain is clearly beyond the spread
(non-overlapping hyperfine ranges, criterion's "change" verdict, `benchstat` p < 0.05) and the tests
pass. Revert micro-optimizations that do not show in the numbers: they cost readability.

## Report

The command, before and after (mean ± spread), the hot spot found and how, the change. What was not
measured: other inputs, other machines, concurrency, memory if you measured time.
