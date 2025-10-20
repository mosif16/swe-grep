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
