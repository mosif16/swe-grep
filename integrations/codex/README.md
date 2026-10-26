# Codex node plugin

This package versions the node router, portable/compatibility manifests, bundled
skill and desktop launcher used by the local Codex deployment. It reuses the
harness backend and contracts. Routing is trusted operator configuration; model
calls must select a node and absolute path. Failed routes never fall back locally.
Context requires a directory and can request relationships with `direction`.
Use `exactSymbol: true` for complete literal identifiers, then carry a selected
search hit's `file`/`line` into context or references as `targetFile`/`targetLine`.
The selector file is relative to the requested analysis directory and cannot
escape that scope. Inspect returned `analysis` and `resolution` status rather
than interpreting every empty relationship array as absence. See the
[shared tool contract](../harness/README.md#tools-and-minimal-calls) for bounds.

## Build a local marketplace

Requires Node >=22.19, npm and a matching platform binary from this revision:

```sh
cargo build --release --locked -p swe-grep
node integrations/codex/package.mjs /absolute/path/to/target/release/swe-grep /absolute/path/to/routes.json /absolute/path/to/new-marketplace
```

The destination must not exist. The command creates a complete marketplace at
that destination, installs locked production dependencies with lifecycle scripts
and optional native peers disabled, and includes the binary. A failed dependency
installation retains its partial output for inspection; use a new destination
when retrying. It never changes installed plugins, shell startup files, SSH pins,
credentials or Codex settings. Outputs contain the supplied trusted route data;
keep them local and out of version control.

The route configuration has the following shape (replace the example identity):

```json
{
  "localNode": "my-node",
  "identity": {"account": "my-login", "hostnames": ["my-hostname"]},
  "routes": {}
}
```

The packager supplies a plugin-relative binary path. Optional `machineId` pins
`/etc/machine-id` on Linux. Optional routes map node names to an existing trusted
`command` and string `args` array that starts another copy of this MCP server.
Only use existing authorized routes with strict SSH host verification; no SSH
configuration is generated. Foreign node responses must identify the requested
node in `_swe.node`. No credentials or host-specific routes ship in this repo.

Register the resulting marketplace with
`codex plugin marketplace add /absolute/path/to/new-marketplace`, then select the
plugin in the desktop app and test in a fresh chat. The package can be copied into
a cache: its launcher resolves the server and binary relative to the installed
plugin. The Node runtime must remain available at its packaging-time path.
The `.cmd` launcher is supplied for Windows but Windows execution is not covered
by the Linux/macOS CI matrix.

For shell verification without registration:

```sh
node /absolute/path/to/new-marketplace/plugins/swe-grep/server.mjs --tool swe_search --args '{"node":"my-node","path":"/absolute/project","query":"my_symbol"}'
```

`--config /absolute/routes.json` selects alternate trusted config. `--local-only`
removes outgoing routes. Shell `--tool` calls default an omitted node to the local
node; MCP calls always require it explicitly.

## Verification

```sh
npm ci --ignore-scripts --prefix integrations/harness
SWE_GREP_BINARY=/absolute/path/to/target/debug/swe-grep npm test --prefix integrations/harness
npm audit --prefix integrations/harness
```

Tests exercise the real binary through official MCP stdio, package relocation,
local and routed results, wrong node identities, invalid paths and strict inputs.
Routes in tests run local fixture servers; they do not connect to real SSH nodes.
CI runs these tests on Linux and macOS with Node 22. The full dependency audit
includes native development peers. The `undici` and `zod` overrides address
advisories in those transitive dependencies while retaining the tested DSH SDK.

Plugin layout follows the [official packaging documentation](https://developers.openai.com/plugins/build/plugins).
