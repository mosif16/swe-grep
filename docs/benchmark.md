# Benchmark Log

All benchmark runs must be tracked here with command, environment, timestamp, and observed metrics.

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
