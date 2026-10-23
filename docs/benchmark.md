# Measuring performance

[Documentation home](<../README.md>) · [Tuning and caches](<performance-tuning.md>)

The former benchmark rankings and latency tables have been removed: they do not establish performance for the current revision. No current comparative speed claim is made. Functional tests and cache-operation counters are not latency benchmarks.

## Reproducible local measurement

Build a release executable and inspect the built-in harness options:

```sh
cargo build --release -p swe-grep
./target/release/swe-grep bench --help
```

The built-in harness advertises a default scenario path, but that file is not present in this checkout. Supply your own scenario JSON with --scenario /absolute/path/to/scenarios.json; do not assume a bare bench run works. Its schema is defined in the [benchmark implementation](<../crates/swe-grep-core/src/bench.rs>). A minimal scenario array is:

```json
[{"name":"rust_login","path":"fixtures/multi_lang","symbol":"login_user","language":"rust","expected":{"path":"src/lib.rs","line":1,"top_n":3}}]
```

Run from the checkout root so the scenario root resolves as shown. Pass --iterations 5 for repeated samples. Fixture expectations measure those scenarios, not general search accuracy. Record the exact scenario file and command with every result. --output can append a machine-readable summary to a path you choose.

## What to record

- Commit/revision and whether the tree was modified; build profile and feature flags.
- OS, architecture, CPU, memory, storage, Rust version, and helper-tool versions/availability.
- Repository revision, file count/size, ignore rules, query, scope, mode, result limits, and output format.
- Separate cold initialization from warm repeated searches; state exactly which caches were reset. --clear-cache deletes SWE-Grep cache/index state, not the OS page cache.
- Total wall-clock process or tool-call time separately from engine-reported latency. Include transport, initialization, parsing, and output costs where relevant.
- Sample count, individual observations, median/tail statistics, failures, warnings, and result correctness.

Only compare runs that perform equivalent work on the same scope. A regex line scan, identifier-expanded search, definition lookup, and semantic graph query are different operations. Verify results before interpreting timing differences. Preserve raw results alongside any published summary; do not extrapolate fixture timings into a product ranking.
