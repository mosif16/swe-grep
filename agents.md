# SWE-Grep Agent Checklist

## Phase 0: Governance & Vision
- [x] Enforce the strict tracking rule: update this checklist and the CLI plan tool immediately after each sub-task.
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
- [ ] Log search cycle data for offline analysis and regression tracking.
- [ ] Build a SWE-Bench-style benchmark harness capturing queries/sec and recall@line.
- [ ] Prepare reinforcement-learning tooling for policy updates.
- [ ] Create synthetic repository fixtures spanning Rust, Swift, TypeScript, and config files.
- [ ] Add regression tests for query routing, timeout handling, and JSON parsing.
- [ ] Implement benchmark suites tracking latency to first hit, precision@5, and dedup effectiveness.

## Phase 6: API, Ops & Developer Experience
- [ ] Expose a gRPC/HTTP service alongside the CLI.
- [ ] Containerize the agent with minimal runtime dependencies.
- [ ] Document integration workflows for external coding agents.
- [ ] Emit structured JSON logs with query metadata, exit status, and latency metrics.
- [ ] Export Prometheus/OpenTelemetry counters for tool calls, rewards, and cache hit rates.
- [ ] Provide feature flags to toggle individual tools (e.g., disable `ast-grep`).
- [ ] Publish a quickstart guide covering installation of external binaries.
- [ ] Supply sample cycle JSON outputs, CLI usage examples, and integration snippets.
- [ ] Maintain a troubleshooting guide for common failure modes (missing binaries, large repos).

## Phase 7: Future Extensions
- [ ] Explore adaptive query rewriting guided by accumulated rewards.
- [ ] Fingerprint repositories to preload specialized heuristics.
- [ ] Produce optional WASI builds for sandboxed environments.
- [ ] Develop shared cache services for multi-agent deployments.
