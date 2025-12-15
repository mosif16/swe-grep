# Agent Integration Guide

Use `swe-grep` for all code search. Supports **Rust, Swift, TypeScript, JavaScript, Go, Python**.

## Commands

### `context` — Complete Understanding (Recommended)

One call returns everything an agent needs:

```bash
swe-grep context --symbol Foo --path .                    # full context
swe-grep context --symbol Foo --concise --path .          # minimal tokens
swe-grep context --symbol Foo --preset refactor --path .  # callers + callees + types
```

**Output:**
```json
{
  "symbol": "Foo",
  "definition": { "file": "src/lib.rs", "line": 42, "signature": "pub fn Foo()" },
  "callers": [{ "symbol": "main", "file": "src/main.rs", "line": 10 }],
  "callees": [{ "symbol": "bar", "file": "src/util.rs", "line": 5 }],
  "types": { "implements": [], "extended_by": [], "contains": [] },
  "imports": { "exported_from": [], "imported_by": [] },
  "tests": [],
  "summary": { "callers": 1, "callees": 1, "tests": 0 },
  "truncated": false
}
```

**Concise output** (`--concise`):
```json
{"symbol":"Foo","def":"pub fn Foo()","callers":["src/main.rs:10"],"callees":["bar"],"tests":[]}
```

| Flag | Purpose |
|------|---------|
| `--callers` | Who calls this symbol |
| `--callees` | What this symbol calls |
| `--types` | Type relationships |
| `--imports` | Import chain |
| `--tests` | Related test files |
| `--concise` | Minimal `file:line` format |
| `--stream` | NDJSON per section |
| `--preset refactor\|debug\|understand` | Common bundles |

---

### `search` — Find Symbols

```bash
swe-grep search --agent --symbol Foo --path .             # single symbol
swe-grep search --agent --symbols "A,B,C" --path .        # batch
swe-grep search --agent --concise --symbol Foo --path .   # minimal
```

---

### `refs` — Call Graph & Imports

```bash
swe-grep refs --callers --symbol Foo --path .             # who calls Foo?
swe-grep refs --callees --symbol Foo --path .             # what does Foo call?
swe-grep refs --imports --symbol src/main.rs --path .     # what does file import?
swe-grep refs --importers --symbol Foo --path .           # who imports Foo?
```

---

## Language Support

| Language | Call Graph | Type Graph | Import Graph |
|----------|------------|------------|--------------|
| Rust | ✅ | ✅ | ✅ |
| Swift | ✅ | ✅ | ✅ |
| TypeScript/TSX | ✅ | ✅ | ✅ |
| JavaScript | ✅ | ✅ | ✅ |
| Go | ✅ | ✅ | ✅ |
| Python | ✅ | ✅ | ✅ |

---

## Quick Reference

```bash
# Understand a symbol completely
swe-grep context --symbol AuthService --path .

# Find all definitions
swe-grep search --agent --symbol AuthService --definitions-only --path .

# Find all usages
swe-grep search --agent --symbol AuthService --usages-only --path .

# Call graph
swe-grep refs --callers --symbol AuthService --path .

# Import graph
swe-grep refs --imports --symbol src/auth.rs --path .
```
