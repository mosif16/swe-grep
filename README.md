# SWE-Grep: The #1 Agent Search Tool

**Instant code understanding in a single call.** Supports Rust, Swift, TypeScript, JavaScript, Go, and Python.

```bash
# One call = complete understanding
swe-grep context --symbol AuthService --path .

# Returns: definition + callers + callees + types + imports + tests
```

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
- Config defaults: create `~/.swe-grep.toml` (or set `SWE_GREP_CONFIG=/path/to/file.toml`) to set defaults for common flags; CLI flags always win.
- The default build does **not** pull in Tantivy, so compilation stays fast and dependency-light.
- Persistent hints + analysis caches are stored under `$XDG_CACHE_HOME/swe-grep/<root-hash>/` (or `$HOME/.cache/swe-grep/<root-hash>/`); fallback is `<root>/.swe-grep-cache/<root-hash>/`.
- Language-aware rewrites can now be pre-seeded from the CLI: pass `--language swift`, `--language tsx`, or multi-language presets such as `--language auto-swift-ts` to hydrate Swift/TypeScript heuristics simultaneously (snippets, AST-grep, cache hints).

Example `~/.swe-grep.toml`:

```toml
disable_telemetry = true

[search]
language = "swift"
max_matches = 50
top_hits = 10

[search.tools]
use_fd = true
use_ast_grep = true

[agent]
enabled = true
format = "markdown"
concise = true
```

## Agent Mode

- `swe-grep search --agent --symbols Foo,Bar --path . --format json` — batch search with jq-ready output and schema versioning.
- `swe-grep search --json-only --symbols Foo,Bar --path .` — shorthand for pipeline-safe JSON-only output (implies `--agent --format json --quiet`).
- `swe-grep search --agent --symbols Foo,Bar --stream` — NDJSON per symbol (final line has `done=true`), exit code `2` on runtime errors, `1` on usage errors.
- `swe-grep search --agent --grouped --budget 2000 --symbol Foo --path .` — semantic grouping (definition/impl/usages/tests) with token-budget truncation to avoid re-sending redundant context.
- Defaults: telemetry off, bodies fetched (first 50 lines by default), deterministic ordering; add `--definitions` for best-effort signatures, `--concise` to drop context/origin/score/body, and `--json-pretty`/`--progress`/`--trim-body N` to tune output.
- See `docs/agent-mode.md` for the schema, exit codes, batch inputs (`--symbols/--symbols-file/--stdin-symbols`), and `--format json|table`.

## Discovery Helpers

- Concept/semantic search: `swe-grep search --concept "text to speech"` builds intent-driven rewrites that surface related symbols even when names do not match.
- Literal text: `swe-grep search --text --symbol "Practice aloud" --path .` performs raw fixed-string searches without symbol rewrites or AST.
- Regex/wildcard friendly: `--regex` accepts raw patterns (e.g. `speak|aloud`), `--wildcard` translates `*`/`?`; AST disambiguation is skipped automatically when patterns are non-literal.
- Fuzzy no-hit guidance: when there are zero hits, swe-grep returns `fuzzy_matches` + `did_you_mean` (agent mode) to suggest symbols like `AuthService` for a query like `authservice`; tune with `--fuzzy 0.7` (0.0–1.0).
- Single-file scope: `swe-grep search --symbol deckHeaderSection --scope-file path/to/View.swift --path .` restricts the search surface to a single file (useful for local/private symbols).
- String literals: `--string` generates quoted/ci variants and disables AST so inline CSS vars or JSX strings are easy to surface.
- CSS/SCSS aware: `--language css|scss` scopes discovery to stylesheets; extension filters include CSS/SCSS by default when selected.
- Localization-aware: `--include-strings` folds `xcstrings`/`strings`/`arb`/`json` resources into the search surface so string keys show up alongside code hits.
- Path globs: `--path-glob "src/**/*.tsx"` narrows scope without forcing a language hint; accepts comma-separated patterns.
- Output formats: `--format json|json-lines|table|compact|tree|edit|minimal|summary|markdown|csv` (agent mode).
- Shell completions: `swe-grep completions zsh` (or `bash`/`fish`) prints a completion script to stdout.
- Usage graph: `swe-grep usages --symbol Foo --path .` emits a tree of references/types/imports; `--format json|compact` are available for automation.
- Cross-reference: `swe-grep refs --symbol Foo --callers|--callees --path .` surfaces callers (best-effort static call graph; cached + incremental) and callees (extracted from the symbol definition body).
  - **Qualified name support**: `--symbol Foo::new` finds only `Foo::new()` callers, not all `new()` calls.
  - **Import graph**: `--imports` / `--importers` plus `--import-depth N` for transitive traversal.
  - **Output formats**: `--format json|tree|compact` for different use cases.
