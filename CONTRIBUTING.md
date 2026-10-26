# Contributing to SWE-Grep

SWE-Grep is maintained by Record and Learn LLC. The default branch is `master`.
Start with the [README](README.md) and [documentation index](docs/README.md).

## Permission and scope

The current distribution is proprietary; [LICENSE](LICENSE) governs use and
modification. These instructions do not grant a license or change rights under
earlier licenses. Obtain the owner's written authorization before working on or
submitting code unless an existing agreement already covers your contribution.
Use [msayf@recordandlearn.info](mailto:msayf@recordandlearn.info) for permission or
licensing questions. Submit only work you have the right to share; identify new
third-party code, dependencies and their licenses for review.

Report reproducible bugs and discuss substantial changes through
[GitHub issues](https://github.com/mosif16/swe-grep/issues). Report suspected
vulnerabilities through [SECURITY.md](SECURITY.md), not a public issue. Keep
feedback focused and respectful; see [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## Set up an authorized checkout

Use a current stable Rust toolchain supporting edition 2024, including rustfmt
and Clippy, and Node.js >=22.19 with npm. CI runs Node 22 on Linux and macOS.
Python 3 is used by some optional benchmark scripts, not the core build.

```sh
git clone https://github.com/mosif16/swe-grep.git
cd swe-grep
git switch -c fix/describe-the-change
cargo build --locked -p swe-grep
npm ci --ignore-scripts --prefix integrations/harness
```

The root npm package is repository metadata, not the integration development
entry point. Run npm commands in `integrations/harness` or use `--prefix` as shown.
Do not run a root npm install to set up the plugin. Optional fd, ast-grep and rga
helpers are described in [quickstart](docs/quickstart.md); the normal fixture
checks do not require them.

Use SWE-Grep for source navigation as described in [AGENTS.md](AGENTS.md).
Build the binary from the same revision as the adapter. Tool examples and limits
are in the [agent workflow](docs/agent-use.md) and
[integration guides](docs/integration.md).

## Make and verify a change

Keep one concrete problem per change. Add a regression test for changed behavior,
preferably against the real binary when a CLI/adapter boundary is involved.
Verify returned definitions and edges against source, not just result shapes.
Documentation-only edits need checked commands, links and examples rather than
new behavior tests. Update relevant usage docs when changing public behavior.

From the checkout root, run the checks relevant to your change:

```sh
cargo fmt --all -- --check
cargo test --locked -p swe-grep --all-targets
cargo test --locked -p swe-grep --all-targets --features indexing
cargo test --locked -p swe-grep-indexer
cargo build --locked -p swe-grep
SWE_GREP_BINARY="$PWD/target/debug/swe-grep" npm test --prefix integrations/harness
npm run check --prefix integrations/harness
npm audit --prefix integrations/harness
cargo clippy --locked -p swe-grep -p swe-grep-indexer --all-targets --features swe-grep/indexing -- -D clippy::correctness
git diff --check
```

For desktop changes, also run `cargo check --locked -p swe-grep-desktop --all-targets`;
CI performs that check on macOS. To inspect the npm package, run
`npm pack --dry-run --ignore-scripts` **from `integrations/harness`**.
For dependency updates, run `cargo audit` after installing the version specified
in [.github/workflows/security.yml](.github/workflows/security.yml). Include both
manifest and lockfile changes. Do not use broad force-updates or advisory ignore
lists just to get a green audit.

`SWE_GREP_BINARY` must be an absolute path. Without it, real-binary tests skip;
a successful test exit alone does not prove the whole integration ran. Read the
pass/fail/skip summary and explain any platform-specific skips. Keep existing
non-correctness Clippy warnings distinct from the blocking correctness gate.

Windows runtime behavior and real SSH nodes are not covered by the Linux/macOS
CI matrix. Tests that use local route fixtures do not establish remote deployment
health. Record the platforms and paths actually tested.

## Submit for review

Target `master` and use a short title describing the resulting behavior. The PR
body should explain the problem, the change, validation commands/results, and any
compatibility or migration implications. Use the
[PR template](.github/pull_request_template.md). Link the related issue when one
exists; identify unverified behavior plainly.

Keep credentials, real route configurations, private source, local session logs,
plugin installations, build outputs and generated tarballs out of the patch.
Use small synthetic fixtures and sanitized excerpts. Generated evidence belongs
in a local scratch directory; ignored `Feedback/runs/` is available for local
verification artifacts and is not a shared evidence store.

Maintainers review the code, tests, docs, dependency changes and CI results before
accepting a contribution. An owner may explicitly authorize direct changes to
`master`; that does not remove the need to verify them. Do not force-push shared
branches or rewrite another contributor's work without authorization.
