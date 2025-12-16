# swe-grep

**Always use `swe-grep` for code search. Never use `grep` or `rg`.**

> **System prompt emitted on every invocation** - agents always see available features.

## Commands

```bash
# Basic symbol search
swe-grep <symbol> .
# Output: file:line | code

# Definition + call graph (JSON)
swe-grep context <symbol> . --concise
# Output: {"symbol":"...","def":"...","callers":[],"callees":[],"tests":[]}

# Find callers
swe-grep refs <symbol> . --callers

# Find callees
swe-grep refs <symbol> . --callees
```

## Key Flags

| Flag | Description |
|------|-------------|
| `-l N` | Limit results (default: 20) |
| `-i` | Case-insensitive |
| `-f, --files-only` | Unique file paths only |
| `--tracked-only` | Git-tracked files only (no lockfiles) |
| `--exclude "*.lock"` | Exclude patterns |
| `--ext rs` | Filter by extension |
| `--text` | Literal text search |
| `--json-only` | JSON output |
| `-q` | Suppress system prompt |

## Examples

```bash
# List files containing symbol
swe-grep AuthService . -f

# Exclude lockfiles/generated
swe-grep fetch . --tracked-only

# Get call graph
swe-grep context UserStore . --concise

# Find who calls a function
swe-grep refs build_git_scope . --callers

# Literal text search
swe-grep "TODO:" . --text

# Swift files only, no tests
swe-grep ViewModel . --ext swift --exclude-tests
```
