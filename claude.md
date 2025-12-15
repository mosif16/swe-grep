# SWE-Grep Agent Playbook

Use `swe-grep` for all code searches. Supports **Rust, Swift, TypeScript, TSX, JavaScript, Go, and Python**.

## Simple Mode (Recommended)

```bash
swe-grep <symbol> [language|path]
```

**Examples:**
```bash
swe-grep AuthService                    # search current dir
swe-grep AuthService rust               # with language hint
swe-grep AuthService src/               # in specific path
```

**Output:** `file:line | code`
```
src/auth.rs:42 | pub struct AuthService {
src/api.rs:89 | let auth = AuthService::new(config);
```

## Advanced: `swe-grep context`

**One call = complete understanding.** Returns definition, callers, callees, types, imports, and tests.

```bash
swe-grep context AuthService .                              # full context
swe-grep context Foo . --explain                            # + fields/methods/complexity
swe-grep context Foo . --concise                            # minimal tokens
swe-grep context Foo . --preset refactor                    # callers + callees + types
```

## Quick Start (Search Mode)

```bash
# Single symbol search
swe-grep search MyFunction . --agent

# Batch search (multiple symbols)
swe-grep search --agent --symbols "Foo,Bar,Baz" --path .

# Minimal output (fastest for LLMs)
swe-grep search MyClass . --agent --concise

# Stream results as NDJSON
swe-grep search --agent --stream --symbols "A,B,C" --path .

# Restrict to a single file (useful for local/private symbols)
swe-grep search --agent --symbol deckHeaderSection --scope-file path/to/View.swift --path .
```

## Call Graph & Import Graph

```bash
# Call graph: find callers/callees
swe-grep refs Foo::new . --callers    # who calls Foo::new()?
swe-grep refs run_cycle . --callees   # what does run_cycle call?

# Import graph: what imports what
swe-grep refs src/main.rs . --imports                 # what does this file import?
swe-grep refs AuthService . --importers               # who imports this symbol?
swe-grep refs src/api.rs . --imports --import-depth 3 # transitive
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
| `--explain` | Enrich `context.definition` with symbol intelligence |
| `--tests` | Include related test files (context) |
| `--grouped` | Semantic grouping: definition → impls → usages → tests |
| `--budget N` | Approximate token budget for grouped output |
| `--fuzzy N` | Fuzzy threshold (0.0-1.0) for typo recovery |
| `--signatures` | Include full signatures in callers/callees (default: true) |

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
    "kind": "struct",
    "visibility": "pub",
    "signature": "pub struct AuthService {",
    "doc": "Authentication service (example).",
    "fields": ["config: AuthConfig", "repo: UserRepo"],
    "methods": ["new(config)", "login(user, password)", "logout(user)"],
    "implements": ["Service", "Clone"],
    "complexity": { "fields": 2, "methods": 3, "loc": 120 }
  },
  "callers": [
    { "symbol": "handle_login", "signature": "pub async fn handle_login(req: Request) -> Result<Response>", "call_context": "let auth = AuthService::new(config);", "file": "src/api.rs", "line": 89 }
  ],
  "callees": [
    { "symbol": "hash_password", "signature": "fn hash_password(pwd: &str) -> String", "call_context": "hash_password(&user.password)", "file": "src/crypto.rs", "line": 12 }
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

### Grouped Search Output (`--grouped`)
```json
{
  "version": 1,
  "results": [{
    "symbol": "AuthService",
    "grouped": {
      "definition": { "file": "src/auth.rs:42", "signature": "pub struct AuthService" },
      "implementations": [{ "file": "src/auth.rs:89", "trait": "Service" }],
      "usages_by_file": {
        "src/main.rs": [{ "line": 8, "context": "import" }, { "line": 45, "context": "method_call" }]
      },
      "test_coverage": ["tests/auth_test.rs"]
    },
    "summary": { "definition": 1, "impls": 1, "usages": 2, "tests": 1 }
  }]
}
```

### Fuzzy Match Output (zero hits)
```json
{
  "hits": [],
  "fuzzy_matches": [
    { "symbol": "AuthService", "score": 0.95, "file": "src/auth.rs:42" },
    { "symbol": "AuthServiceMock", "score": 0.82, "file": "tests/mock.rs:10" }
  ],
  "did_you_mean": "AuthService (exact match available)"
}
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
