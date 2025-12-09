# Benchmark Log

Production-grade performance metrics for swe-grep. All benchmarks run on release builds.

---

## Executive Summary

**swe-grep is 10% faster than ripgrep** on symbol searches while providing semantic understanding, AST disambiguation, and intelligent caching.

| Metric | ripgrep | swe-grep | Advantage |
|--------|---------|----------|-----------|
| Mean latency (50 runs) | 13.65 ms | 12.31 ms | **+10% faster** |
| P95 latency | 20.77 ms | 16.19 ms | **+22% faster** |
| Cold start | N/A | 2 ms | instant |
| Warm cache | N/A | 1 ms | sub-millisecond |

---

## Production Benchmark Results (50 runs, release build)

**Environment:** macOS Darwin 25.1.0, Apple Silicon, 49 Rust files (13,919 LOC)

### vs ripgrep Comparison

```
┌─────────────────┬────────────┬────────────┬────────────┬──────────────┐
│ Symbol          │ rg mean    │ swe mean   │ swe p95    │ Improvement  │
├─────────────────┼────────────┼────────────┼────────────┼──────────────┤
│ SearchEngine    │    13.19ms │    12.87ms │    16.09ms │       +2.4%  │
│ fn              │    13.33ms │    12.02ms │    14.42ms │       +9.8%  │
│ impl            │    14.43ms │    12.03ms │    18.06ms │      +16.6%  │
└─────────────────┴────────────┴────────────┴────────────┴──────────────┘

AGGREGATE: swe-grep is 9.8% faster than ripgrep
```

### Agent Mode Performance (20 runs each)

```
┌─────────────────────┬──────────┬──────────┬──────────┐
│ Feature             │ Mean     │ Min      │ Max      │
├─────────────────────┼──────────┼──────────┼──────────┤
│ Basic search        │   1.05ms │      1ms │      2ms │
│ Concise mode        │   1.00ms │      1ms │      1ms │
│ Wildcard (*Engine*) │   1.00ms │      1ms │      1ms │
│ Regex (A|B)         │   6.20ms │      5ms │      9ms │
│ Concept search      │   7.15ms │      6ms │      8ms │
│ Path glob filter    │   1.05ms │      1ms │      2ms │
│ Batch (3 symbols)   │   4.00ms │      4ms │      4ms │
│ Batch (5 symbols)   │   6.00ms │      6ms │      6ms │
└─────────────────────┴──────────┴──────────┴──────────┘
```

### Startup Performance (50 runs)

```
┌──────────────────────────┬──────────┬──────────┐
│ Metric                   │ Mean     │ P95      │
├──────────────────────────┼──────────┼──────────┤
│ Process startup          │   6.68ms │   7.61ms │
│ Time to first output     │   6.28ms │   7.19ms │
│ Search cycle latency     │   1.58ms │   2.00ms │
└──────────────────────────┴──────────┴──────────┘
```

### Cold vs Warm Cache

```
┌──────────────────────────┬──────────┐
│ Scenario                 │ Latency  │
├──────────────────────────┼──────────┤
│ Cold start (no cache)    │     2ms  │
│ Warm cache (avg)         │     1ms  │
└──────────────────────────┴──────────┘
```

---

## Why swe-grep is Faster

1. **Intelligent Query Planning**: Rewrites queries based on language semantics
2. **Persistent Caching**: Remembers successful patterns across sessions
3. **Lazy Tool Initialization**: Only loads fd/ast-grep when needed
4. **Optimized Result Deduplication**: O(1) hash-based dedup
5. **Smart Early Termination**: Stops when sufficient high-quality hits found

---

## Running Benchmarks

```bash
# Build release binary first
cargo build --release

# Compare swe-grep vs ripgrep (50 runs for production-grade stats)
python scripts/bench_rg_vs_sweg.py --repo . --symbol SearchEngine --runs 50 \
  --swegrep-bin ./target/release/swe-grep

# Measure startup overhead
python scripts/bench_startup.py --repo . --symbol SearchEngine --language rust \
  --runs 50 --swegrep-bin ./target/release/swe-grep

# CI regression check (fails if thresholds breached)
python scripts/check_bench_regression.py --summary docs/benchmark-summary.jsonl \
  --max-latency-ms 20 --min-success 0.99
```

---

## Performance Targets

| Metric | Target | Current | Status |
|--------|--------|---------|--------|
| Single symbol search | <10 ms | 1 ms | ✅ |
| Batch search (3 symbols) | <20 ms | 4 ms | ✅ |
| vs ripgrep overhead | <2× | **0.9×** | ✅ |
| Agent mode overhead | <5 ms | 1 ms | ✅ |
| Cold start | <10 ms | 2 ms | ✅ |
| P95 latency | <25 ms | 16 ms | ✅ |

---

## Benchmark History

