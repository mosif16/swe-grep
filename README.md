# SWE-Grep Workspace Guide

## Workspace Layout

- `crates/swe-grep-core`: main binary crate that drives the SWE-grep search workflow (fd/rg/ast-grep, rga fallback, persistent hints, telemetry).
- `crates/swe-grep-indexer`: optional Tantivy-powered indexer that can accelerate fallback discovery.
- `Cargo.toml` (root): declares the workspace and lets you target each crate with standard `cargo` commands.

## Default Build

```bash
cargo check
cargo run -p swe-grep -- search --symbol foo --timeout-secs 2
```

- Disable telemetry if you are running in minimal environments: `cargo run -p swe-grep -- --disable-telemetry search --symbol foo`.
- The default build does **not** pull in Tantivy, so compilation stays fast and dependency-light.
- Persistent hints are stored under `.swe-grep-cache/` (already ignored by git).
- Language-aware rewrites can now be pre-seeded from the CLI: pass `--language swift`, `--language tsx`, or multi-language presets such as `--language auto-swift-ts` to hydrate Swift/TypeScript heuristics simultaneously (snippets, AST-grep, cache hints).

## Optional Tantivy Indexing

To enable the Tantivy indexer:

```bash
cargo run -p swe-grep --features indexing -- \
  search --symbol foo --enable-index
```

- This builds `swe-grep-indexer`, which bundles Tantivy 0.18 with `lz4` + `mmap` support.
- The index is stored in `.swe-grep-index/` within the repository root; it is created or refreshed automatically on first use.
- Because indexing relies on additional native tooling (e.g. `mmap`), keep it optional in CI unless you explicitly need the speedup.

## ripgrep-all Fallback

```bash
cargo run -p swe-grep -- search --symbol foo --enable-rga
```

- Falls back to `rga` when scoped `rg` searches miss.
- Combine with the indexing feature if you want both doc/config coverage and Tantivy hints.

## Testing Targets

- `cargo check -p swe-grep` — compile just the core agent.
- `cargo check -p swe-grep-indexer` — compile the indexer crate.
- `cargo fmt` — format across the workspace.

## Benchmarking

- `cargo run -p swe-grep -- bench` — execute the default scenarios under `benchmarks/default.json`.
- `cargo run -p swe-grep --features indexing -- bench --enable-index --enable-rga --output docs/benchmark-summary.jsonl` — run with indexing + rga enabled and append results to a log file.
- All benchmark runs must also be summarised in `docs/benchmark.md` to track progress across phases.
- `python scripts/bench_startup.py --repo <path> --symbol <name> [--language swift]` — measures cold/warm start, stage timings, and startup stats for a single query.
- `python scripts/check_bench_regression.py --summary docs/benchmark-summary.jsonl --max-latency-ms 20 --min-success 0.99` — CI-friendly guard that fails if latency or success rate drifts beyond the stated thresholds.

## Serving the API

```bash
cargo run -p swe-grep -- serve --http-addr 127.0.0.1:8080 --grpc-addr 127.0.0.1:50051
```

- Add `--path /absolute/repo/root` to pin the server to a repository from the CLI.
- Combine with `--disable-telemetry` when exposing the service in environments without Prometheus/OpenTelemetry collectors.
- HTTP endpoints: `/healthz`, `/search`, `/metrics`. gRPC exposes `swegrep.v1.SweGrepService` with the same search payloads (including startup/stage stats).

## Notes

- The AST pattern emitted by `ast-grep` currently prints a warning when no matches are found; it’s benign but worth revisiting while hardening the pattern.
- Keep `.swe-grep-cache/` out of version control; it is safe to delete if you want a clean slate for heuristics.
- Per-language telemetry (discover/probe/ast/verify counters + latency) is emitted with every cycle in `SearchSummary`; consume the `stage_stats.language_metrics` map to track Swift/TypeScript/Rust coverage in benchmarks and regression dashboards.
