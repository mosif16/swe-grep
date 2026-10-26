# SWE-Grep harness integration

Three code-navigation tools backed by the existing Rust SWE-Grep engine, available as a native **DeepSeek Harness (DSH) plugin**, an **MCP stdio server**, or a reusable JavaScript backend. There is no separate search engine or HTTP service.

This is a **local, unpublished npm package**. The npm tarball contains ready-to-run JavaScript and plugin assets, **not the SWE-Grep binary**. It does not download an executable or require an npm build/install lifecycle script.

## Requirements and supported integration paths

- **Node.js 22.19 or newer**. Building the binary also requires the repository’s Rust toolchain.
- A release binary built from the **same source revision** as this adapter. Every call uses the global `--no-config` option; an older installed binary may not support it. No SWE-Grep configuration file is required.
- Native adapter: **DSH 0.2.1-rc.2**, using its official tool, working-directory, filesystem, and subprocess SDK services.
- MCP adapter: the official **`@modelcontextprotocol/sdk` 1.32.1**, using its standard stdio transport. Native DSH peer packages are optional and are not required for portable backend/MCP production installs.

Validation includes 49 passing integration tests with a real binary and a verified production-only portable installation without native peers. These are tested configurations, not a promise that every MCP client, DSH version, operating system, or proprietary plugin API is compatible.

## Quickstart: build, pack, install in DSH

### 1. Build the executable and local tarball

Run from this repository’s root:

```sh
cargo build --release -p swe-grep
cd integrations/harness
npm ci --ignore-scripts
npm pack --ignore-scripts
```

`npm pack` prints the generated tarball filename. Keep its absolute path for the GUI installer. The release executable is under `target/release/swe-grep` (`swe-grep.exe` on Windows); keep its absolute path separately. A tarball installation installs the package’s production dependencies, not the Rust binary.

The binary and repository must be accessible **in the harness execution world**. For a remote/container provider, a host-machine path is not sufficient: build or place the matching binary where that provider executes it.

### 2. Install and select the bundle

1. Open the desired DSH profile in the Web GUI.
2. Open **Plugins → Install**, choose the generated local tarball, inspect the proposed bundle, and confirm installation.
3. Ensure the bundle is selected/enabled in that profile. Its patch registers `@recordandlearn/swe-grep-harness/dsh`, not the portable package-root export.
4. Configure the native row in the active profile’s `cordis.patch.yml`. In the tested DSH build, this plugin’s details page exposes enable/disable switches, not a generic configuration editor. Follow any restart instructions shown by DSH.

The default profile location is `~/.dsh/profiles/YOUR_PROFILE/cordis.patch.yml`; a configured `DSH_HOME` changes that base. Preserve existing entries and add this trusted override:

```yaml
- id: swe-grep-tools
  config:
    binary: /absolute/execution-world/path/to/swe-grep
    # Optional: omit root to use each calling agent’s current working directory.
    # root: /absolute/execution-world/path/to/project
    timeoutMs: 30000
    maxOutputBytes: 2097152
```

A row override replaces its configuration rather than deep-merging arbitrary fields. The active profile watcher applied this override during live verification; other deployments may require a restart.

`binary` accepts an absolute executable path or a trusted PATH command name (default: `swe-grep`); relative paths such as `./swe-grep` are rejected. An absolute path avoids PATH ambiguity.

`root` is optional. Without it, each call is bounded to the calling agent’s current working directory. A relative configured root resolves against that directory; an absolute root fixes the workspace boundary independently of agent directory changes. Model-provided `path` values resolve within the resulting root.

**Dependency installation is not bundle activation.** For manual profile management, the following only adds the npm dependency:

```sh
dsh plugin --profile YOUR_PROFILE add /absolute/path/to/package.tgz
```

Also select the package in that profile’s `dsh.profile.bundles` list and configure the native row. Prefer the GUI bundle installer unless managing profiles directly. Installation changes the chosen profile; these instructions do not assume anything about another user’s existing profile state. The bundle adds tools, not a replacement GUI or web server, and no hot-reload behavior is promised.

### 3. Verify tool availability

Confirm that the agent can discover `swe_search`, `swe_context`, and `swe_refs`. Start with a small search for a known identifier in your project. If tools are absent, check bundle selection before debugging the binary; if a call fails, use the troubleshooting table below.

## Tools and minimal calls

All three require `query`; `path` defaults to `.`. These are tool-name/argument examples, usable through either adapter, not raw shell commands:

