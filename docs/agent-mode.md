# Agent Mode (`--agent`)

Agent mode shapes CLI output for programmatic consumption and bundles sensible defaults.

- Defaults: telemetry off, bodies fetched (first 50 lines by default), compact context, deterministic sorting.
- Exit codes: `0` success (hits or none), `1` usage/argument error, `2` runtime error.
- Noise control: data on stdout only; logs on stderr; `--quiet` silences logs entirely (tracing subscriber disabled in agent/quiet mode).
- Convenience: `--concise` drops context/origin/score/body, `--json-pretty` formats JSON, `--trim-body N` overrides the 50-line default, `--progress` emits `[idx/total] Searching <symbol>...` on stderr.

## Schema (version 1)

```json
{
  "version": 1,
  "cli_version": "0.1.0",
  "query": {"symbols": ["X"], "path": "/abs/repo"},
  "hits": [
    {
      "symbol": "X",
      "file": "src/lib.rs",
      "line": 42,
      "def": "fn X()",
      "snippet": "...",
      "context": "...",
      "body": "...",
      "body_truncated": false,
      "body_retrieved": true,
      "origin": "rg-scoped [rs]",
      "score": 1.2
    }
  ],
  "summary": {"count": 5, "latency_ms": 12},
  "errors": [],
  "warnings": [],
  "truncated": false
}
```

- `truncated` is `true` when hits reach `--max-matches`.
- `errors` carries per-symbol failures; `warnings` surfaces tool degradations.
- `cli_version` pins the emitting binary version.
- Bodies default to the first 50 lines in agent/streaming mode; override via `--trim-body` or drop them entirely with `--concise`.

## Batch inputs and streaming

```bash
# Inline comma-separated
swe-grep search --agent --symbols Foo,Bar --path .

# File input (one symbol per line)
swe-grep search --agent --symbols-file queries.txt --path .

# Stdin
printf "Foo\nBar\n" | swe-grep search --agent --stdin-symbols --path .

# NDJSON streaming (one object per symbol, last has done=true)
swe-grep search --agent --symbols Foo,Bar --stream --path .
# Add --progress to log batch status to stderr
swe-grep search --agent --symbols Foo,Bar --stream --progress --path .
```

## Output formatting and body control

- `--format json|table` (table is compact, json is jq-ready; streaming is always JSON).
- `--json-pretty` switches agent JSON to pretty-printing (non-streaming).
- `--concise` omits body/context/origin/score to keep payloads minimal.
- `--trim-body N` trims fetched bodies to the first N lines (defaults to 50 in agent/streaming mode).
- `--definitions` adds best-effort definition labels from context hints.
- `--context-before/--context-after` override the default 2/2 context; set both to 0 for no context.
