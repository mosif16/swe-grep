# Performance Tuning

Guidelines for keeping swe-grep responsive while preserving rich context.

## Modes

- **Fast literal search** – default. Works when `symbol` contains only
  `[A-Za-z0-9_]`.
  - Cost: single union `rg`. ~9 ms warm (debug build) vs 4 ms `rg` plain.
  - Recommendations: disable AST (`--disable-ast-grep`) if the calling agent
    relies solely on textual matches.

- **Full-context search** – triggered for non-literal symbols or when the fast
  path is skipped.
  - Pipeline: `fd` → multi-rewrite `rg` → optional Tantivy/rga → AST.
  - Warm latency (fixtures): ~20–25 ms with AST, ~12 ms without AST.

## Flags to adjust

| Flag | Default | Effect |
| --- | --- | --- |
| `--disable-fd` | `false` | Skip fd discovery; rely on hints or global scope. Helpful if `fd` missing. |
| `--disable-ast-grep` | `false` | Bypass structural validation; recommended for literal queries. |
| `--enable-rga` | `false` | Enable ripgrep-all fallback (adds ~8 ms when invoked). |
| `--enable-index` | `false` | Use Tantivy indices (requires `indexing` feature). |
| `--max-matches` | `20` | Cap matches retrieved; lowering reduces verification work. |
| `--timeout-secs` | `3` | Per-tool timeout; lower values cut runaway cost. |

## Telemetry

- `--log-dir` adds minimal cost but can be toggled off in latency-critical paths.
- Set `RUST_LOG=warn` to minimize tracing output.
- Prometheus counters (`/metrics`) are updated in-memory; scraping introduces no
  meaningful overhead.

## Benchmark budgets

- Warm literal queries: `swe-grep` should stay within `rg_mean_ms + 6 ms`.
- Full-context searches: target < 30 ms warm; monitor `stage_stats` to isolate
  spikes (fd, AST, index).

Use `scripts/bench_rg_vs_sweg.py` and `scripts/evaluate_bench.py` in CI to
enforce these budgets.
