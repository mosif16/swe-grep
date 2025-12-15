# Agent Mode (`--agent`)

Agent mode shapes CLI output for programmatic consumption and bundles sensible defaults.

- Defaults: telemetry off, bodies fetched (first 50 lines by default), compact context, deterministic sorting.
- Exit codes: `0` success (hits or none), `1` usage/argument error, `2` runtime error.
- Noise control: data on stdout only; logs on stderr; `--quiet` silences logs entirely (tracing subscriber disabled in agent/quiet mode). Optional tools (`fd`, `ast-grep`) auto-disable after a single “command not found” and stop emitting user-facing warnings; real runtime errors still appear in `warnings`.
- Convenience: `--json-only` implies `--agent --format json --quiet` for pipeline-safe output; `--concise` drops context/origin/score/body; `--grouped` buckets results to reduce redundant context; `--budget N` truncates grouped output to an approximate token budget; `--fuzzy N` (0.0-1.0) enables fuzzy matching with did-you-mean suggestions; `--explain` enriches context definitions with fields/methods/complexity; `--json-pretty` formats JSON; `--trim-body N` overrides the 50-line default; `--full-body` disables body trimming; `--body-limit BYTES` / `--no-truncate` control max body bytes; `--progress` emits `[idx/total] Searching <symbol>...` on stderr.
- Strings/paths: `--text` treats `--symbol` as raw literal text (no rewrites/AST); `--string` generates quoted variants for literal string lookups; `--path-glob "src/**/*.tsx"` narrows scope without forcing a language hint.

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
  "related_symbols": [],
  "suggested_symbols": [],
  "fuzzy_matches": [{"symbol": "AuthService", "score": 0.95, "file": "src/auth.rs:42"}],
  "did_you_mean": "AuthService (exact match available)",
  "related_paths": [],
  "notes": [],
  "fd_candidates": [],
  "ast_fallback": false,
  "truncated": false
}
```

- `truncated` is `true` when hits reach `--max-matches`.
- `errors` carries per-symbol failures; `warnings` surfaces tool degradations.
- `cli_version` pins the emitting binary version.
- Bodies default to the first 50 lines in agent/streaming mode; override via `--trim-body` or drop them entirely with `--concise`.
- When there are **zero hits**, swe-grep populates `fuzzy_matches` + `did_you_mean` to help agents recover from casing/qualification mismatches (e.g. `authservice` → `AuthService`, `::new` → `Foo::new`). Control the threshold with `--fuzzy 0.0..=1.0` (default `0.7`).

## Context output (`swe-grep context --explain`)

The `context` command with `--explain` returns symbol intelligence:

```json
{
  "symbol": "SearchEngine",
  "definition": {
    "file": "src/search/engine.rs",
    "line": 65,
    "kind": "struct",
    "visibility": "pub(crate)",
    "signature": "pub(crate) struct SearchEngine {",
    "fields": ["config: SearchConfig", "rg_tool: RipgrepTool"],
    "methods": ["new(config)", "run_cycle(self)", "verify(self, hits)"],
    "complexity": { "fields": 19, "methods": 37, "loc": 1772 }
  },
  "callers": [
    {
      "symbol": "execute",
      "signature": "pub async fn execute(&self, request: SearchInput) -> Result<SearchSummary>",
      "call_context": "engine.run_cycle().await",
      "file": "src/search/mod.rs",
      "line": 32
    }
  ],
  "callees": [...],
  "summary": { "callers": 10, "callees": 5, "tests": 2 }
}
```

Key fields in `--explain` mode:
- `kind`: `struct`, `fn`, `class`, `trait`, `enum`, `interface`, `type`
- `visibility`: `pub`, `pub(crate)`, `private`, `internal`, `protected`
- `fields`: Array of `"name: Type"` strings for structs/classes
- `methods`: Array of `"name(params)"` signatures
- `complexity`: `{ fields, methods, loc }` counts

## Grouped output (`--grouped`)

Grouped output reduces repeated context by emitting a per-symbol object that buckets hits into: `definition`, `implementations`, `usages_by_file`, and `test_coverage`.

- `--grouped` currently supports `--format json` only.
- `--budget N` is an approximate token budget (based on JSON size) and truncates usages/impls/tests when exceeded.

```json
{
  "version": 1,
  "cli_version": "0.1.0",
  "query": {"symbols": ["AuthService"], "path": "/abs/repo"},
  "results": [
    {
      "symbol": "AuthService",
      "grouped": {
        "definition": {"file": "src/auth.rs:42", "signature": "pub struct AuthService"},
        "implementations": [{"file": "src/auth.rs:89", "trait": "Service", "methods": ["handle"]}],
        "usages_by_file": {"src/main.rs": [{"line": 8, "context": "import"}]},
        "test_coverage": ["tests/auth_test.rs"]
      },
      "summary": {"definition": 1, "impls": 1, "usages": 1, "tests": 1},
      "truncated": false
    }
  ],
  "summary": {"count": 1, "latency_ms": 12},
  "errors": [],
  "warnings": [],
  "related_symbols": [],
  "suggested_symbols": [],
  "related_paths": [],
  "notes": [],
  "fd_candidates": [],
  "concept": null,
  "concept_expansion": null,
  "ast_fallback": false,
  "truncated": false
}
```

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

# NDJSON streaming (grouped per symbol)
swe-grep search --agent --symbols Foo,Bar --stream --grouped --budget 2000 --path .
```

## Output formatting and body control

- `--format json|json-lines|table|compact|tree|edit|minimal|summary|markdown|csv` (streaming is always JSON).
- `--json-pretty` switches agent JSON to pretty-printing (non-streaming).
- `--concise` omits body/context/origin/score to keep payloads minimal.
- `--trim-body N` trims fetched bodies to the first N lines (defaults to 50 in agent/streaming mode).
- `--definitions` adds best-effort definition labels from context hints.
- `--context-before/--context-after` override the default 2/2 context; set both to 0 for no context.
