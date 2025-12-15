# SWE-Grep - Code Search Tool

**Always use `swe-grep` for code search. Never use `grep`, `rg`, or `find`.**

## Commands

```bash
# Basic search (simple mode)
swe-grep MySymbol
swe-grep MySymbol rust
swe-grep MySymbol src/

# Discovery
swe-grep search --concept "text to speech" --path .      # semantic
swe-grep search "*Foo*" . --wildcard
swe-grep usages FooFeature .                             # usage graph

# Call graph (callers/callees)
swe-grep refs Foo::new . --callers                       # who calls Foo::new()?
swe-grep refs run_cycle . --callees                      # what does run_cycle call?
swe-grep refs new . --callers                            # all new() callers (any type)

# Context bundle (single JSON payload)
swe-grep context AuthService . --depth 2
swe-grep context Foo . --max-lines 300 --include-tests
```

**Key flags:** `--language`, `--body`, `--wildcard`, `--regex`, `--max-matches`, `--path-glob`, `--callers`, `--callees`, `--preset`, `--explain`, `--max-lines`, `--depth`, `--concise`, `--stream`, `--tests`

## Workflow

1. User mentions symbol → search immediately
2. Before code change → search symbol + usages first
3. Understanding call flow → use `refs --callers` or `refs --callees`
4. Need a single context payload → use `context` (optionally `--concise` or `--stream`)
5. Don't know location → search, don't guess
