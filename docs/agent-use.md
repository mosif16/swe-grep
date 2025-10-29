# Agent Integration Guide

This document shows agent developers how to call `swe-grep`, interpret the
response, and adjust runtime knobs for latency-sensitive workflows.

## 1. Running the binary

```
swe-grep search --symbol <identifier> --path <repo-root>
```

- `--symbol` can be any identifier. Literal symbols (letters, digits, `_`) run
  through a fast path (~10 ms warm on fixtures) but still produce full context.
- Non-literal or mixed-case symbols trigger the full workflow (fd → rg →
  ast-grep). Expect ~25 ms warm in debug builds.

### Feature toggles

- `--disable-fd` – skip fd discovery; useful if `fd` is missing or for literal queries.
- `--disable-ast-grep` – skip structural validation when unneeded.
- `--enable-rga` – enable ripgrep-all fallback (requires `rga` on PATH).
- `--enable-index` – use Tantivy indices (build with `--features indexing`).
- `--context-before/--context-after` – request additional lines for each hit.
- `--body` – stream the full UTF-8 file for the surfaced hits (512 KiB guardrail).

## 2. Output contract

The CLI, HTTP, gRPC, and log outputs share the same JSON structure. A typical
response now includes enriched hit metadata:

```json
{
  "cycle": 1,
  "symbol": "login_user",
  "queries": ["login_user", "login_user User", "..."],
  "top_hits": [
    {
      "path": "src/lib.rs",
      "line": 18,
      "score": 1.2,
      "origin": "rg-scoped",
      "origin_label": "rg-scoped [rust]",
      "snippet": "fn login_user_allows_admin() {",
      "raw_snippet": "{\"type\":\"match\",...}",
      "snippet_length": 42,
      "raw_snippet_truncated": false,
      "expanded_snippet": "017 fn login_user_allows_admin() {\n018 ...",
      "context_start": 17,
      "context_end": 19,
      "body": "pub fn login_user(...",
      "body_retrieved": true
    }
  ],
  "deduped": 4,
  "next_actions": ["inspect src/lib.rs:18"],
  "stage_stats": {
    "discover_ms": 0,
    "probe_ms": 7,
    "disambiguate_ms": 0,
    "cycle_latency_ms": 7,
    "reward": 0.28
  },
  "reward": 0.28
}
```

Agents typically:

1. Use `top_hits` snippets for immediate context.
2. Follow `next_actions` to fetch additional files/lines. When `body_retrieved`
   is `true`, the full source is already embedded in the hit and can be cached.
3. Inspect `stage_stats` to detect degraded runs (e.g., non-zero `discover_ms`
   means fast path was bypassed).

## 3. HTTP/gRPC use

Start the service:

```
swe-grep serve --path <repo-root> --http-addr 0.0.0.0:8080 --grpc-addr 0.0.0.0:50051
```

HTTP example:

```
curl -X POST http://localhost:8080/search \
  -H 'content-type: application/json' \
  -d '{"symbol":"login_user","root":"/repo"}'
```

gRPC example (grpcurl):

```
grpcurl -plaintext \
  -d '{"symbol":"login_user","root":"/repo"}' \
  localhost:50051 swegrep.v1.SweGrepService/Search
```

Both return the same JSON summary as the CLI.

## 4. Performance guidance

- Literal queries benefit from the fast path (single `rg` union). Keep symbols
  simple (`foo_bar`) when possible.
- Disable AST (`--disable-ast-grep`) for bulk raw scans when structural context
  is unnecessary.
- For workloads requiring AST precision, expect ~6 ms overhead for the AST pass.
- Use release builds in production (`cargo build --release`); warm literal runs
  measure ~9–10 ms vs ~4 ms for raw `rg`.

## 5. Telemetry & logging

- `--log-dir DIR` writes JSON lines (`search.log.jsonl`) with the full summary
  plus metadata (`use_fd`, `use_ast_grep`, `latency_ms`).
- `/metrics` exposes Prometheus counters:
  - `swegrep_tool_invocations_total`
  - `swegrep_tool_results_total`
  - `swegrep_cache_hits_total`
  - `swegrep_reward_score_bucket`
  - `swegrep_cycle_latency_ms_bucket`

Disable telemetry via environment variables if needed:
- `RUST_LOG=off` (suppress tracing)
- `SWE_GREP_DISABLE_TELEMETRY=true` *(future toggle placeholder)*

## 6. Benchmarking

Use `scripts/bench_rg_vs_sweg.py` locally to evaluate regressions:

```
python scripts/bench_rg_vs_sweg.py \
  --repo /path/to/repo \
  --symbol login_user \
  --runs 20 \
  --output docs/benchmark-warm.json
```

Compare `mean_ms` and `p95_ms` for `rg` vs `swe_grep`. Production budgets target
`swe_grep` ≤ `rg` + ~6 ms for literal queries. Run this benchmark locally before
publishing a new release.
