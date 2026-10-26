# Agent workflow

[Documentation home](<../README.md>) · [JSON output](<agent-mode.md>)

Prefer the [native plugin or MCP tools](<../integrations/harness/README.md>) over constructing shell commands when your host supports them.

1. **Locate** with swe_search. Use mode text for literal prose, regex for patterns, and literal for identifier-oriented search. Set exactSymbol to true when longer identifiers would be noise. Narrow path and pathGlob before raising limit (default 20, maximum 100).
2. **Inspect** with swe_context. Pass a directory; concise defaults to true and relationships are omitted unless direction is requested. If resolution is ambiguous, select a declaration with targetFile and targetLine from a search hit. Context also accepts language.
3. **Trace** with swe_refs. Set direction to callers, callees, or both (default). Use language, exclude, pathGlob, excludeTests, and trackedOnly to bound graph results.
4. **Verify** the returned source before editing. Static edges, suggested symbols, related tests, and missing results are not proofs of runtime behavior or exhaustive coverage.

Example tool arguments (JSON, not shell commands):

```json
{"query":"login_user","path":"fixtures/multi_lang","mode":"literal","exactSymbol":true,"definitionsOnly":true,"limit":10}
```

```json
{"query":"login_user","path":"fixtures/multi_lang","targetFile":"src/lib.rs","targetLine":1,"direction":"both"}
```

The second example works with swe_context or swe_refs. targetFile selects the declaration; it does not confine caller discovery to that file. targetLine is its 1-based declaration start line and requires targetFile. A missing selector match does not silently select another declaration. Use swe_refs for pathGlob, exclude, excludeTests, and trackedOnly relationship filters.

These paths assume the portable tool's configured workspace root is this checkout. The Codex node router additionally requires the authorized node and an absolute path on it. Unknown keys are rejected. definitionsOnly and usagesOnly cannot both be true; exactSymbol requires literal mode.

Inspect resolution, analysis, warnings, and truncated. Analysis status not_requested means a section was not analyzed; incomplete covers unresolved identity or omitted results. complete means the requested static pass finished within its scope and budgets, not compiler-verified or exhaustive runtime behavior. Duplicate declarations can prevent reliable caller attribution; those callers are omitted and marked incomplete when a declaration is selected. With no preset or explicit depth, context relationships are direct (depth 1).

For CLI-only hosts:

```sh
swe-grep --no-config --disable-telemetry search login_user fixtures/multi_lang --exact-symbol --json-only --concise --quiet-hint
swe-grep --no-config --disable-telemetry context login_user fixtures/multi_lang --target-file src/lib.rs --target-line 1 --concise --callers --callees --quiet-hint
swe-grep --no-config --disable-telemetry refs login_user fixtures/multi_lang --target-file src/lib.rs --target-line 1 --callers --callees --quiet-hint
```

These commands assume the current binary is on PATH. In a checkout, replace swe-grep with ./target/release/swe-grep. Inspect exit status, errors, warnings, and truncation rather than treating all empty arrays as successful complete analysis. See [migration notes](<changelog.md>).
