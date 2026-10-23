# JSON output for agents

[Documentation home](<../README.md>) · [Agent workflow](<agent-use.md>) · [Integration](<integration.md>)

Search, context, and refs have different JSON shapes. Do not apply the search envelope schema to context or refs. The examples below were captured from the release executable on this checkout's fixtures; absolute root and latency values are normalized. Optional fields depend on flags, results, and analysis state.

## Search envelope

Run from the repository root:

```sh
./target/release/swe-grep --no-config --disable-telemetry search login_user fixtures/multi_lang --json-only --concise --disable-fd --disable-ast-grep --top-hits 2 --quiet-hint
```

```json
{
  "version": 1,
  "cli_version": "0.1.0",
  "query": {"symbols": ["login_user"], "path": "/absolute/checkout/fixtures/multi_lang"},
  "hits": [
    {"symbol": "login_user", "file": "src/lib.rs", "line": 1, "snippet": "pub fn login_user(username: &str, password: &str) -> Option<String> {"},
    {"symbol": "login_user", "file": "src/lib.rs", "line": 18, "snippet": "fn login_user_allows_admin() {"}
  ],
  "summary": {"count": 2, "latency_ms": 0},
  "errors": [],
  "truncated": false
}
```

version identifies this search envelope; cli_version is the package version, not an exact source revision. hits use file/line/snippet, not the non-agent summary's top_hits structure. summary.count describes surfaced hits, not a guaranteed exhaustive repository count. In this example, the query also matches a longer identifier. A false truncated flag does not prove every possible rewrite or repository match was returned.

--json-only enables agent JSON output; --concise omits rich context/body/origin/score fields. Non-concise payloads may include warnings, suggestions, notes, and body metadata. Treat omitted fields as optional and check errors/warnings where present. --quiet-hint suppresses the instructional stderr banner; stdout and stderr are separate streams.

## Concise context

```sh
./target/release/swe-grep --no-config --disable-telemetry context login_user fixtures/multi_lang --concise --callers --callees --quiet-hint
```

```json
{
  "symbol": "login_user",
  "def": "pub fn login_user(username: &str, password: &str) -> Option<String> {",
  "def_location": "src/lib.rs:1",
  "callers": ["src/lib.rs:19"],
  "callees": ["Some", "to_string"],
  "tests": []
}
```

Without --callers/--callees those arrays can be empty because the sections were not requested. Concise caller locations and callee names are strings, unlike refs objects. Definition absence can omit def and def_location; clients should accept missing/null definition fields. Full context uses a definition object and richer sections; --body and --explain request additional definition detail. Do not assume a fixed rich-object field set from this concise example.

The harness context tool defaults to concise output and exposes no relationship flags. Use swe_refs for graph queries. Fact-extraction errors are command failures with no success JSON, not an empty valid context object.

## References

```sh
./target/release/swe-grep --no-config --disable-telemetry refs login_user fixtures/multi_lang --callers --callees --quiet-hint
```

```json
{
  "symbol": "login_user",
  "callers": [
    {"file": "src/lib.rs", "line": 19, "snippet": "assert_eq!(login_user(\"admin\", \"swe\"), Some(\"token-admin\".to_string()));", "origin": "callgraph"}
  ],
  "callees": [
    {"symbol": "to_string", "file": "src/lib.rs", "line": 3, "snippet": "Some(\"token-admin\".to_string())"}
  ],
  "notes": ["call graph: 1 callers, 1 callees (updated 6 / scanned 6)"]
}
```

Graph-cache counters in notes vary between runs; do not parse prose as a stable metrics API. Warnings may indicate incomplete graph extraction. Optional imports/importers and other fields depend on CLI flags and findings. Constructor/variant filtering differs from context: refs omits Rust Some/Ok-style constructors unless --include-constructors is requested.

## Batch, streaming, and grouped CLI output

These are CLI options, not model-facing tool arguments:

```sh
./target/release/swe-grep search --agent --symbols login_user,fetchUser --path fixtures/multi_lang --stream --quiet-hint
./target/release/swe-grep search login_user fixtures/multi_lang --agent --grouped --budget 2000 --quiet-hint
```

Streaming output is NDJSON, not a single JSON document. Grouped output is not the flat hits schema above; budgets are approximate, not tokenizer guarantees. Check search --help and context --help for available formats and section controls before selecting a parser.

## Failure handling

Check process status before accepting stdout. Search envelopes can also carry per-symbol errors. Context extraction failures must not be interpreted as absence; refs warnings can mean partial analysis. The backend rejects truncated/incomplete JSON and nonzero CLI status, and the MCP transport maps execution failures to isError. Do not depend on a single exit-code table across every CLI subcommand and validation path.
