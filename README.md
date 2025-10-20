# SWE-Grep MCP Workspace Guide

## Workspace Layout

- `crates/swe-grep-core`: main binary that drives the SWE-grep search workflow (fd/rg/ast-grep, rga fallback, persistent hints, telemetry).
- `crates/swe-grep-indexer`: optional Tantivy-powered indexer that can accelerate fallback discovery.
- `Cargo.toml` (root): declares the workspace and lets you target each crate with standard `cargo` commands.

## Default Build

```bash
cargo check
cargo run -p swe-grep-mcp -- search --symbol foo --timeout-secs 2
```

- The default build does **not** pull in Tantivy, so compilation stays fast and dependency-light.
- Persistent hints are stored under `.swe-grep-cache/` (already ignored by git).

## Optional Tantivy Indexing

To enable the Tantivy indexer:

```bash
cargo run -p swe-grep-mcp --features indexing -- \
  search --symbol foo --enable-index
```

- This builds `swe-grep-indexer`, which bundles Tantivy 0.18 with `lz4` + `mmap` support.
- The index is stored in `.swe-grep-index/` within the repository root; it is created or refreshed automatically on first use.
- Because indexing relies on additional native tooling (e.g. `mmap`), keep it optional in CI unless you explicitly need the speedup.

## ripgrep-all Fallback

```bash
cargo run -p swe-grep-mcp -- search --symbol foo --enable-rga
```

- Falls back to `rga` when scoped `rg` searches miss.
- Combine with the indexing feature if you want both doc/config coverage and Tantivy hints.

## Testing Targets

- `cargo check -p swe-grep-mcp` — compile just the core agent.
- `cargo check -p swe-grep-indexer` — compile the indexer crate.
- `cargo fmt` — format across the workspace.

## Benchmarking

- `cargo run -p swe-grep-mcp -- bench` — execute the default scenarios under `benchmarks/default.json`.
- `cargo run -p swe-grep-mcp --features indexing -- bench --enable-index --enable-rga --output docs/benchmark-summary.jsonl` — run with indexing + rga enabled and append results to a log file.
- All benchmark runs must also be summarised in `docs/benchmark.md` to track progress across phases.

## Notes

- The AST pattern emitted by `ast-grep` currently prints a warning when no matches are found; it’s benign but worth revisiting while hardening the pattern.
- Keep `.swe-grep-cache/` out of version control; it is safe to delete if you want a clean slate for heuristics.
