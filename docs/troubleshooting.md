# Troubleshooting

Common runtime issues and remediation steps.

## Missing dependencies

- **`fd` not found** – install `fd` (`fd-find` on Debian) or run with
  `--disable-fd`.
- **`sg` (ast-grep) not found** – install `ast-grep` via
  `scripts/install-tools.sh` or disable with `--disable-ast-grep`.
- **`rga` not found** – install `ripgrep_all` via the helper script or avoid
  passing `--enable-rga`.

## Large repositories

- Raise timeouts with `--timeout-secs`.
- Increase probe concurrency (default 8) with `--concurrency`.
- Enable Tantivy indexing (`--enable-index`) for repeated searches within the
  same repo. Ensure sufficient disk space for `.swe-grep-index`.

## Cache path permissions

- By default caches live at `<root>/.swe-grep-cache`. Override with
  `--cache-dir` when running in read-only environments.
- Verify the process user has write access; otherwise structured state cannot
  persist between runs.

## Metrics or logs missing

- Ensure `--log-dir` points to a writable directory.
- The `/metrics` endpoint requires at least one completed search cycle before
  counters appear. Scrape after the first request.

## gRPC connectivity

- Confirm ports `50051` (gRPC) and `8080` (HTTP) are reachable from the client.
- Use `grpcurl -plaintext localhost:50051 list` to verify the service is
  reachable.

## Pattern parser warnings

- The AST pipeline prints warnings when patterns cannot be parsed. These
  warnings are non-fatal but indicate reduced precision. Consider tuning the
  symbol query or disabling AST disambiguation for that run.
