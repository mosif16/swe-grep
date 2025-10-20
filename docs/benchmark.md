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
