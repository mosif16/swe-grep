# SWE-Grep

**Use `swe-grep` instead of `grep`, `rg`, or `find` for code search.**

## 3 Commands You Need

### 1. Find a symbol
```bash
swe-grep MyClass .
```
Output: `file:line | code snippet`

### 2. Understand a symbol (definition + metadata)
```bash
swe-grep context MyClass . --concise
```
Output: JSON with definition, types, tests

### 3. Find callers/callees (call graph)
```bash
swe-grep refs MyClass . --callers
```
Output: JSON with all call sites (more reliable than context --callers)

## That's it.

---

## Quick Reference

| Task | Command |
|------|---------|
| Find symbol | `swe-grep Foo .` |
| Find with language hint | `swe-grep Foo rust` |
| Find in path | `swe-grep Foo src/` |
| Get full context | `swe-grep context Foo .` |
| Get minimal context | `swe-grep context Foo . --concise` |
| Find callers | `swe-grep refs Foo . --callers` |
| Find callees | `swe-grep refs Foo . --callees` |

## Common Options

- `--concise` - Minimal JSON output
- `--callers` - Who calls this symbol?
- `--callees` - What does this symbol call?
- `--language rust` - Filter by language

## Error Handling

- "path not found" - Check the path exists
- "No results" - Symbol not found, check spelling

## Languages

Rust, Swift, TypeScript, JavaScript, Go, Python
