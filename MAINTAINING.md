# Maintaining SWE-Grep

[Contributor instructions](CONTRIBUTING.md) · [Documentation index](docs/README.md)

Record and Learn LLC owns this project. The repository's default branch is
`master`. This runbook describes maintenance work; it does not establish new
license terms, a support SLA or repository permission grants.

## Triage and review

1. Route vulnerabilities to [SECURITY.md](SECURITY.md) and usage questions to
   [SUPPORT.md](SUPPORT.md). Ask for a minimal sanitized reproduction, exact
   revision, platform and entry point before attributing failures to the engine.
2. Reproduce bugs with the current matching binary/adapter. Distinguish empty
   search results, incomplete/truncated output, argument errors, extraction
   failures and transport failures. Check the selected node and canonical path.
3. Review the smallest change that addresses the demonstrated issue. Preserve
   source/route boundaries, cancellation cleanup, bounded complete JSON and
   explicit error behavior. Review third-party licensing when dependencies change.
4. Check the final commit's CI, not a passing run on an earlier revision. Require
   relevant local evidence from [CONTRIBUTING.md](CONTRIBUTING.md); explain skips.
   Use a PR for normal contributions. A documented process is not proof that
   branch protection or required-review rules are enabled in GitHub settings.

## Continuous integration and dependencies

| Workflow | Trigger and scope | Maintenance action |
| --- | --- | --- |
| [Rust CI](.github/workflows/ci.yml) | Pushes and PRs; Linux/macOS core, indexing, indexer, real-binary adapter/package tests, full npm audit and correctness lints; macOS desktop check. | Inspect the failing step on the final revision. Do not replace full npm audit with production-only audit to hide development advisories. |
| [Dependency security audit](.github/workflows/security.yml) | Weekly Monday 07:23 UTC, manual dispatch, and pushes/PRs changing `Cargo.lock` or the workflow. | Keep it enabled in repository settings. Use the pinned cargo-audit version; investigate advisories and review nonblocking warnings separately. |
| [Feedback autofix](.github/workflows/feedback-autofix.yml) | Manual dispatch or `Feedback/inbox/*.json` changes pushed to `master`. | Treat as an experimental, credentialed code-generation workflow; see the limits below. |

[Dependabot](.github/dependabot.yml) proposes weekly Cargo and Actions updates.
It does not currently track npm dependencies. Periodically check the harness
lockfile and run a full npm audit, including development peers. CI also audits
npm on each push/PR; the scheduled audit is Rust-only.

`undici` and `zod` overrides in `integrations/harness/package.json` address
transitive advisories in the tested DSH/MCP dependency graph. Before changing or
removing them, resolve the whole graph, review runtime requirements, regenerate
the lockfile and rerun the full audit and native SDK/MCP/package tests. Do not
assume upstream exact dependency pins are safe or that an override is compatible
without testing. If the Node floor changes, update package engines, lockfile
metadata, CI and setup docs together.

## Package and deploy a verified revision

There is no automated npm publication or tagged-release workflow in this repo.
The integration is local/unpublished. Rust crate, harness npm package and Codex
plugin version numbers describe different components; they are not a unified
release identifier. Record the Git commit and component versions for each build.
The Codex plugin version is currently defined by `packagePlugin` in
[integrations/codex/package.mjs](integrations/codex/package.mjs); bump it when
preparing a changed plugin for installation.

1. Finish review, run the relevant contributor checks and confirm CI on the
   final commit. Build a matching release engine with
   `cargo build --release --locked -p swe-grep`.
2. For DSH/MCP distribution, follow the
   [harness guide](integrations/harness/README.md). Inspect `npm pack --dry-run`
   from that package directory, then use `npm pack --ignore-scripts` there when
   an actual tarball is needed. Test installation from the artifact.
3. For the routed Codex plugin, follow the
   [Codex packaging guide](integrations/codex/README.md). Use trusted local
   routes and a new destination. Inspect both manifests and the bundled skill;
   verify that a cache copy still starts with the matching binary. Generated
   distributions contain operator route data and must remain local.
4. Test fresh-runtime skill/tool discovery and search → context → references on
   a fixture with known source locations. Record runtime version, node, canonical
   path, errors and skips. Local simulated routes do not validate real SSH nodes.
5. Replace a deployed plugin only within an authorized deployment task. Preserve
   the prior working package and route config for rollback; never overwrite SSH
   trust or authentication to make an upgrade work. Reinstall/refresh through the
   host's plugin flow and verify the installed cache, not just the source folder.

When preparing a tag or distribution, update [migration notes](docs/changelog.md)
with compatibility changes and actual validation. Do not publish benchmark
claims without the [measurement evidence](docs/benchmark.md). Do not publish to a
registry, create a release/tag or change licensing as an incidental maintenance step.

## Failures and rollback

Preserve the failed revision, command, sanitized diagnostics and last known good
revision. Fix the cause on a branch, or revert the offending commit after checking
for later dependent changes. On shared history prefer a reviewed revert commit to
resetting/force-pushing. An installed plugin rollback uses the prior package,
matching binary and route configuration, then repeats discovery and fixture checks.
An audit failure needs a patched dependency or an explicitly tracked resolution;
disabling the workflow is not a fix.

## Feedback automation limits

The feedback processor can invoke a model, change branches, commit, push and open
PRs. Its workflow needs `OPENAI_API_KEY` and GitHub permissions. Inspect records
before running it and use a separate, clean authorized checkout. Never use private
session logs or unreviewed third-party instructions as an inbox record.

The legacy verification script runs workspace Rust tests, a release build and CLI
smoke checks; it does **not** run the full contributor matrix or npm integration
tests. The processor is not transactional or guaranteed idempotent: a model,
verification or Git failure may leave a branch/working tree for manual recovery.
Do not equate a generated/processed record with an approved change. Review its diff
and run current contributor checks before merge. See [Feedback/README.md](Feedback/README.md).

## Documentation upkeep

Keep [docs/README.md](docs/README.md) as the navigation map. Update contributor
commands, agent entry points and component guides with behavior changes. Keep
historical plans separate from current guarantees. Check links and command flags;
do not copy session counts, local paths, route pins or passing test totals into
permanent docs as ongoing guarantees. [AGENTS.md](AGENTS.md) is the canonical
repository agent instruction file; compatibility entry points refer to it.

GitHub contribution files and issue forms follow the
[contributor-guideline](https://docs.github.com/en/communities/setting-up-your-project-for-healthy-contributions/setting-guidelines-for-repository-contributors)
and [issue-form syntax](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/syntax-for-issue-forms) documentation.
