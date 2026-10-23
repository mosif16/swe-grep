# Agent workflow

[Documentation home](<../README.md>) · [JSON output](<agent-mode.md>)

Prefer the [native plugin or MCP tools](<../integrations/harness/README.md>) over constructing shell commands when your host supports them.

1. **Locate** with swe_search. Use mode text for literal prose, regex for patterns, and literal for identifier-oriented search. Narrow path and pathGlob before raising limit (default 20, maximum 100).
2. **Inspect** with swe_context. Pass a directory; concise defaults to true. The adapter exposes definition context, not arbitrary CLI flags or every context relationship option.
3. **Trace** with swe_refs. Set direction to callers, callees, or both (default). Use language, exclude, pathGlob, excludeTests, and trackedOnly to bound graph results.
4. **Verify** the returned source before editing. Static edges, suggested symbols, related tests, and missing results are not proofs of runtime behavior or exhaustive coverage.

Example tool arguments (JSON, not shell commands):

```json
{"query":"login_user","path":"fixtures/multi_lang","mode":"literal","definitionsOnly":true,"limit":10}
```

```json
{"query":"login_user","path":"fixtures/multi_lang","direction":"both","excludeTests":true}
```

These paths assume the tool's configured workspace root is this checkout. Unknown keys are rejected. definitionsOnly and usagesOnly cannot both be true. Context accepts only query, path, and concise; use swe_refs for filtered relationship queries.

For CLI-only hosts:

```sh
swe-grep --no-config --disable-telemetry search login_user fixtures/multi_lang --json-only --concise --quiet-hint
swe-grep --no-config --disable-telemetry context login_user fixtures/multi_lang --concise --callers --callees --quiet-hint
swe-grep --no-config --disable-telemetry refs login_user fixtures/multi_lang --callers --callees --quiet-hint
```

These commands assume the current binary is on PATH. In a checkout, replace swe-grep with ./target/release/swe-grep. Inspect exit status, errors, warnings, and truncation rather than treating all empty arrays as successful complete analysis. See [migration notes](<changelog.md>).