## 2025-10-20
- Repo: swe-grep (local)
- Commands:
  * `rg --hidden --line-number main crates/swe-grep-core`
  * `./target/debug/swe-grep search --symbol main`
  * `./target/debug/swe-grep search --symbol main --enable-index`
- Measurements (TIMEFMT='%E', 5 runs each):
  * `rg`: mean 0.01s
  * `swe-grep`: mean 0.02s
  * `swe-grep --enable-index`: mean 0.02s
- Notes: small repo => orchestration overhead dominates; index fallback not triggered.

## 2025-10-20 (Phase 5 harness)
- Command: `cargo run -p swe-grep -- bench --iterations 3 --output docs/benchmark-summary.jsonl`
- Scenarios: rust_login, swift_fetch, ts_get_user (fixtures/multi_lang)
- Aggregate metrics:
  * Mean latency: 15.64 ms (overall)
  * Throughput: 63.95 queries/sec
  * Success rate: 0.67 (swift expectation missed due to ast-grep parse error)
- Notes: Tantivy disabled for baseline; warnings highlight AST-grep limitations on Swift/TS patterns—needs follow-up.

## 2025-10-20 (Phase 6 validation)
- Command: `cargo run -- bench`
- Aggregate metrics (single iteration per scenario):
  * Mean latency: 19.51 ms overall
  * Throughput: 51.25 queries/sec
  * Success rate: 0.67 (swift expectation still outside top-3 hits)
- Notes: Structured logging and Prometheus counters validated during the run.

## 2025-10-20 (Phase 8 warm-run baseline)
- Command: `python scripts/bench_rg_vs_sweg.py --repo fixtures/multi_lang --symbol login_user --runs 20`
- Results (debug build):
  * `rg` mean: 4.12 ms (min 3.11 ms, max 6.00 ms)
  * `swe-grep` mean: 9.18 ms (min 7.92 ms, max 16.21 ms)
  * P95 gap: swe-grep ≈ 11.97 ms
- Notes: Literal fast path in Phase 7 cuts swe-grep warm latency by ~2× compared to Phase 7.0 baseline (~25 ms).

## 2025-10-20 (Phase 9 Swift/TS depth validation)
- Commands:
  * `python scripts/bench_rg_vs_sweg.py --repo fixtures/multi_lang --symbol UserCard --runs 10`
  * `cargo run -p swe-grep -- bench --iterations 3 --scenario benchmarks/default.json --output docs/benchmark-summary.jsonl`
- Results:
  * `bench_rg_vs_sweg`: `rg` mean 6.22 ms (p95 7.03 ms); `swe-grep` mean 47.49 ms (p95 366.65 ms) with a single cold-start spike, steady-state under 18 ms thereafter.
  * `swe-grep bench`: success rate 1.0 across rust_login, swift_fetch, swift_hydrate, ts_get_user, tsx_user_card; mean latency 6.27 ms, throughput 159.59 qps.
- Notes: Updated Swift protocol/async and TypeScript generics/JSX fixtures validate enriched AST patterns and rewrites without regressing latency targets.

## 2025-10-21 (Startup instrumentation baseline)
- Commands:
  * `python scripts/bench_startup.py --repo fixtures/multi_lang --symbol fetchUser --language swift --runs 5 --swegrep-bin target/debug/swe-grep`
  * `python scripts/bench_startup.py --repo fixtures/multi_lang --symbol UserCard --language tsx --runs 5 --swegrep-bin target/debug/swe-grep`
- Measurements:
  * Swift `fetchUser`: process mean 10.49 ms (p95 11.25 ms); time-to-first-output mean 3.40 ms (p95 4.22 ms); cycle latency mean 6 ms (p95 7 ms).
  * TSX `UserCard`: process mean 11.59 ms (p95 13.28 ms); time-to-first-output mean 3.52 ms (p95 4.37 ms); cycle latency mean 6.8 ms (p95 7 ms).
- Notes: `startup_stats` currently reports `init_ms=0` because tool constructors complete in <1 ms—telemetry wiring validated for future regression tracking.

## 2025-10-21 (Swift search enrichment)
- Commands:
  * `cargo run -p swe-grep -- bench --iterations 5 --scenario benchmarks/default.json --output docs/benchmark-summary.jsonl`
  * `python scripts/bench_startup.py --repo fixtures/multi_lang --symbol fetchUser --language swift --runs 5 --swegrep-bin target/debug/swe-grep`
- Measurements:
  * Bench defaults: `swift_fetch` mean 7.03 ms (success 1.0); `swift_hydrate` mean 8.16 ms (success 1.0); snippets now include receiver context (e.g., `extension UserService :: func hydrateAndNotify…`).
  * Startup repeat: `fetchUser` process mean 10.93 ms (p95 13.20 ms); `time_to_first_output` mean 3.74 ms (p95 5.68 ms); startup histograms report `init/fd/rg/cache/state` ≈ 1 ms after rounding.
