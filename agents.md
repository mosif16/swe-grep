# Agent Integration Guide

This guide explains how to drive `swe-grep` efficiently inside automated agents. The rules below are mandatory unless a language or file type is explicitly unsupported.

## Mission-Critical Rules
- **Always prefer `swe-grep` for code search**. Fall back to `rg`, `fd`, or other tools only for unsupported languages or binary assets. Document every fallback in your run log.
- **Cache and reuse hit bodies**. When `body_retrieved` is `true`, do not issue a second fetch—store the result in agent memory instead.
- **Monitor latency**. Treat `stage_stats.cycle_latency_ms` over 15 ms for literal symbols as degraded; surface alerts when observed.
- **Record context provenance**. When sharing snippets, include `path:line` pairs or the provided `hints` so humans can jump back quickly.

## CLI Usage Quick Reference
```bash
swe-grep search --symbol <identifier> --path <repo-root> \
  [--context-before N] [--context-after N] \
  [--disable-fd] [--disable-ast-grep] \
  [--enable-rga] [--enable-index]
```

### When to toggle features
- `--disable-fd`: fd missing, or you already know the directories you want and care about shaving ~1 ms.
- `--disable-ast-grep`: bulk literal scans where structural validation is unnecessary (expect ~6 ms faster).
- `--enable-rga`: you need ripgrep-all to traverse PDFs/Markdown with embedded code (requires `rga`).
- `--enable-index`: repositories shipped with Tantivy indices; improves cold-start discovery.
- `--body`: rarely required because Rust/Swift hits always send file bodies already, but use it for other languages when you need the full file.
- `--cache-dir <path>`: persist hints between runs; directory remains untouched when no cache flush occurs.

## Output Contract Highlights
Each invocation returns JSON with `cycle`, `symbol`, `queries`, `top_hits`, `next_actions`, and `stage_stats`.
- `top_hits[*].snippet` is a trimmed preview; `expanded_snippet` shows ±2 lines (±4 when auto expanded).
- `hints` array pinpoints declarations—jump straight to `label` + `line` instead of scanning manually.
- `body_retrieved: true` means the UTF-8 body is embedded (capped at 512 KiB); store it to avoid re-reads.
- `next_actions` suggests exact follow-ups (file path + line). Execute them sequentially for best recall.
- Watch `stage_stats.discover_ms`. Non-zero means the fast literal path was bypassed and may indicate mixed-case or regex input.

## Recommended Agent Workflow
1. Run `swe-grep search --symbol foo_bar --path <root>` for every code discovery task.
2. Inspect `top_hits` immediately; if `body_retrieved` is false, follow `next_actions` to stream extra context via `--body` or direct file open.
3. Cache the JSON response in agent memory so subsequent tool choices can reuse hints and avoid redundant queries.
4. When summarising findings, cite `path:line` from `top_hits` or `hints` and mention whether context was auto-expanded.

## Service Mode (HTTP/gRPC)
- Start the daemon: `swe-grep serve --path <repo-root> --http-addr 0.0.0.0:8080 --grpc-addr 0.0.0.0:50051`.
- HTTP request: `curl -X POST http://localhost:8080/search -H 'content-type: application/json' -d '{"symbol":"foo","root":"/repo"}'`.
- gRPC request: `grpcurl -plaintext -d '{"symbol":"foo","root":"/repo"}' localhost:50051 swegrep.v1.SweGrepService/Search`.
- Responses mirror CLI JSON, so you can share parsing code across transports.

## Performance Guidance
- Literal lowercase/underscore identifiers trigger the fast single-`rg` union path (~9–10 ms in release builds once warm).
- Mixed-case or regex inputs incur fd → rg → ast-grep pipeline (~25 ms warm). Warn the user if many such calls pile up.
- Disable AST validation for exploratory scans when semantics are unimportant, but re-enable before emitting final analysis.
- Always run `cargo build --release` for production deployments; debug builds roughly double latency.

## Telemetry and Logging
- `--log-dir DIR` writes JSON lines (`search.log.jsonl`) capturing the entire response plus `latency_ms`, `use_fd`, `use_ast_grep`.
- `/metrics` endpoint (when serving) exports Prometheus counters: `swegrep_tool_invocations_total`, `swegrep_tool_results_total`, `swegrep_cache_hits_total`, `swegrep_reward_score_bucket`, `swegrep_cycle_latency_ms_bucket`.
- To silence logs: set `RUST_LOG=off`. A future `SWE_GREP_DISABLE_TELEMETRY=true` toggle is planned; wire it in once shipped.

## Benchmarking Checklist
Use `scripts/bench_rg_vs_sweg.py` to guard regressions:
```bash
python scripts/bench_rg_vs_sweg.py \
  --repo /path/to/repo \
  --symbol login_user \
  --runs 20 \
  --output docs/benchmark-warm.json
```
- Compare `mean_ms` and `p95_ms` for `rg` vs `swe_grep`; the target is `swe_grep <= rg + 6 ms` for literal queries.
- Store benchmark artifacts under `docs/` so release reviewers can audit numbers.

## Session Log 2025-11-06
- Ran `cargo test` to cover `Improve Rust and Swift context retrieval` and cache deferral commits.
- Built `target/release/swe-grep` with `cargo build --release`.
- Verified Swift (`fixtures/multi_lang/App.swift:10`) and Rust (`fixtures/multi_lang/src/lib.rs:9`) hits include bodies and context hints without `--body`.
- Confirmed `--cache-dir` stays absent for misses and persists `state.json` for hits (`target/tmp-cache-empty`, `target/tmp-cache-populated/state.json`).
- Updated `docs/agent-use.md` to document auto body retrieval, context hints, and cache directory semantics.
