# Troubleshooting

[Documentation home](<../README.md>) · [Plugin setup](<../integrations/harness/README.md>)

## Tools do not appear in DSH

Installing an npm dependency is not the same as activating the plugin bundle. Use the GUI bundle installer and verify the native module is selected in the intended profile. Follow any restart instruction from the host. Native compatibility is tested at DSH 0.2.1-rc.2; do not assume every release or proprietary host loads the same module.

## Binary missing or --no-config rejected

Build with cargo build --release -p swe-grep from this revision. Configure an absolute binary path visible in the host's execution world, or ensure the correct executable is on that world's PATH. The package does not fetch a binary. A host-local executable path need not exist inside a remote/container execution provider.

## Path rejected

Targets must exist and resolve within the configured root. Check symlinks, the calling agent's working directory, and provider paths. Context requires a directory; for file-scoped locations use search, or refs for dependencies. Do not work around confinement by passing raw arguments: those are not accepted.

## Empty or surprising results

- Verify mode: literal identifier-oriented search, regex pattern search, and raw --text search differ.
- Check path, language, include/exclude patterns, test exclusions, ignore rules, and Git scope. Quote shell globs.
- Literal symbol queries can match longer identifiers. Declaration/usage filters are not compiler identity resolution.
- Context relationships require section flags or presets. The adapter's context tool exposes only query/path/concise; use swe_refs for callers and callees.
- Review warnings and truncation before assuming absence. Static analysis can be incomplete; see [limits](<context_extraction_todo.md>).

## refs rejects declaration filters

--definitions-only and --kind do not apply to usage edges. Move declaration lookup to search. --usages-only is accepted, but does not turn refs into a list of every textual occurrence.

## Context fails before JSON output

A diagnostic containing failed to extract repository facts indicates extraction failure, not a missing symbol. Check the named source file for invalid UTF-8 and check cache-directory permissions. An unrelated invalid source file in the analyzed directory can fail context too. This applies to concise, grouped, and streaming modes. See [migration notes](<changelog.md>).

## Optional tools missing

Basic search uses an embedded engine; external rg is a fallback. fd and ast-grep can be disabled with --disable-fd and --disable-ast-grep. --enable-rga requires rga. --mode ast explicitly asks for the external AST path; it is not the same as ordinary search or tree-sitter graph analysis. Avoid enabling optional paths unless their dependencies are installed.

## Timeouts, large output, and caches

The adapter defaults to a 30-second request deadline and 2 MiB stdout cap. Narrow scope and reduce search limit; prefer concise results over full bodies. Limits are trusted host configuration, not tool-call arguments. Output overflow or incomplete JSON is an error.

For direct CLI use, --timeout-secs applies per tool invocation. --cache-dir is available on search/refs, not context. Ensure the normal context cache location is writable. Do not assume cache persistence errors are always ignored. See [cache and index guidance](<performance-tuning.md>) before using --clear-cache, which deletes cache/index state.

## MCP client cannot connect

Use the Node.js >=22.19 stdio command and absolute root/binary paths from the [plugin guide](<../integrations/harness/README.md>). MCP does not connect to the CLI serve HTTP/gRPC ports. Stdout is protocol-only; inspect stderr for startup diagnostics. Confirm dependencies were installed and the executable is current.
