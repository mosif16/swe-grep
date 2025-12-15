# SWE-Grep Agent Playbook

Use `swe-grep` for all code searches. Supports Rust, Swift, TypeScript, TSX, and JavaScript.

## Quick Start (Agent Mode)

```bash
# Single symbol search
swe-grep search --agent --symbol MyFunction --path .

# Batch search (multiple symbols)
swe-grep search --agent --symbols "Foo,Bar,Baz" --path .

# Minimal output (fastest for LLMs)
swe-grep search --agent --concise --symbol MyClass --path .

# Stream results as NDJSON
swe-grep search --agent --stream --symbols "A,B,C" --path .

# Restrict to a single file (useful for local/private symbols)
swe-grep search --agent --symbol deckHeaderSection --scope-file path/to/View.swift --path .

# Call graph: find callers/callees
swe-grep refs --callers --symbol Foo::new --path .    # who calls Foo::new()?
swe-grep refs --callees --symbol run_cycle --path .   # what does run_cycle call?

# Import graph: importers/imports (best-effort)
swe-grep refs --imports --symbol src/main.rs --path .       # what does this file import?
swe-grep refs --importers --symbol AuthService --path .     # who imports the file exporting this symbol?
swe-grep refs --imports --import-depth 3 --symbol src/api.rs --path .

# Context bundle: definition + callers/callees + types/imports + tests (best-effort)
swe-grep context --symbol AuthService --depth 2 --path .
swe-grep context --symbol Foo --max-lines 300 --include-tests --path .
```

## Agent Mode Defaults
- `--agent` enables: body retrieval, ±2 context lines, trim bodies to 50 lines, quiet stderr
- Output: structured JSON with `version`, `cli_version`, `hits[]`, `summary`, `errors`
- Exit codes: `0` success, `1` usage error, `2` runtime error

## Common Flags

| Flag | Purpose |
|------|---------|
| `--agent` | Enable agent mode with sensible defaults |
| `--concise` | Strip body/context/origin/score (minimal output) |
| `--symbols "A,B"` | Batch search multiple symbols |
| `--stream` | NDJSON streaming (one line per symbol) |
| `--progress` | Show `[1/N] Searching...` on stderr |
| `--definitions` | Extract function/struct signatures |
| `--format table` | Human-readable table output |
| `--json-pretty` | Pretty-print JSON |
| `--trim-body N` | Limit body to N lines (default: 50) |
| `--language rust` | Language hint (rust/swift/ts/tsx) |
| `--callers` | Find all call sites (refs command) |
| `--callees` | Find functions called by symbol (refs command) |
| `--imports` | Show direct/transitive imports (refs command) |
| `--importers` | Show direct/transitive importers (refs command) |
| `--import-depth N` | Set traversal depth (refs command) |

## Output Schema (v1)

```json
{
  "version": 1,
  "cli_version": "0.1.0",
  "query": { "symbols": ["Foo"], "path": "/repo" },
  "hits": [{
    "symbol": "Foo",
    "file": "src/lib.rs",
    "line": 42,
    "snippet": "pub fn Foo() {",
    "body": "...",
    "body_truncated": true
  }],
  "summary": { "count": 1, "latency_ms": 5 },
  "errors": [],
  "truncated": false
}
```

## Rules
- Always use `--agent` for structured output
- Use `--concise` to minimize token usage
- Clean up `.swe-grep-cache/` after runs
- Prefer `swe-grep` over `rg`/`grep` for code search

## Dev Commands

```bash
cargo build --release          # Build
cargo test                     # Run tests
./target/release/swe-grep --help
```
