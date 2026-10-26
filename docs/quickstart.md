# Quickstart

[Documentation home](<../README.md>) · [Integration](<integration.md>) · [Troubleshooting](<troubleshooting.md>)

## Build and connect

Use a current stable Rust toolchain supporting edition 2024. Node.js >=22.19 is required for the harness/MCP integration, not for direct CLI use.

```sh
cargo build --release --locked -p swe-grep
./target/release/swe-grep --help
npm ci --ignore-scripts --prefix integrations/harness
```

Use a binary built from this revision. Follow the [plugin guide](<../integrations/harness/README.md>) to install the native DSH bundle or configure an MCP client. Native DSH compatibility is tested at 0.2.1-rc.2; MCP uses the official SDK 1.32.1. Package installation does not download the Rust executable.

Basic text search runs in-process. External rg is a fallback; fd and ast-grep are optional discovery/structural helpers. rga is needed only for its opt-in fallback. The examples below do not require these optional discovery helpers.

## Find locations

Run from the checkout root:

```sh
./target/release/swe-grep login_user fixtures/multi_lang
./target/release/swe-grep login_user fixtures/multi_lang -C 2
./target/release/swe-grep search login_user fixtures/multi_lang --json-only --concise
./target/release/swe-grep search login_user fixtures/multi_lang --definitions-only --kind func --json-only
./target/release/swe-grep search 'login_user|fetchUser' fixtures/multi_lang --mode regex --json-only
./target/release/swe-grep search 'token-admin' fixtures/multi_lang --text --json-only
```

Default literal-symbol search may match longer identifiers too. --text requests raw fixed-string search without symbol rewrites; --mode regex interprets a regex. --definitions-only and --usages-only classify matching locations; they are mutually exclusive. --kind filters declaration kinds, not runtime value types.

For a file, pass its path to search, or use --scope-file relative to the root. Quote shell glob patterns:

```sh
./target/release/swe-grep search login_user fixtures/multi_lang --path-glob 'src/**/*.rs' --exclude-tests --json-only
```

## Understand a symbol

```sh
./target/release/swe-grep context login_user fixtures/multi_lang --concise --callers --callees
./target/release/swe-grep context login_user fixtures/multi_lang --body --explain
./target/release/swe-grep refs login_user fixtures/multi_lang --callers --callees
```

Context includes the definition; request relationships with flags such as --callers and --callees, or a --preset refactor|debug|understand bundle. An empty unrequested section is not evidence of no relationships. Use directories for context: the harness rejects files rather than widening to their parent directory.

References report usage edges. Do not pass --definitions-only or --kind to refs; use search for declarations. For filtered graph queries, use refs with --exclude, --path-glob, --scope-file, --exclude-tests, or Git scope flags. Filters retain allowed semantic edges rather than switching to textual guesses.

## Reproducible automation

Use global --no-config to ignore ambient user configuration and --disable-telemetry to disable exporters. --quiet-hint suppresses the instructional stderr banner; --json-only controls stdout, not every stderr message.

```sh
./target/release/swe-grep --no-config --disable-telemetry search login_user fixtures/multi_lang --json-only --concise --quiet-hint
```

See [actual JSON shapes](<agent-mode.md>), [analysis limits](<context_extraction_todo.md>), and [cache/index guidance](<performance-tuning.md>).
