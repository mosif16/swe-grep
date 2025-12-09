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

## Agent Mode

- `swe-grep search --agent --symbols Foo,Bar --path . --format json` — batch search with jq-ready output and schema versioning.
- `swe-grep search --agent --symbols Foo,Bar --stream` — NDJSON per symbol (final line has `done=true`), exit code `2` on runtime errors, `1` on usage errors.
- Defaults: telemetry off, bodies fetched (first 50 lines by default), deterministic ordering; add `--definitions` for best-effort signatures, `--concise` to drop context/origin/score/body, and `--json-pretty`/`--progress`/`--trim-body N` to tune output.
- See `docs/agent-mode.md` for the schema, exit codes, batch inputs (`--symbols/--symbols-file/--stdin-symbols`), and `--format json|table`.

## Discovery Helpers

- Concept/semantic search: `swe-grep search --concept "text to speech"` builds intent-driven rewrites that surface related symbols even when names do not match.
- Regex/wildcard friendly: `--regex` accepts raw patterns (e.g. `speak|aloud`), `--wildcard` translates `*`/`?`; AST disambiguation is skipped automatically when patterns are non-literal.
- String literals: `--string` generates quoted/ci variants and disables AST so inline CSS vars or JSX strings are easy to surface.
- CSS/SCSS aware: `--language css|scss` scopes discovery to stylesheets; extension filters include CSS/SCSS by default when selected.
- Localization-aware: `--include-strings` folds `xcstrings`/`strings`/`arb`/`json` resources into the search surface so string keys show up alongside code hits.
- Path globs: `--path-glob "src/**/*.tsx"` narrows scope without forcing a language hint; accepts comma-separated patterns.
- Output formats: `--format compact|tree|edit` (agent mode) for fast scanning, grouped results, or edit-tool ready `path:line:snippet` lines.
- Usage graph: `swe-grep usages --symbol Foo --path .` emits a tree of references/types/imports; `--format json|compact` are available for automation.
- Removal planning: `swe-grep remove --symbol Foo --dry-run` lists files to modify/delete; add `--apply` to delete obvious symbol-owned files cautiously.
- Cache hygiene: `--clear-cache` wipes `.swe-grep-cache` and `.swe-grep-index` before executing the chosen subcommand.
- Batch everywhere: multiple symbols now work without `--agent` (default mode emits a JSON array); agent mode still supports streaming and table output.
- No-hit guidance: empty searches now return related symbols/paths plus notes when the search was constrained (regex/wildcard/strings), making reruns faster.
- Telemetry defaults: set `SWE_GREP_TELEMETRY=off` or add `disable_telemetry = true` to `~/.swe-grep.toml` to avoid passing `--disable-telemetry` every run.

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

- Persistent hints now degrade quietly when the cache directory is missing or unwritable; state is disabled without surfacing user-visible warnings.
- fd/ast-grep are treated as optional: a single “command not found” disables the tool for the run with only debug logs; real runtime failures still surface in `warnings`.
- The AST pattern emitted by `ast-grep` still prints a warning when no matches are found; it’s benign but worth revisiting while hardening the pattern.
- Keep `.swe-grep-cache/` out of version control; it is safe to delete if you want a clean slate for heuristics.
- Per-language telemetry (discover/probe/ast/verify counters + latency) is emitted with every cycle in `SearchSummary`; consume the `stage_stats.language_metrics` map to track Swift/TypeScript/Rust coverage in benchmarks and regression dashboards.

## Changelog

- Recent commits are summarized in `docs/changelog.md`; see that file for a latest-first snapshot or run `git log` for full history.