```json
[
  {"name":"swe_search","arguments":{"query":"sample","path":".","limit":5}},
  {"name":"swe_context","arguments":{"query":"sample","path":".","concise":true}},
  {"name":"swe_refs","arguments":{"query":"sample","path":".","direction":"callers"}}
]
```

Submit each object as an individual call, not the entire array as a batch.

Replace `sample` with an identifier in your workspace. A successful search returns a JSON object containing bounded matches; context and references return semantic JSON from the Rust engine. No-match results are not evidence of a broken installation. The integration tests exercise these call shapes; exact results depend on the source and language.

| Tool | Use | Additional inputs |
| --- | --- | --- |
| `swe_search` | Locate code, declarations, usages, or text | `language`, `mode` (`literal`, `regex`, `text`), `limit`, `exclude`, `pathGlob`, `definitionsOnly`, `usagesOnly` |
| `swe_context` | Definition and related semantic context | `concise` (default `true`); `path` must be a directory |
| `swe_refs` | Semantic callers/callees with graph-preserving filters | `language`, `direction` (`callers`, `callees`, `both`), `exclude`, `pathGlob`, `excludeTests`, `trackedOnly` |

Search defaults to 20 matches; `limit` must be 1–100. Reference direction defaults to `both`. `definitionsOnly` and `usagesOnly` cannot both be true. Query length is 1–256 UTF-8 bytes (nonblank); paths are bounded to 4096 bytes. Each glob list accepts at most 20 nonempty strings, each at most 512 bytes. Unknown keys are rejected. Models cannot supply `env`, raw CLI arguments, binary paths, roots, timeouts, output caps, or cache locations.

## Use an MCP client instead

Install the local tarball in a directory managed for your MCP server. For a portable production-only installation, native peers can be omitted:

```sh
npm install --omit=dev --omit=optional --ignore-scripts /absolute/path/to/package.tgz
```

Configure the client to launch Node with the installed server entry. Adapt the enclosing keys to your client’s configuration format; replace all placeholder paths:

```json
{
  "mcpServers": {
    "swe-grep": {
      "command": "node",
      "args": [
        "/absolute/path/to/node_modules/@recordandlearn/swe-grep-harness/src/mcp.js",
        "--root", "/absolute/path/to/project",
        "--binary", "/absolute/path/to/swe-grep"
      ]
    }
  }
}
```

Use an absolute Node executable path too if your client does not inherit the expected PATH. npm also exposes the `swe-grep-mcp` bin command. `--root` must name an absolute existing directory; trusted server configuration may instead set `SWE_GREP_ROOT`. Optional `--timeout-ms` and `--max-output-bytes` set the same backend limits as native configuration.

The server communicates over **stdin/stdout only**; it opens no network listener. Stdout is reserved for MCP messages, and startup diagnostics go to stderr. The official SDK handles initialization, discovery, calls, and request cancellation. Results contain `structuredContent` and matching JSON text for clients that read text content. Backend failures set `isError: true`; unknown tools and invalid arguments produce protocol parameter errors. Disconnect, stdin EOF, SIGTERM, and SIGINT cancel active calls.

## Execution boundaries and failure behavior

- **Provider integration, not an OS sandbox.** The native adapter resolves canonical filesystem targets through DSH’s filesystem provider, obtains each agent’s directory through its working-directory service, and launches/collects managed children through its subprocess provider. It preserves opaque provider identities for containment checks. It does not bypass policies, grant permissions, request escalation, or create an additional sandbox. Isolation depends on the configured provider.
- **Bounded model authority.** Canonical containment rejects traversal and symlink escapes. Context rejects files rather than silently widening analysis to their parent directory. Arguments are constructed without a shell. These checks are not protection against malicious local processes racing filesystem changes; use trusted binaries, repositories, and host installations.
- **Controlled environment.** The portable local runner allowlists launch essentials (PATH, home, temp, locale, and cache location), excluding credentials, harness controls, loader injection, and Git-root overrides. Native DSH passes explicit overrides to the provider rather than copying the Node host environment; provider environment handling remains the host’s responsibility.
- **Predictable backend behavior.** Invocations disable config loading, telemetry, Git optional locks, and Git `core.fsmonitor` hooks. Search/reference calls disable external fd/AST-grep tools; semantic analysis remains parser-backed. Tools do not edit source files, but analysis caches/indexes may be written. Read-only annotations do not mean zero filesystem writes.
- **Deadline and output caps.** The backend defaults to a 30,000 ms deadline covering path resolution and execution, with a 2,097,152-byte stdout cap and bounded stderr. Truncated, malformed, non-object, or unsuccessful JSON is an error, never partial success. Nonzero exit status or engine-reported errors also fail the call. Cancellation waits for runner teardown rather than abandoning a live child; cleanup can extend elapsed time beyond the deadline.
- **Lifecycle limits.** POSIX local cancellation targets the process group and escalates; Windows local cancellation is direct-child-only. Native process-tree cleanup depends on the provider. Native tools use exclusive dispatch; MCP clients may issue concurrent calls, and cache/index lock failures remain explicit.

