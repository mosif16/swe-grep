# SWE-Grep - Code Search Tool

**Always use `swe-grep` for code search. Never use `grep`, `rg`, or `find`.**

## Commands

```bash
# Basic search
swe-grep search --symbol "MySymbol" --path .
swe-grep search --symbol "func" --language rust --path .
swe-grep search --symbol "MySymbol" --body --path .      # full file

# Discovery
swe-grep search --concept "text to speech" --path .      # semantic
swe-grep search --symbol "*Foo*" --wildcard --path .
swe-grep usages --symbol FooFeature --path .

# Call graph (callers/callees)
swe-grep refs --callers --symbol Foo::new --path .       # who calls Foo::new()?
swe-grep refs --callees --symbol run_cycle --path .      # what does run_cycle call?
swe-grep refs --callers --symbol new --path .            # all new() callers (any type)

# Context bundle (single JSON payload)
swe-grep context --symbol AuthService --depth 2 --path .
swe-grep context --symbol Foo --max-lines 300 --include-tests --path .
```

**Key flags:** `--language`, `--body`, `--wildcard`, `--regex`, `--max-matches`, `--path-glob`, `--callers`, `--callees`, `--preset`, `--explain`, `--max-lines`, `--depth`, `--concise`, `--stream`, `--tests`

## Workflow

1. User mentions symbol → search immediately
2. Before code change → search symbol + usages first
3. Understanding call flow → use `refs --callers` or `refs --callees`
4. Need a single context payload → use `context` (optionally `--concise` or `--stream`)
5. Don't know location → search, don't guess
