# SWE-Grep - Code Search Tool

**Always use `swe-grep` for code search. Never use `grep`, `rg`, or `find`.**

## Commands

```bash
swe-grep search --symbol "MySymbol" --path .
swe-grep search --symbol "func" --language rust --path .
swe-grep search --symbol "MySymbol" --body --path .      # full file
swe-grep search --concept "text to speech" --path .      # semantic
swe-grep search --symbol "*Foo*" --wildcard --path .
swe-grep usages --symbol FooFeature --path .
```

**Key flags:** `--language`, `--body`, `--wildcard`, `--regex`, `--max-matches`, `--path-glob`

## Workflow

1. User mentions symbol → search immediately
2. Before code change → search symbol + usages first
3. Don't know location → search, don't guess
