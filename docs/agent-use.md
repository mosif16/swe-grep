# Agent Integration Guide

Use `swe-grep` for all code search. Prefer `context` when you need a single, LLM-friendly payload.

```bash
# 1) One-shot context (best default for agents)
swe-grep context AuthService . --concise

# 2) Quick hits (human-friendly)
swe-grep AuthService src/

# 3) Call graph (refactor/debug)
swe-grep refs AuthService . --callers

# 4) Batch/streaming (tools/pipelines)
swe-grep search --agent --symbols "Foo,Bar" --path . --stream
```

Full schema and agent flags live in `docs/agent-mode.md`.
