# Integration Guide

This document summarises how upstream coding agents can interact with
`swe-grep` programmatically and how to consume the structured telemetry
produced by the search cycle.

## HTTP workflow

`POST /search` accepts a JSON body mirroring the CLI flags. Minimal example:

```bash
curl -s http://localhost:8080/search \
  -H 'content-type: application/json' \
  -d '{
        "symbol": "login_user",
        "root": "fixtures/multi_lang",
        "timeout_secs": 4,
        "tool_flags": {"fd": true, "ast-grep": true}
      }'
```

Key fields in the response:

- `top_hits` – sorted by score (path, line, snippet, origin)
- `next_actions` – pre-canned follow-up suggestions for the caller
- `stage_stats` – latency and precision metrics per phase
- `reward` – accumulated reinforcement score for the cycle

The HTTP API surfaces health and metrics endpoints too:

- `GET /healthz`
- `GET /metrics` – Prometheus/OpenTelemetry counters

## gRPC workflow

The protobuf definition lives at `proto/swegrep.proto`. Example request using
`grpcurl`:

```bash
grpcurl -plaintext \
  -d '{"symbol":"login_user","root":"fixtures/multi_lang"}' \
  localhost:50051 swegrep.v1.SweGrepService/Search
```

The RPC returns the same `SearchSummary` structure as the CLI/HTTP path. Tool
flags can be toggled via the `tool_flags` map (e.g. `{ "ast-grep": false }`).

## Structured JSON logs

When `--log-dir` is specified, results are appended as JSON Lines to
`search.log.jsonl`. Each entry contains:

- `timestamp`
- tool configuration (`use_fd`, `use_ast_grep`, `use_index`, `use_rga`)
- `latency_ms`
- embedded `summary` payload identical to the API response

This format is ingestion-ready for analytics platforms such as BigQuery or
Splunk.

## Telemetry metrics

The `/metrics` endpoint exposes counters and histograms including:

- `swegrep_tool_invocations_total{tool="rg"}`
- `swegrep_tool_results_total{tool="fd"}`
- `swegrep_cache_hits_total{cache="symbol_hints"}`
- `swegrep_reward_score_bucket`
- `swegrep_cycle_latency_ms_bucket`

These metrics are generated via OpenTelemetry and can be scraped by Prometheus
or bridged to OTLP exporters.

## Feature toggles

All entry points honour the following switches:

- `--disable-fd` / `tool_flags: { "fd": false }`
- `--disable-ast-grep` / `tool_flags: { "ast-grep": false }`
- `--enable-rga` (requires the `rga` binary to be available on `PATH`)
- `--enable-index` (requires the `indexing` cargo feature and Tantivy indices)

Use these toggles to align the agent with resource-constrained environments or
to degrade gracefully when external binaries are unavailable.
