# Measuring performance

[Documentation home](<../README.md>) · [Tuning and caches](<performance-tuning.md>)

The former benchmark rankings and latency tables have been removed: they do not establish performance for the current revision. No current comparative speed claim is made. Functional tests and cache-operation counters are not latency benchmarks.

## Reproducible local measurement

Build a release executable and inspect the built-in harness options:

```sh
cargo build --release --locked -p swe-grep
./target/release/swe-grep bench --help
```

From the checkout root, a bare `bench` run uses the shipped [default scenarios](<../benchmarks/default.json>). These three lookups have source-checked expectations in the multi-language fixture. Supply `--scenario /absolute/path/to/scenarios.json` for another workload. Its schema is defined in the [benchmark implementation](<../crates/swe-grep-core/src/bench.rs>). A minimal scenario array is:

```json
[{"name":"rust_login","path":"fixtures/multi_lang","symbol":"login_user","language":"rust","expected":{"path":"src/lib.rs","line":1,"top_n":3}}]
```

Run from the checkout root so the scenario root resolves as shown. Pass `--iterations 7` for repeated samples. Fixture expectations measure those scenarios, not general search accuracy. Record the exact scenario file and command with every result. `--output` appends a machine-readable summary to a path you choose.

The built-in harness measures **in-process search engine time**. Each scenario reports every latency sample, whether the expected location matched, extraction errors, warnings and truncation, plus mean, nearest-rank median/p95 and success rate. A correctness failure produces a report and a nonzero exit status. Empty scenarios and zero iterations are rejected. The first sample is labeled `first_run_uncontrolled`, since existing application and OS caches may already be warm; subsequent samples are labeled `repeat`. No cache is deleted by this harness.

## End-to-end agent workflows

The [task-driver script](<../scripts/benchmark-agent-workflows.mjs>) measures wall time across CLI subprocesses and the local adapter, including launch, parsing and output costs:

```sh
cargo build --release --locked -p swe-grep --features indexing
node scripts/benchmark-agent-workflows.mjs \
  --binary "$PWD/target/release/swe-grep" \
  --iterations 7 --files 200 \
  --output /tmp/swe-agent-workflows.json
```

It generates a disposable Rust corpus with known definitions, callers/callees, longer identifier distractors and an exact raw-text marker. It verifies result locations/relationships and exits nonzero if any task fails. Reports retain commands, stdout/stderr or adapter results, elapsed time, tool-call counts and output bytes for each iteration. Only temporary generated corpus/cache directories are removed. The output report is retained.

Measured tasks:

| Task | Variants |
| --- | --- |
| Exact definition lookup | Current CLI scan and opt-in index |
| Definition plus callers/callees | Current CLI and adapter; optional baseline CLI and adapter |
| Separate search → context → refs | Current CLI |
| Raw-text locations | Current CLI, optional baseline CLI, and ripgrep when installed |

For a before/after comparison, build an earlier revision with the same release profile/features in a separate checkout, then add:

```sh
node scripts/benchmark-agent-workflows.mjs \
  --binary /absolute/current/target/release/swe-grep \
  --baseline-binary /absolute/baseline/target/release/swe-grep \
  --baseline-adapter /absolute/baseline/integrations/harness/src/backend.js \
  --iterations 7 --files 200 \
  --output /tmp/swe-agent-before-after.json
```

The baseline uses its regex-boundary workaround for exact identifiers. The CLI can request definition and relationships in two calls in either revision. The old adapter needs a separate references call because its context contract cannot request relationships; the current adapter can complete the same task in two calls. Both must return the expected definition and caller/callee locations. Do not interpret this deterministic reduction in calls as proof that a model will choose the tool or solve arbitrary tasks faster.

Each variant starts with a separate empty application cache; later samples reuse it. OS page cache is uncontrolled. Variants are interleaved in a fixed order, so increase sample counts, vary corpus size and repeat independent runs before inferring a stable latency advantage. Median/p95 summarize warm samples separately from the initial sample. CLI output bytes include stderr; adapter output bytes measure serialized results. Compare only variants performing the same task. An index may cost more than scanning on a small corpus.

These measurements exclude model reasoning, MCP transport, remote SSH and real repository diversity. A claim of **agent preference** requires an additional controlled agent evaluation with the same model, prompts, tool descriptions, budgets and repository revisions; record chosen tools and successful task outcomes across runs. The task driver establishes local capability/cost evidence, not a general product ranking.

## What to record

- Commit/revision and whether the tree was modified; build profile and feature flags.
- OS, architecture, CPU, memory, storage, Rust version, and helper-tool versions/availability.
- Repository revision, file count/size, ignore rules, query, scope, mode, result limits, and output format.
- Separate cold initialization from warm repeated searches; state exactly which caches were reset. --clear-cache deletes the selected root's managed runtime payloads, preserving active data, legacy/custom caches, indexes and the OS page cache.
- Total wall-clock process or tool-call time separately from engine-reported latency. Include transport, initialization, parsing, and output costs where relevant.
- Sample count, individual observations, median/tail statistics, failures, warnings, and result correctness.

Only compare runs that perform equivalent work on the same scope. A regex line scan, identifier-expanded search, definition lookup, and semantic graph query are different operations. Verify results before interpreting timing differences. Preserve raw results alongside any published summary; do not extrapolate fixture timings into a product ranking.
