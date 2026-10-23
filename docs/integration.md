# Integration and trust boundaries

[Documentation home](<../README.md>) · [Plugin installation](<../integrations/harness/README.md>) · [JSON output](<agent-mode.md>)

## Supported integration paths

| Entry point | Role |
| --- | --- |
| Native DSH plugin | Registers swe_search, swe_context, and swe_refs with DSH's tools service; uses managed subprocess and filesystem services. Tested against 0.2.1-rc.2. |
| MCP stdio server | Exposes the same contracts using official MCP SDK 1.32.1. Requires Node.js 22+. No HTTP listener is needed. |
| Reusable adapter | SweGrepBackend plus shared contracts; other hosts supply their own registry and, for remote/sandboxed execution, runner and filesystem services. |
| CLI | Direct access to the Rust engine, including options intentionally omitted from tool contracts. |

The JavaScript package does not implement a second search engine. All tool calls execute a Rust binary built from this revision. See [installation and host examples](<../integrations/harness/README.md>); do not assume native DSH modules load unchanged into another host.

## Model-facing contracts

All tools require query (1–256 UTF-8 bytes). path defaults to the configured root. Search accepts language, mode (literal|regex|text), limit (1–100), exclude/pathGlob arrays, definitionsOnly, and usagesOnly. Context accepts only query, directory path, and concise. References accept language, direction (callers|callees|both), exclude/pathGlob, excludeTests, and trackedOnly.

The authoritative [contracts](<../integrations/harness/src/contracts.js>) reject unknown properties and invalid combinations. Filter arrays allow at most 20 nonempty patterns of at most 512 UTF-8 bytes each. Root, executable, environment, cache paths, arbitrary arguments, and timeouts are not model-controlled.

## Execution boundary

The backend resolves the configured root and target, checks containment, and rejects invalid or escaping paths. Local resolution uses real paths; native DSH resolution uses provider-owned filesystem identities. Context rejects file targets because CLI context can widen a file path to its parent directory. Search and refs can accept files.

Calls use argument arrays, not shell interpolation. They pass --no-config and --disable-telemetry, suppress the hint banner, and constrain output. Local execution keeps an allowlist of launch environment variables; native execution delegates to host policy. Git operations disable optional locks and fsmonitor hooks. This is not a claim that the standalone CLI is an OS sandbox or that a malicious repository is safe in every execution environment.

Default backend limits are 30 seconds and 2 MiB stdout. Cancellation and output overflow terminate managed execution; incomplete JSON is an error, not a successful partial object. Cache writes can occur even though these tools do not edit source files. Choose a trusted binary and enforce the host's filesystem/process policy.

## Results and failures

The reusable backend returns parsed CLI JSON; see [output shapes](<agent-mode.md>). The MCP server provides structuredContent plus JSON text content and reports execution failures with isError. Protocol parameter validation and native host errors are transport-specific. Clients should not parse stderr as search results.

Reference filters preserve semantic graph analysis and constrain reported origins/endpoints. Failed graph extraction may yield warnings about incomplete refs; it must not invent dependencies from unrelated functions. Context extraction errors fail the command without a success payload. See [capabilities and limits](<context_extraction_todo.md>).

## Other consumers

The [desktop application](<../crates/swe-grep-desktop/Cargo.toml>) is optional and not required to run the plugin. The CLI still exposes serve; it is a separate HTTP/gRPC service, not the MCP transport and not covered by the adapter's root-confinement contract. Use its --help and review the [service implementation](<../crates/swe-grep-core/src/service/server.rs>) before deployment; this guide does not recommend exposing it to untrusted networks.
