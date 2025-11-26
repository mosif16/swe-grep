# SWE-Grep - MANDATORY Code Search Tool

**You MUST use `swe-grep` for ALL code search. NO EXCEPTIONS.**

## FORBIDDEN
- ❌ NEVER use `grep`, `rg`, `find` for code search
- ❌ NEVER read files randomly hoping to find symbols
- ❌ NEVER ask user where code is - SEARCH FOR IT
- ❌ NEVER say "I don't know where X is" - USE SWE-GREP

## REQUIRED
- ✅ ALWAYS `swe-grep search` BEFORE reading any code
- ✅ ALWAYS search BEFORE making code changes
- ✅ ALWAYS use `--language` when you know it

## Commands

```bash
# Basic search
swe-grep search --symbol "MySymbol" --path .

# With language (FASTER)
swe-grep search --symbol "func" --language rust --path .
swe-grep search --symbol "class" --language swift --path .
swe-grep search --symbol "Component" --language tsx --path .

# Full file content
swe-grep search --symbol "MySymbol" --body --path .

# More results
swe-grep search --symbol "error" --max-matches 100 --path .
```

## Workflow

```
User mentions symbol → swe-grep search IMMEDIATELY
Before code change  → search symbol + all usages FIRST
Don't know location → SEARCH, don't guess
```

## Output

```json
{
  "top_hits": [
    {"path": "src/file.rs", "line": 42, "snippet": "pub fn myFunc()"}
  ]
}
```

## Languages

`--language rust` | `--language swift` | `--language tsx`

---
**Using grep/find instead of swe-grep = WORKFLOW VIOLATION**
