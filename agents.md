# SWE-Grep Agent Checklist

## Phase 0: Governance & Vision
- [x] Enforce the strict tracking rule: update this checklist and the CLI plan tool immediately after each sub-task.
- [ ] Every benchmark run must be recorded in `docs/benchmark.md` before the task is marked complete.
- [ ] After completing any phase checklist, run the benchmark suite and log the results in `docs/benchmark.md` before checking off the phase.
- [ ] Deliver a Rust-native search agent that returns exact file and line spans within a 4-second reasoning loop.
- [ ] Operate without large language models or embeddings, relying on deterministic Rust tooling plus heuristic feedback.
- [ ] Serve as the search backbone for higher-level coding agents by streaming precise retrieval contexts.
- [ ] Maintain a Rust-first toolchain wrapping `fd`, `rg`, `ast-grep`, `rga`, and `tantivy`, honoring `.gitignore`.
- [ ] Execute up to eight parallel probes per loop with cooperative cancellation on high-confidence hits.
- [ ] Emit structured JSON telemetry each cycle (queries, top hits, dedup stats, next actions).
- [ ] Reinforce search heuristics using precision, density, clustering, and novelty rewards.
- [ ] Enforce two-to-four second latency budgets via timeouts and adaptive query refinement.

## Phase 1: MVP Tool Harness
- [x] Ship async wrappers for `fd`, `rg --json`, and `ast-grep --json` with timeout handling.
- [x] Provide hard-coded query rewrites and simple scoring heuristics.
- [x] Deliver a working CLI command `swe-grep search --symbol <identifier>`.

## Phase 2: Search Workflow Foundation
- [x] Implement the Discover stage with scoped `fd` scans and heuristic pruning.
- [x] Implement the Probe stage firing four `rg --json` rewrite variants with path scoping.
- [x] Implement the Disambiguate stage using `ast-grep` to validate symbol alignment.
- [x] Implement the Escalate stage to relax filters, use `rga`, or consult `tantivy` indices.
- [x] Implement the Verify & Summarize stage to rank hits, compute rewards, and surface next actions.

## Phase 3: Parallel Scheduler & Heuristic Refinement
- [x] Introduce a Tokio worker pool with cancellation and latency caps.
- [x] Expose configurable concurrency controls (default eight workers).
- [x] Enrich cycle summaries with latency, reward, and precision metrics.
- [x] Implement precision, density, and clustering metrics for scoring.
- [x] Add reward accumulation per reasoning cycle.
- [x] Build a deduplication cache that reuses past high-value hits across cycles.

## Phase 4: Advanced Tools & Caching
- [x] Integrate optional `tantivy` micro-indices for hot repositories.
- [x] Add a heuristic-driven `rga` fallback for documentation and config searches.
- [x] Persist symbol-to-path hints and high-value directory caches between runs.

## Phase 5: Evaluation & Benchmarking
- [x] Log search cycle data for offline analysis and regression tracking.
- [x] Build a SWE-Bench-style benchmark harness capturing queries/sec and recall@line.
- [x] Prepare reinforcement-learning tooling for policy updates.
- [x] Create synthetic repository fixtures spanning Rust, Swift, TypeScript, and config files.
- [x] Add regression tests for query routing, timeout handling, and JSON parsing.
- [x] Implement benchmark suites tracking latency to first hit, precision@5, and dedup effectiveness.

## Phase 6: API, Ops & Developer Experience
- [x] Expose a gRPC/HTTP service alongside the CLI.
- [x] Document integration workflows for external coding agents.
- [x] Emit structured JSON logs with query metadata, exit status, and latency metrics.
- [x] Export Prometheus/OpenTelemetry counters for tool calls, rewards, and cache hit rates.
- [x] Provide feature flags to toggle individual tools (e.g., disable `ast-grep`).
- [x] Publish a quickstart guide covering installation of external binaries.
- [x] Supply sample cycle JSON outputs, CLI usage examples, and integration snippets.
- [x] Maintain a troubleshooting guide for common failure modes (missing binaries, large repos).

## Phase 7: Low-Latency Core
- [x] Profile cold and warm search cycles to identify dominant latency contributors (runtime init, fd, rg, ast-grep, telemetry).
- [x] Keep discovery/probe tools hot (process pools, cached handles) and reuse fd results between rewrites.
- [x] Trim orchestration overhead by eliminating redundant ripgrep invocations and batching JSON serialization.
- [x] Implement a literal fast path that bypasses AST/rga when context features are unnecessary while preserving top-hit enrichment.

## Phase 8: Performance Guardrails
- [x] Build an automated benchmark harness comparing `swe-grep` vs `rg` across representative repositories and symbols.
- [x] Track cold/warm latency, throughput, and reward via CI dashboards with regression thresholds.
- [x] Make telemetry/logging overhead configurable so performance modes stay within target budgets.
- [x] Document tuning guidelines for maintaining rich context with sub-`rg` latency in production environments.
