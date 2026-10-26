# Documentation map

[Project README](../README.md) · [License](../LICENSE)

## Start here

| Task | Maintained guide |
| --- | --- |
| Build and try the CLI | [Quickstart](quickstart.md) |
| Contribute an authorized change | [Contributing](../CONTRIBUTING.md) |
| Maintain, package, upgrade or roll back | [Maintaining](../MAINTAINING.md) |
| Get help or report a bug | [Support](../SUPPORT.md) |
| Report a vulnerability privately | [Security](../SECURITY.md) |
| Participate in project discussions | [Participation expectations](../CODE_OF_CONDUCT.md) |
| Work as an agent | [AGENTS.md](../AGENTS.md) and [agent workflow](agent-use.md) |

## Usage and integration

- [DSH native plugin, portable MCP and backend reuse](../integrations/harness/README.md).
- [Codex node router and marketplace packaging](../integrations/codex/README.md).
- [Integration contracts and trust boundaries](integration.md).
- [JSON output and error handling](agent-mode.md).
- [Troubleshooting](troubleshooting.md) and [analysis capabilities/limits](context_extraction_todo.md).
- [Cache and performance tuning](performance-tuning.md) and [measurement procedure](benchmark.md).
- [Safe Cargo build-artifact cleanup](build-cache-cleanup.md).
- [Current behavior and migration notes](changelog.md), not a tagged release history.
- [Feedback collection](../Feedback/README.md); its autofix processor is experimental
  and does not replace the contributor verification matrix.

## Source of truth and historical material

For supported commands and checks, consult the current manifests, `--help`,
[workflows](../.github/workflows/ci.yml), tests and the guides above. Documented
examples must agree with the revision being changed. Version numbers and past
passing test totals do not establish current compatibility or deployment health.

The root [roadmap](../ROADMAP.md), [implementation plan](../IMPLEMENTATION_PLAN.md),
[phase-two plan](../PHASE2-PLAN.md), [improvement notes](../IMPROVEMENTS.md),
[context tasks](../CONTEXT_RETRIEVAL_TASKS.md),
[feedback implementation plan](../FEEDBACK-IMPLEMENTATION-PLAN.md),
[feedback improvement plan](../FEEDBACK-IMPROVEMENT-PLAN.md), and
[pipeline review](../PIPELINE_REVIEW.md) are historical design or review records.
Their status lists, commands, timings and licensing descriptions may describe
older revisions or proposals. Files under `MARKETING/` are historical drafts.
Use [LICENSE](../LICENSE) for current licensing and the maintained guides for
setup, contribution and maintenance. Preserve historical evidence; do not silently
turn proposals or past measurements into present guarantees.
