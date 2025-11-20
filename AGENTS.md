# SWE-Grep Agent Playbook

Authoritative guide for agents working in this repository. Follow these rules exactly; non-compliance can break upstream automation.

## Mandatory Rules
- Use `swe-grep` for all code searches in supported languages (Rust/Swift/TypeScript/TSX/JS/etc). Do **not** fall back to `rg/grep` unless targeting unsupported file types.
- Use Tavily MCP for any web search.
- Always delete `swe-grep` cache directories after use. If you set `--cache-dir`, remove that folder when done (default is `<repo>/.swe-grep-cache`, also safe to delete).

## Core CLI Flow
- Fast path for literal identifiers: `./target/release/swe-grep --disable-telemetry search --symbol foo_bar --path <repo> --cache-dir /tmp/swe-grep-cache`.
- Non-literal or mixed-case identifiers automatically run the full pipeline (fd → rg → optional index/rga → ast-grep).
- Context: omit `--context-before/--context-after` to auto-attach ±2 lines (±4 when `rg` truncates); flags override this. `top_hits[].expanded_snippet/context_start/context_end/auto_expanded_context` describe the window.
- Bodies: Rust and Swift hits include full bodies by default; other languages require `--body`. Guardrail: 512 KiB.
- Feature toggles: `--disable-fd`, `--disable-ast-grep`, `--enable-rga`, `--enable-index` (requires `cargo build --features indexing`). Language hints (`--language rust|swift|ts|tsx|auto-swift-ts`) tighten rewrites/AST scopes.
- Logging/telemetry: add `--log-dir <dir>` for JSONL summaries; set `--disable-telemetry` or `RUST_LOG=off` in minimal environments.
- Cleanup: remove any directory passed to `--cache-dir` (or `.swe-grep-cache/` if left default) after runs.

## Service Integration
- Start servers: `./target/release/swe-grep serve --path <repo> --http-addr 127.0.0.1:8080 --grpc-addr 127.0.0.1:50051 [--disable-telemetry] [--cache-dir ...]`.
- HTTP: `POST /search` with the CLI-equivalent JSON body; `GET /healthz`; `GET /metrics` for Prometheus text.
- gRPC: service `swegrep.v1.SweGrepService` (`Search`, `Health`). `tool_flags` map toggles fd/ast-grep/index/rga/body. **Limitation:** current gRPC mapping ignores `enable_index=false`/`enable_rga=false` values (defaults prevail); prefer HTTP or `tool_flags` to force disablement.
- Responses mirror CLI: `top_hits[].raw_snippet/raw_snippet_truncated/snippet_length/expanded_snippet/body/body_retrieved/hints`, `next_actions`, `stage_stats` (latency + per-language metrics), `startup_stats`.

## Performance Hints
- Prefer literal identifiers to hit the fast `rg` union path. For bulk scans, disable AST (and optionally fd) to cut overhead.
- Timeouts/concurrency: defaults 3s/8. Tune down for large repos or sandboxed hosts.
- Indexing: enable only when built with `--features indexing`; index stored in `.swe-grep-index/`.

## Benchmarks & Tests
- Smoke tests: `cargo test -p swe-grep-core`.
- Benchmarks: `cargo run -p swe-grep -- bench [--iterations N --scenario benchmarks/default.json --output docs/benchmark-summary.jsonl]`.
- Startup/rg vs swe-grep comparisons: `python scripts/bench_startup.py ...`, `python scripts/bench_rg_vs_sweg.py --repo <path> --symbol <id> --runs N`.
- Validate latency guardrails: `python scripts/check_bench_regression.py --summary docs/benchmark-summary.jsonl --max-latency-ms 20 --min-success 0.99`.

## Operational Pitfalls
- Telemetry gap: `startup_stats.cache_ms` is never populated, so cache init latency is always reported as 0.
- gRPC flag caveat (above) prevents clients from turning off index/rga when the server default is enabled.
- Binaries required on PATH: `fd`, `rg`, `ast-grep` (`sg`), optional `rga`; use `scripts/install-tools.sh` to install.

## Usage Checklist
- [ ] Default to `swe-grep` for code search; Tavily MCP for web search.
- [ ] Pick a temp `--cache-dir` and delete it after use.
- [ ] Capture logs/metrics only when needed; disable telemetry in constrained runs.
- [ ] Surface `top_hits` bodies/snippets to avoid extra file fetches in agents.