- Context bundles: `swe-grep context --symbol Foo --path .` assembles a single JSON response with the definition, callers/callees, type relationships, and related tests (best-effort).
  - **Presets**: `--preset refactor|debug|understand` selects common bundles; flags like `--tests` and `--body` can add more detail.
  - **Symbol intelligence**: `--explain` enriches the `definition` with kind/visibility/docs/fields/methods/complexity (not available in `--concise` mode).
  - **Concise mode**: `--concise` returns a minimal object (`symbol/def/callers/callees/tests`) for token-sensitive prompts.
  - **Grouped mode**: `--grouped --budget 2000` groups output by file and truncates intelligently to reduce repeated context.
  - **Streaming**: `--stream` outputs NDJSON sections (`definition`, `callers`, `callees`, `types`, `imports`, `tests`, `summary`) for incremental UIs/pipelines.
- Git metadata: add `--git-info` to search/usages/refs to append the last commit hash/date per hit.
- Removal planning: `swe-grep remove --symbol Foo --dry-run` lists files to modify/delete; add `--apply` to delete obvious symbol-owned files cautiously.
- Cache hygiene: `--clear-cache` wipes the computed cache dir (including `facts/` + `graphs/`) plus legacy `.swe-grep-cache/` + `.swe-grep-index/` before executing the chosen subcommand.
- Tool feedback: `swe-grep feedback shell --command "<cmd>" --prompt-only` prints a JSON-shaped prompt for collecting improvements to the `shell_command` tool; pipe the response back to persist under `<cwd>/Feedback` (when `--cwd` is provided).
- Feedback autofix: drop records into `Feedback/inbox/` and run `./scripts/feedback/process_inbox.sh` (or enable `.github/workflows/feedback-autofix.yml`) to have Codex implement + verify changes on a branch/PR.
- Batch everywhere: multiple symbols now work without `--agent` (default mode emits a JSON array); agent mode still supports streaming and table output.
- No-hit guidance: empty searches now return related symbols/paths plus notes when the search was constrained (regex/wildcard/strings), making reruns faster.
- Telemetry defaults: set `SWE_GREP_TELEMETRY=off` or add `disable_telemetry = true` to `~/.swe-grep.toml` to avoid passing `--disable-telemetry` every run.
- Config defaults: `~/.swe-grep.toml` can also provide defaults for common search flags (e.g. language/timeout/max-matches/max-columns/context/body). CLI flags always win.

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

## Desktop App (GUI)

- Dev: `cargo run -p swe-grep-desktop`
- Release: `cargo build --release -p swe-grep-desktop` (binary at `target/release/swe-grep-desktop`)

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
- Per-language telemetry (discover/probe/ast/verify counters + latency) is emitted with every cycle in `SearchSummary`; consume the `stage_stats.language_metrics` map to track Rust/Swift/TypeScript/Go/Python coverage in benchmarks and regression dashboards.

## Changelog

- Recent commits are summarized in `docs/changelog.md`; see that file for a latest-first snapshot or run `git log` for full history.