## Troubleshooting

| Symptom | Check / action |
| --- | --- |
| Package installed, tools absent | Select/enable the bundle in the correct profile; verify the native `/dsh` row and follow DSH restart instructions. Installing an npm dependency alone is insufficient. |
| `SWE_GREP_BINARY_NOT_FOUND` / execution failure | Set `binary` to an absolute executable path in the execution world; check permissions and provider access. The npm package does not include the binary. |
| Unknown `--no-config`, CLI failure, or invalid JSON | Rebuild with `cargo build --release -p swe-grep` from the adapter’s revision and update the configured path. Do not use a wrapper that prints non-JSON text to stdout. |
| `SWE_GREP_INVALID_PATH` / `SWE_GREP_PATH_OUTSIDE_ROOT` | Use an existing path inside the configured root. Check canonical symlink targets; give `swe_context` a directory, not a file. |
| `SWE_GREP_INVALID_INPUT` / MCP invalid parameters | Use only the documented tool keys and bounds; do not pass CLI flags, `env`, or configuration as model arguments. |
| `SWE_GREP_TIMEOUT` / `SWE_GREP_OUTPUT_TRUNCATED` | Narrow the path/query, reduce search `limit`, or use concise context. A trusted operator may raise the configured limits when appropriate. |
| `SWE_GREP_ABORTED` | The request or transport was cancelled; retry only if the call is still needed. |
| MCP startup exits or protocol fails | Check Node >=22, installed production dependencies, absolute root and server paths, and stderr. Do not add logs to protocol stdout. |
| Empty or limited semantic results | Check the identifier, path, language, and filters; the adapter does not add language/graph support beyond the Rust engine. |

## Reuse the backend in another native host

The package root exports `SweGrepBackend`; `/contracts` exports the shared `TOOLS` schemas and validation. These portable entry points do not import native DSH peers. The implementation is shared JavaScript, not a generated TypeScript build. The native `/dsh` adapter uses the actual SDK’s `defineTool`, canonical JSON Schema validation, and structured output definitions.

```js
import { SweGrepBackend } from '@recordandlearn/swe-grep-harness';
import { TOOLS } from '@recordandlearn/swe-grep-harness/contracts';

const backend = new SweGrepBackend({
  root: '/absolute/path/to/project',
  binary: '/absolute/path/to/swe-grep',
});
const result = await backend.call(TOOLS[0].name, { query: 'sample', limit: 5 });
```

Register those contracts using your host’s real registry API and forward its cancellation signal with `backend.call(name, args, { cwd, signal })`. This is an adapter seam, not universal native-plugin compatibility.

For remote/policy-aware execution, inject **both** `runner` and `filesystem` into the constructor instead of retaining local defaults:

- `runner({ argv, cwd, env, signal, maxOutputBytes })` must honor cancellation and settle only after cleanup. Return `{ exitCode, signal, stdout, stderr, stdoutTruncated, stderrTruncated }`. Treat `env` as controlled overrides and apply the host’s environment policy.
- `filesystem.resolve(value, cwd, signal)` returns a canonical `{ path, kind, ...opaqueIdentity }` (`kind`: `file` or `directory`); `filesystem.contains(root, target)` checks canonical containment. Keep provider identities intact and expose a process-usable path, not a display-only path.

The native adapter demonstrates these seams without substituting local Node filesystem/process operations for DSH services.

## Development and verification

From the integration package directory:

```sh
npm ci --ignore-scripts
npm run check
npm test
SWE_GREP_BINARY=/absolute/path/to/current/release/swe-grep npm test
npm pack --dry-run --ignore-scripts
```

The `SWE_GREP_BINARY` command enables real CLI and official MCP stdio tests; without it, those tests explicitly skip. Tests cover strict contracts, actual DSH SDK definitions with provider mocks, official MCP clients/transports, path escapes, injection, output limits, cancellation/cleanup, packaged exports, environment scrubbing, and Git metadata safety. SDK/provider-mock tests are not a substitute for checking a particular deployed provider and executable configuration.

## License

SWE-Grep is proprietary software owned by **Record and Learn LLC**. See the [packaged LICENSE](<./LICENSE>) for the governing terms. Installation instructions and compatibility statements do not grant additional rights.
