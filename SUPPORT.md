# Getting help

Start with [quickstart](docs/quickstart.md), the
[troubleshooting guide](docs/troubleshooting.md), and the guide for your entry
point: [DSH/MCP](integrations/harness/README.md) or
[Codex node plugin](integrations/codex/README.md).

For reproducible bugs and proposed improvements, use
[GitHub issues](https://github.com/mosif16/swe-grep/issues). Include:

- Git commit, whether the tree is modified, binary build profile/features and
  adapter/plugin version. A CLI version alone may not identify the source revision.
- OS/architecture, Rust/Node versions, host/client version and entry point.
- Minimal synthetic source, exact command or tool arguments, expected behavior and
  actual result/error. For routed calls, include the selected node and sanitized
  `_swe` identity/path; never attach private routes, keys or complete session logs.
- Relevant stdout/stderr separately, exit status, truncation/skip indicators and
  whether the matching binary was rebuilt. A no-match result alone is not a crash.

For private questions or permission/licensing requests, contact
[msayf@recordandlearn.info](mailto:msayf@recordandlearn.info). Suspected
vulnerabilities follow [SECURITY.md](SECURITY.md). No response-time or platform
support guarantee is implied. Contribution steps are in
[CONTRIBUTING.md](CONTRIBUTING.md).
