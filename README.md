# SWE-Grep

Code search and static symbol analysis for coding agents. SWE-Grep exposes the same Rust engine through a native DeepSeek Harness plugin, an MCP stdio server using the official SDK, a reusable adapter, and a CLI.

## Start with the harness plugin

Prerequisites: **Node.js 22.19 or newer** for the integration, and a **current stable Rust toolchain supporting edition 2024** to build the engine. Native integration is tested against **DSH 0.2.1-rc.2**; the MCP integration uses **SDK 1.32.1**.

From this checkout:

```sh
cargo build --release --locked -p swe-grep
npm ci --ignore-scripts --prefix integrations/harness
```

Install the local package through DSH's **Plugins → Install** flow, then configure its binary as the absolute path to the release executable and its root as your project directory. **Build the binary from this revision**: the adapter requires current CLI behavior, including --no-config. No binary is downloaded automatically.

Follow the [plugin installation guide](<integrations/harness/README.md>) for bundle activation, execution-world paths, MCP configuration, and adapter reuse. The package is local/unpublished; adding an npm dependency alone does not activate a DSH bundle.

| Tool | Use |
| --- | --- |
| swe_search | Find code locations with literal, regex, or raw-text search; use `exactSymbol` for complete identifiers and declaration/usage filters to narrow matches. |
| swe_context | Retrieve a definition in a directory; select a search hit with `targetFile`/`targetLine`, and request callers/callees with `direction`. Concise by default. |
| swe_refs | Trace callers/callees for a selected definition with scope-aware filters. |

For symbol investigation, search with `exactSymbol: true` and
`definitionsOnly: true`, then carry the chosen hit's `file` and declaration-start
`line` into context or references as `targetFile` and `targetLine`. Keep the same
analysis directory: `targetFile` is relative to it and cannot escape that scope.
Selecting a definition still allows callers from other files in the allowed scope.

Context defaults to a definition lookup; `direction: callers`, `callees`, or
`both` requests direct relationships. Inspect `resolution` for missing or ambiguous
definitions and `analysis` to distinguish `not_requested`, `complete`, and
`incomplete` sections. A completed static pass is not proof of exhaustive runtime
behavior. See the [agent workflow](<docs/agent-use.md>) for tool examples and
[JSON output](<docs/agent-mode.md>) for result shapes and limits.

Model calls cannot supply arbitrary CLI arguments or environment variables. The portable/native backend checks requested paths against its configured root; the Codex router permits readable paths on an explicitly selected node. Context rejects file paths. These controls complement, rather than replace, host execution policy. See [integration boundaries](<docs/integration.md>).

## Codex node plugin

The [Codex packaging guide](<integrations/codex/README.md>) builds a local marketplace with the routed MCP server, bundled skill and matching binary. The router and installed-package workflow are covered by the integration suite.

## Use the CLI

The following examples run from the repository root:

```sh
./target/release/swe-grep login_user fixtures/multi_lang
./target/release/swe-grep --no-config --disable-telemetry search login_user fixtures/multi_lang --exact-symbol --definitions-only --json-only --concise --quiet-hint
./target/release/swe-grep --no-config --disable-telemetry context login_user fixtures/multi_lang --target-file src/lib.rs --target-line 1 --concise --callers --callees --quiet-hint
./target/release/swe-grep --no-config --disable-telemetry refs login_user fixtures/multi_lang --target-file src/lib.rs --target-line 1 --callers --quiet-hint
```

Search finds matching locations; it is not exact symbol identity resolution. Context and references use best-effort static analysis, not a compiler or runtime trace. See [quickstart](<docs/quickstart.md>) and [capabilities and limits](<docs/context_extraction_todo.md>).

## Architecture

