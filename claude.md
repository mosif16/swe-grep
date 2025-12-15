# SWE-Grep Agent Playbook

Use `swe-grep` for all code searches. Supports **Rust, Swift, TypeScript, TSX, JavaScript, Go, and Python**.

## The Killer Feature: `swe-grep context`

**One call = complete understanding.** Returns definition, callers, callees, types, imports, and tests.

```bash
# Full context bundle (definition + relationships + tests)
swe-grep context --symbol AuthService --path .

# With callers/callees explicitly enabled
swe-grep context --symbol run_cycle --callers --callees --path .

# Minimal output for token efficiency
swe-grep context --symbol Foo --concise --path .
# → {"symbol":"Foo","def":"...","callers":["file:line",...],"callees":[...],"tests":[]}

# Streaming NDJSON (one section per line)
swe-grep context --symbol Bar --stream --path .

# Presets for common workflows
swe-grep context --symbol Foo --preset refactor --path .   # callers + callees + types
swe-grep context --symbol Foo --preset debug --path .      # callers + body + tests
swe-grep context --symbol Foo --preset understand --path . # everything

# Budget control
swe-grep context --symbol Foo --max-lines 300 --path .
```

## Quick Start (Search Mode)

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
```

## Call Graph & Import Graph

```bash
# Call graph: find callers/callees
swe-grep refs --callers --symbol Foo::new --path .    # who calls Foo::new()?
swe-grep refs --callees --symbol run_cycle --path .   # what does run_cycle call?

# Import graph: what imports what
swe-grep refs --imports --symbol src/main.rs --path .       # what does this file import?
swe-grep refs --importers --symbol AuthService --path .     # who imports this symbol?
swe-grep refs --imports --import-depth 3 --symbol src/api.rs --path .  # transitive
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
| `--stream` | NDJSON streaming (one line per symbol/section) |
| `--progress` | Show `[1/N] Searching...` on stderr |
| `--definitions` | Extract function/struct signatures |
| `--format table` | Human-readable table output |
| `--json-pretty` | Pretty-print JSON |
| `--trim-body N` | Limit body to N lines (default: 50) |
| `--language rust` | Language hint (rust/swift/ts/tsx/go/python) |
| `--callers` | Find all call sites (refs/context) |
| `--callees` | Find functions called by symbol (refs/context) |
| `--types` | Include type relationships (context) |
| `--imports` | Show imports (refs/context) |
| `--importers` | Show importers (refs) |
| `--import-depth N` | Set traversal depth (refs) |
| `--preset X` | Use preset: refactor/debug/understand (context) |
| `--tests` | Include related test files (context) |

## Output Schema (v1)

### Search Output
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

### Context Output
```json
{
  "symbol": "AuthService",
  "definition": {
    "file": "src/auth.rs",
    "line": 42,
    "signature": "pub struct AuthService {"
  },
  "callers": [
    { "symbol": "handle_login", "file": "src/api.rs", "line": 89 }
  ],
  "callees": [
    { "symbol": "hash_password", "file": "src/crypto.rs", "line": 12 }
  ],
  "types": {
    "implements": ["Service", "Clone"],
    "extended_by": ["MockAuthService"],
    "contains": []
  },
  "imports": {
    "exported_from": ["src/lib.rs"],
    "imported_by": [{ "file": "src/main.rs", "line": 3 }]
  },
  "tests": [{ "file": "tests/auth_test.rs", "test_functions": ["test_login"] }],
  "summary": { "callers": 1, "callees": 1, "tests": 1 },
  "truncated": false
}
```

### Concise Context Output
```json
{"symbol":"Foo","def":"pub fn Foo()","callers":["src/main.rs:42"],"callees":["bar","baz"],"tests":["test_foo.rs"]}
```

## Language Support

| Language | Search | Call Graph | Type Graph | Import Graph |
|----------|--------|------------|------------|--------------|
| Rust | ✅ | ✅ | ✅ | ✅ |
| Swift | ✅ | ✅ | ✅ | ✅ |
| TypeScript/TSX | ✅ | ✅ | ✅ | ✅ |
| JavaScript | ✅ | ✅ | ✅ | ✅ |
| Go | ✅ | ✅ | ✅ | ✅ |
| Python | ✅ | ✅ | ✅ | ✅ |

## Rules
- Use `swe-grep context` for understanding code (one call = complete context)
- Use `swe-grep search --agent` for finding symbols
- Use `swe-grep refs` for call graph and import analysis
- Use `--concise` to minimize token usage
- Prefer `swe-grep` over `rg`/`grep` for code search

## Dev Commands

```bash
cargo build --release          # Build
cargo test -p swe-grep         # Run tests
./target/release/swe-grep --help
```