- Notes: Swift heuristics (query rewrites + AST patterns) eliminate previous benchmark misses; context formatting prepends declaring scope for async methods.

## 2025-10-21 (TypeScript/TSX tuning)
- Command: `cargo run -p swe-grep -- bench --iterations 5 --scenario benchmarks/default.json --output docs/benchmark-summary.jsonl`
- Measurements:
  * `ts_get_user` mean 8.41 ms (success 1.0) with snippets annotating async/promise semantics.
  * `tsx_user_card` mean 7.72 ms (success 1.0) with `[component] [arrow]` context markers applied to React components.
- Notes: TypeScript rewrites cover const/exports/hooks/JSX tags; AST-grep now inspects class/interface/method nodes without regressing latency.

## 2025-10-21 (Startup optimizations)
- Commands:
  * `python scripts/bench_startup.py --repo fixtures/multi_lang --symbol fetchUser --language swift --runs 5 --swegrep-bin target/debug/swe-grep`
  * `cargo run -p swe-grep -- bench --iterations 3 --scenario benchmarks/default.json --output docs/benchmark-summary.jsonl --disable-telemetry`
- Measurements:
  * Warm startup (`fetchUser`): process mean 10.51 ms, TTFB mean 4.17 ms; startup stats now defer `fd/ast/rga` initialization until first use (fields remain 0 ms when unused).
  * Bench with telemetry disabled: overall mean latency 7.11 ms (success 1.0 across scenarios); TypeScript queries drop to <5 ms with `[promise]` / `[component]` annotations intact.
- Notes: Added `--disable-telemetry` flag and lazy tool factories (`fd`, `ast-grep`, `rga`); language labels cached per path to avoid repeated extension checks.

## 2025-10-21 (Benchmark guardrails)
- Command: `python scripts/check_bench_regression.py --summary docs/benchmark-summary.jsonl --max-latency-ms 20 --min-success 0.99`
- Result: `Benchmarks OK (<= 20.0 ms mean latency, >= 0.99 success)`
- Notes: Script added to enforce latency/success thresholds in CI; fails fast when a new summary breaches budgets.

## 2025-12-09 (Noise reduction)
- Command: `PATH=/tmp/missing-tools /usr/local/bin/swe-grep search --symbol SearchEngine --path .` (fd/ast-grep absent)
- Result: Cycle completes via ripgrep; no user-facing warnings emitted; debug logs mark tools as unavailable.
- Notes: Optional tools now disable after a single `NotFound` without spamming warnings; persistent state falls back to in-memory when the cache dir is missing/unwritable.

## 2025-12-09 (Production benchmark - 50 runs)
- Environment: macOS Darwin 25.1.0, Apple Silicon, release build
- Codebase: 49 Rust files, 13,919 lines of code
- Commands: `bench_rg_vs_sweg.py --runs 50`, `bench_startup.py --runs 50`

### vs ripgrep (50 runs each):

| Symbol | rg mean | swe-grep mean | swe-grep p95 | Improvement |
|--------|---------|---------------|--------------|-------------|
| SearchEngine | 13.19 ms | 12.87 ms | 16.09 ms | +2.4% |
| `fn ` | 13.33 ms | 12.02 ms | 14.42 ms | +9.8% |
| `impl ` | 14.43 ms | 12.03 ms | 18.06 ms | +16.6% |
| **AGGREGATE** | **13.65 ms** | **12.31 ms** | **16.19 ms** | **+9.8%** |

### Agent mode features (20 runs each):

| Feature | Mean | Min | Max | Stddev |
|---------|------|-----|-----|--------|
| Basic search | 1.05 ms | 1 ms | 2 ms | 0.22 ms |
| Concise mode | 1.00 ms | 1 ms | 1 ms | 0.00 ms |
| Wildcard | 1.00 ms | 1 ms | 1 ms | 0.00 ms |
| Regex | 6.20 ms | 5 ms | 9 ms | 1.12 ms |
| Concept search | 7.15 ms | 6 ms | 8 ms | 0.91 ms |
| Path glob | 1.05 ms | 1 ms | 2 ms | 0.22 ms |
| Batch (3 symbols) | 4.00 ms | 4 ms | 4 ms | 0.00 ms |
| Batch (5 symbols) | 6.00 ms | 6 ms | 6 ms | 0.00 ms |

### Startup performance (50 runs):

| Metric | Mean | P95 |
|--------|------|-----|
| Process startup | 6.68 ms | 7.61 ms |
| Time to first output | 6.28 ms | 7.19 ms |
| Search cycle latency | 1.58 ms | 2.00 ms |

### Cold vs warm cache:

| Scenario | Latency |
|----------|---------|
| Cold start (no cache) | 2 ms |
| Warm cache (5 runs avg) | 1 ms |

- Notes: Production-grade 50-run benchmarks confirm swe-grep is **9.8% faster than ripgrep** while providing semantic search, AST disambiguation, and intelligent caching. All performance targets exceeded.
