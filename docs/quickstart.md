# SWE-Grep Quickstart
#
# This guide walks through installing the required search tooling, running the
# CLI, and launching the new HTTP/gRPC service. Pick the workflow that matches
# your environment.

## 1. Prerequisites

- **Rust** 1.80 or newer (required for building the binary and helper tools)
- **fd** (binary name `fd` or `fdfind`)
- **ripgrep** (`rg`)
- **ast-grep** (`sg`) – optional but recommended for structural disambiguation
- **ripgrep-all** (`rga`) – optional docs/config fallback

The repository ships with `scripts/install-tools.sh`, which installs
`ast-grep` and `ripgrep_all` via `cargo install`:

```bash
./scripts/install-tools.sh
```

The script honours `AST_GREP_VERSION` and `RGA_VERSION` environment variables
if you need a specific release.

On Debian/Ubuntu hosts you can install `fd` and `rg` directly:

```bash
sudo apt-get update
sudo apt-get install -y fd-find ripgrep
sudo ln -s /usr/bin/fdfind /usr/local/bin/fd
```

Ensure `$HOME/.cargo/bin` is on your `PATH` so that `sg` and `rga` are
discoverable by the CLI.

## 2. CLI usage

Run a search against the fixtures to verify everything is wired correctly:

```bash
RUST_LOG=error cargo run -- search login_user fixtures/multi_lang
```

Simple mode ergonomics (context lines, JSON output, and banner control):

```bash
cargo run -p swe-grep -- login_user fixtures/multi_lang -C 2
cargo run -p swe-grep -- login_user fixtures/multi_lang --json-only
cargo run -p swe-grep -- login_user fixtures/multi_lang --no-banner
SWE_GREP_QUIET_HINT=1 cargo run -p swe-grep -- login_user fixtures/multi_lang
```

Agent-friendly grouped output (definition/impl/usages/tests) for token-sensitive workflows:

```bash
cargo run -p swe-grep -- search login_user fixtures/multi_lang \
  --agent --grouped --budget 2000
```

Assemble a context bundle (definition + callers/callees + types/tests) for the same symbol:

```bash
cargo run -p swe-grep -- context login_user fixtures/multi_lang
```

Token-efficient output for LLM prompts:

```bash
cargo run -p swe-grep -- context login_user fixtures/multi_lang --concise
```

Grouped context output with token budgeting:

```bash
cargo run -p swe-grep -- context login_user fixtures/multi_lang --grouped --budget 2000
```

NDJSON streaming (useful for UIs/pipelines):

```bash
cargo run -p swe-grep -- context login_user fixtures/multi_lang --stream
```

Symbol intelligence (enrich the definition with fields/methods/doc/complexity):

```bash
cargo run -p swe-grep -- context login_user fixtures/multi_lang --explain
```

Presets (can be overridden with flags like `--tests` / `--body`):

```bash
cargo run -p swe-grep -- context login_user fixtures/multi_lang --preset refactor
cargo run -p swe-grep -- context login_user fixtures/multi_lang --preset debug
cargo run -p swe-grep -- context login_user fixtures/multi_lang --preset understand
```

### Collecting tool feedback (shell_command)

`swe-grep` can generate a “ask the agent” prompt for the `shell_command` tool, then persist the response as structured JSON under `<cwd>/Feedback` (when `--cwd` is provided).

```bash
cargo run -p swe-grep -- feedback shell --command "ls -la" --prompt-only
```

To save feedback, pipe either JSON (preferred) or plain text:

```bash
echo '{"improvements":"Add structured stderr + stable truncation.","rating":4,"tags":["errors","ux"]}' \
  | cargo run -p swe-grep -- feedback shell --command "ls -la" --cwd "$(pwd)" --exit-code 0 --duration-ms 12
```

Example summary (truncated):

```json
{
  "cycle": 1,
  "symbol": "login_user",
  "queries": [
    "login_user",
    "login_user User",
    "login_user error",
    "User.login_user"
  ],
  "top_hits": [
    {
      "path": "src/lib.rs",
      "line": 1,
      "score": 1.2,
      "origin": "rg-scoped",
      "snippet": "pub fn login_user(username: &str, password: &str) -> Option<String> {"
    }
  ],
  "stage_stats": {
    "cycle_latency_ms": 31,
    "precision": 0.0,
    "density": 0.67,
    "reward": 0.28
  }
}
```

Enable or disable tools per search:

```bash
cargo run -- search fetch_user . \
  --disable-ast-grep --disable-fd \
  --enable-rga
```

Structured JSON logs are emitted to stdout and, when `--log-dir` is provided,
appended to `search.log.jsonl`.

## 3. Serving the API

Launch the combined HTTP (default `:8080`) and gRPC (`:50051`) services:

```bash
cargo run -- serve \
  --path $(pwd) \
  --http-addr 0.0.0.0:8080 \
  --grpc-addr 0.0.0.0:50051 \
  --log-dir ./tmp/logs
```

### HTTP quick check

```bash
curl -X POST http://localhost:8080/search \
  -H 'content-type: application/json' \
  -d '{"symbol":"login_user","path":"fixtures/multi_lang"}'
```

Metrics are exposed at `http://localhost:8080/metrics` (Prometheus text
format).

### gRPC with `grpcurl`

```bash
grpcurl -plaintext \
  -d '{"symbol":"login_user","root":"fixtures/multi_lang"}' \
  localhost:50051 swegrep.v1.SweGrepService/Search
```

The protobuf schema is generated from `proto/swegrep.proto` and included in the
crate under `swe_grep::service::proto`.

## 4. Using the distributed binary

If you’re consuming a prebuilt release, place the `swe-grep` binary on your PATH
and run the same commands as above:

```bash
./swe-grep serve --path /path/to/repo --http-addr 0.0.0.0:8080
```

Configuration flags match the CLI options listed earlier; no additional config
files are required.

## 5. Language support

swe-grep supports analysis for 6 languages with full call graph, type graph, and import graph:

| Language | File Extensions |
|----------|-----------------|
| Rust | `.rs` |
| Swift | `.swift` |
| TypeScript | `.ts`, `.tsx` |
| JavaScript | `.js`, `.jsx` |
| Go | `.go` |
| Python | `.py` |

Cross-language call graph example (finds callers across all languages):

```bash
cargo run -p swe-grep -- context helper fixtures/multi_lang --callers
# Returns callers from both Python (Job::run) and Go (File::Use)
```

## 6. Next steps

- Integrate the HTTP service with upstream agents (see `docs/integration.md`)
- Enable Prometheus scraping by pointing at `/metrics`
- Tail structured logs from `search.log.jsonl` to feed downstream analytics
