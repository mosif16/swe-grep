# Context Extraction Enhancements

This document tracks the remaining work to ship snippet expansion and context extraction
so downstream agents can rely on richer snippets and whole-file retrieval.

## Snippet payloads

- [x] Extend `RipgrepMatch` in `crates/swe-grep-core/src/tools/rg.rs` to retain the raw `rg --json`
      payload (store it on `ripgrep::Match` and propagate through `SearchHit`) so
      `top_hits[].raw_snippet` can serialize the exact event.
- [x] Detect truncation introduced by `--max-columns 200` and surface
      `raw_snippet_truncated` plus `snippet_length` on `TopHit` (compare the byte
      length from the JSON event against the emitted text).
- [x] Add `expanded_snippet`, `context_start`, and `context_end` fields to `TopHit`
      by reading neighbouring lines from disk, formatting them with zero-padded line
      numbers, and binding the range metadata.
- [x] Plumb `--context-before` / `--context-after` through `SearchArgs` and the `rg`
      invocation while maintaining the current default of zero for backwards compatibility.

## Whole-body retrieval

- [x] Introduce a `--body` CLI flag (and proto toggle) that requests full-file payloads per hit.
- [x] Stream UTF-8 files when the flag is set, surface them via `body`, and emit a
      `body_retrieved` boolean to differentiate missing files from intentional skips.
- [x] Add size and timeout guardrails so large files cannot stall the search loop.

## AST warnings and fallbacks

- [ ] Capture parser diagnostics from `ast-grep` and expose them as `ast_warnings` in the summary.
- [ ] Increment `swegrep_fallback_total{kind="ast_literal"}` whenever we recover via a literal probe.
- [ ] Record parser errors as `swegrep_fallback_total{kind="ast_error"}` and convert the literal hits
      into `AstGrepMatch` entries to keep downstream scoring consistent.

## Telemetry

- [ ] Measure snippet expansion overhead with a `swegrep_snippet_expansion_ms` histogram.
- [ ] Track whole-body fetch outcomes with `swegrep_body_retrieval_total{status="success"|"failure"}`.
- [ ] Ensure latency, reward, and fallback counters remain in the cycle-level telemetry payload.

## Regression coverage

- [ ] Add async integration coverage for context expansion plus body retrieval using the
      `calculateBudgetRemaining` fixture.
- [ ] Cover AST literal fallback on generic symbols (`FetchDescriptor<Bill>`).
- [ ] Validate quoted symbol recovery paths (e.g. `"struct AppConfig"`).
