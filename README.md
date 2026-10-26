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
| swe_search | Find code locations; literal, regex, or raw-text mode; declaration/usage filters. |
| swe_context | Retrieve a symbol definition in a directory; concise by default. |
| swe_refs | Find semantic callers/callees with scope-aware filters. |

Model calls cannot supply arbitrary CLI arguments or environment variables. The portable/native backend checks requested paths against its configured root; the Codex router permits readable paths on an explicitly selected node. Context rejects file paths. These controls complement, rather than replace, host execution policy. See [integration boundaries](<docs/integration.md>).

## Codex node plugin

The [Codex packaging guide](<integrations/codex/README.md>) builds a local marketplace with the routed MCP server, bundled skill and matching binary. The router and installed-package workflow are covered by the integration suite.

## Use the CLI

The following examples run from the repository root:

```sh
./target/release/swe-grep login_user fixtures/multi_lang
./target/release/swe-grep search login_user fixtures/multi_lang --json-only --concise
./target/release/swe-grep context login_user fixtures/multi_lang --concise --callers --callees
./target/release/swe-grep refs login_user fixtures/multi_lang --callers
```

Search finds matching locations; it is not exact symbol identity resolution. Context and references use best-effort static analysis, not a compiler or runtime trace. See [quickstart](<docs/quickstart.md>) and [capabilities and limits](<docs/context_extraction_todo.md>).

## Architecture

- [Rust core](<crates/swe-grep-core/Cargo.toml>): search pipeline, CLI, context, reference graphs, and cache management. Embedded text search has an external rg fallback; fd, ast-grep, and rga are optional helpers.
- [Harness integration](<integrations/harness/README.md>): native DSH tools, official-SDK MCP transport, and a reusable backend with injectable execution/filesystem services.
- [Optional indexer](<crates/swe-grep-indexer/Cargo.toml>): incremental Tantivy indexing, enabled with the core's indexing feature and CLI --enable-index flag; not part of the default core build.
- [Desktop consumer](<crates/swe-grep-desktop/Cargo.toml>): a separate application consuming the engine, not required for plugin or CLI use.

## Contributing and maintenance

See [CONTRIBUTING.md](CONTRIBUTING.md) for authorized contributions and verification,
[MAINTAINING.md](MAINTAINING.md) for ongoing maintenance and packaging,
[SUPPORT.md](SUPPORT.md) for help, and [SECURITY.md](SECURITY.md) for private
vulnerability reporting. Participation expectations are in
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## Documentation

Start with the [documentation map](docs/README.md), which distinguishes maintained
guides from historical plans and drafts.

- [Quickstart](<docs/quickstart.md>) — build, run, filter.
- [Agent workflow](<docs/agent-use.md>) and [JSON output](<docs/agent-mode.md>).
- [Integration](<docs/integration.md>) and [troubleshooting](<docs/troubleshooting.md>).
- [Cache and performance guidance](<docs/performance-tuning.md>), [measurement procedure](<docs/benchmark.md>), and [migration notes](<docs/changelog.md>).

## License

SWE-Grep is proprietary software owned by **Record and Learn LLC**. See the root [LICENSE](<LICENSE>) for governing terms. Licensing inquiries: [msayf@recordandlearn.info](mailto:msayf@recordandlearn.info).