- [Rust core](<crates/swe-grep-core/Cargo.toml>): search pipeline, CLI, context, reference graphs, and cache management. Embedded text search has an external rg fallback; fd, ast-grep, and rga are optional helpers.
- [Harness integration](<integrations/harness/README.md>): native DSH tools, official-SDK MCP transport, and a reusable backend with injectable execution/filesystem services.
- [Optional indexer](<crates/swe-grep-indexer/Cargo.toml>): incremental Tantivy indexing, enabled with the core's indexing feature and CLI --enable-index flag; not part of the default core build.
- [Desktop consumer](<crates/swe-grep-desktop/Cargo.toml>): a separate application consuming the engine, not required for plugin or CLI use.

## Cache maintenance

Managed runtime caches are cleaned opportunistically, expiring unused data after
30 days and targeting a shared 512 MiB size budget. Active caches are protected by
shared/exclusive leases, so the size budget is a soft limit. Older unmanaged
caches, custom cache directories and index data are preserved. Preview or apply
cleanup explicitly:

```sh
./target/release/swe-grep cache gc
./target/release/swe-grep cache gc --apply
```

`cache gc` defaults to a dry run; `--max-age-days` and `--max-bytes` adjust its
retention limits. See [runtime cache guidance](<docs/performance-tuning.md>) for
managed data, concurrency and scope.

Development build artifacts have a separate, explicit cleanup command:

```sh
python3 scripts/build-cache-gc.py
python3 scripts/build-cache-gc.py --apply
```

The script defaults to a dry run and selects development artifacts by workspace
package, preserving release builds and dependency artifacts. Busy Cargo locks
cause a skip or timeout instead of cleanup during a build. See
[build cache cleanup](<docs/build-cache-cleanup.md>) for package selection and
verification. Build cleanup runs only when invoked.

## Contributing and maintenance

See [CONTRIBUTING.md](CONTRIBUTING.md) for authorized contributions and verification,
[MAINTAINING.md](MAINTAINING.md) for ongoing maintenance and packaging,
[SUPPORT.md](SUPPORT.md) for help, and [SECURITY.md](SECURITY.md) for private
vulnerability reporting. Participation expectations are in
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## Lifelong recursive improvement

SWE-Grep improves through a repeatable maintenance loop grounded in actual usage:

1. Observe a failed or costly search → context → references task and preserve a
   sanitized reproduction with the revision, scope, commands and expected result.
2. Turn the failure into a meaningful regression, then make a scoped fix.
3. Run the relevant [verification matrix](<CONTRIBUTING.md#make-and-verify-a-change>)
   and [comparative benchmarks](<docs/benchmark.md>) for the affected workflows.
   Verify correctness as well as latency, tool calls and output size; retain raw
   evidence and distinguish cold from warm runs.
4. Review the change and its evidence, update usage guidance, and repeat when new
   failures or measurements expose another concrete problem.

This cycle is human reviewed. It does not run indefinitely, modify its own code,
or learn from telemetry. Each verified change becomes the baseline for the next
improvement cycle. Workflow measurements support specific comparisons; broad
claims that agents prefer a tool require controlled agent evaluations. See
[maintenance](<MAINTAINING.md>) for review and rollback, and the
[Feedback guide](<Feedback/README.md>) for the separate experimental automation
and its limits. Feedback automation is not started by following this cycle.

## Documentation

Start with the [documentation map](docs/README.md), which distinguishes maintained
guides from historical plans and drafts.

- [Quickstart](<docs/quickstart.md>) — build, run, filter.
- [Agent workflow](<docs/agent-use.md>) and [JSON output](<docs/agent-mode.md>).
- [Integration](<docs/integration.md>) and [troubleshooting](<docs/troubleshooting.md>).
- [Cache and performance guidance](<docs/performance-tuning.md>), [measurement procedure](<docs/benchmark.md>), and [migration notes](<docs/changelog.md>).

## License

SWE-Grep is proprietary software owned by **Record and Learn LLC**. See the root [LICENSE](<LICENSE>) for governing terms. Licensing inquiries: [msayf@recordandlearn.info](mailto:msayf@recordandlearn.info).
