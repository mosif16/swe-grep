# Security reporting

Report suspected vulnerabilities privately to
[msayf@recordandlearn.info](mailto:msayf@recordandlearn.info), the project
maintainer contact. Use a subject such as `SWE-Grep security report`. Avoid public
issues or PRs containing exploit details, credentials or private repository data.

Include the affected Git commit/component version, platform, entry point
(CLI, DSH, MCP, Codex router or service), a minimal reproduction and the expected
versus observed trust-boundary behavior. Use synthetic files and redacted logs.
Describe the impact and any tested mitigation; send only information you are
authorized to share. Do not test against other people's machines or repositories.

Security fixes are evaluated against the current `master` revision. Older
packages may require upgrading the binary and adapter together; no maintained
backport branches or response-time commitments are established here. Coordinate
disclosure with the maintainer. No bug-bounty or legal safe-harbor program is
created by this document.

## Boundaries relevant to reports

- The portable harness backend confines targets to its configured root. Native
  DSH uses provider filesystem/process policy; neither is an OS sandbox.
- The Codex router intentionally permits readable local/mounted paths on an
  explicitly selected trusted node. Its filesystem root is not a project sandbox.
  Node identity checks and existing trusted routes must not silently fall back.
- Binaries, route commands/configuration, host runtimes and installed helpers are
  trusted operator inputs. Models cannot supply executable paths, arbitrary
  process arguments or environment overrides through tool calls.
- Read-only tools may write analysis caches. Static results are best-effort and
  do not prove runtime reachability or exhaustive absence.
- The CLI HTTP/gRPC `serve` interface is separate from MCP stdio. Do not assume
  MCP adapter confinement supplies authentication or transport protection for it.

See [integration boundaries](docs/integration.md),
[Codex routing](integrations/codex/README.md), and the
[maintenance runbook](MAINTAINING.md) for audit and update procedures.
