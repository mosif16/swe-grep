# Pipeline review and hardening

## Scope

Reviewed CLI/configuration, discovery, embedded/external matching, AST disambiguation, fallback/index search, verification/ranking, context/reference extraction, caches, and HTTP/gRPC request boundaries. This is a tested hardening pass, not a claim that every dependency or performance issue is solved.

## Fixed

| Area | Hole | Change |
| --- | --- | --- |
| Service authorization | Requests could select an unrelated repository or arbitrary output/cache paths. | Canonical root containment, symlink escape rejection, and trusted-startup-only storage paths. File-root services authorize that file only. |
| Common verification | RGA could return siblings outside a single-file scope, bypassing the service boundary. | Every backend result now passes root, file-scope, extension, glob, exclusion, and Git checks before snippets/bodies are returned. |
| AST mode | AST matches were never converted to output hits; the decoder expected `path` rather than real CLI `file`. | Materialize source-line hits, accept the real JSON field, apply normal verification, retain documented strict AST-only behavior. |
| Definitions/usages | Enclosing function hints and a narrow regex lost valid declarations or mislabeled calls. | Parse one syntax tree per file snapshot and index declaration-name rows; preserve regex/wildcard queries, Rust implementations/constants/statics/aliases/modules, local bindings, and JS/TS regex literals. |
| Paths/input | Nested relative file arguments doubled their parent path; invalid scopes/regexes could look like no matches. | Canonicalize inferred file roots; preserve existing literal glob-character filenames; reject missing/directory/outside scopes and malformed regexes. |
| Git scope | Empty change sets became all tracked files; whitespace/renames/subdirectory roots were parsed incorrectly. | Explicit filter intersections, NUL-delimited records, rename destination handling, individual untracked files, repository-relative normalization. |
| File cache | Externally mutable mmap bytes could invalidate unchecked UTF-8 and cached content. | Owned immutable snapshots, explicit cycle invalidation, current-content validation for reusable caches, I/O outside the cache mutex. |
| Analysis cache | Equal-size edits could reuse stale facts; unconditional verification then reread every source on warm queries. | Versioned high-resolution file identities enable reliable metadata hits; changed/unreliable identities are read and hashed once, with pre/post-read race checks. |
| Graph cache | Optional JSON fields broke binary cache round-trips. | Cache-private stable wire representation; reconstruct derived indexes on load. |
| Persistence | Concurrent processes shared temporary filenames; cached hints could escape the selected root. | Unique atomic snapshot replacement; reject absolute, parent-traversal, hidden, and outside-symlink hints. |
| Tool execution | Full stderr pipes, capped stdout, and timed-out waits could hang or orphan direct children. | Concurrent bounded diagnostics, guarded cancellation, kill/reap at caps/errors, bounded JSON records/AST output, safe argument separation. |
| Embedded matching | Delimiter-based matcher cache keys collided; rereads could invalidate byte offsets. | Structured pattern keys, isolated regex alternatives, search the same bytes used for offsets. |
| Context/refs | Extraction errors silently reported absence; unfiltered graphs leaked excluded files and disabling graphs lost valid edges. | Keep semantic parse context, apply the same prepared search scope to emitted edges, and omit unsafe text guesses. Partial/unavailable graphs carry explicit incomplete warnings. |
| Python bodies | Definition spans covered only the name token. | Full declaration node spans include function/class bodies. |
| Optional index | First build/freshness were broken; unconditional rebuilding defeated repeated-query speed. | Atomic versioned manifests, incremental exact-path updates/deletes, and reader-only warm opens on reliable identities. Safely migrate to an owned versioned child; preserve legacy files. |
| Scope cost | Validation repeatedly canonicalized the same hit paths. | Share one memoized eligibility scope between text search and semantic enrichment; cache decisions per query and precompute single-file scope. |
| Logs | Short-lived CLI shutdown could race pending asynchronous file writes. | Flush the summary log before returning. |
| TypeScript documentation | A dead outer loop blocked correctness lints; one-line JSDoc could absorb earlier code. | Simplify the scan and terminate correctly for one-line comments, with ordering regressions. |

## Behavior and performance tradeoffs

- **Do not expose the server directly to untrusted networks.** Root containment is not authentication, TLS, per-user authorization, rate limiting, or protection against concurrent malicious local filesystem mutation. Startup paths and installed external tools remain trusted.
- Service requests cannot override index/cache/log directories; configure those at startup. Literal glob-character service roots remain rejected conservatively. CLI single-file scopes must remain under their selected root.
- Context extraction now fails clearly on unreadable/invalid-UTF-8 recognized source files rather than pretending the symbol is absent. Recoverable malformed cache entries are rebuilt.
- Filtered reference commands retain semantic graphs. Language, Git, glob, exclusion, and file-scope filters restrict emitted call origins; known import endpoints are filtered and excluded nodes are not traversed. A successful empty graph query stays empty: no whole-file regex guesses are added. Reference queries reject declaration-only/kind flags explicitly and direct users to search/context instead.
- File snapshots are immutable within a search cycle and refreshed for subsequent cycles. Matching and verification are not an atomic repository snapshot. The general-purpose cache validates content on each lookup; the search engine reuses per-cycle snapshots to avoid repeated disk I/O per hit.
- Warm index opens still walk/stat eligible paths and read a manifest, but do not read source content, tokenize, commit, or acquire a writer when Unix identities are reliable and unchanged. Refresh reads/hashes only changed candidates and rewrites only changed/deleted documents. The manifest is published in the same Tantivy commit as documents. No live watcher is added. Legacy/corrupt owned child indexes are retained rather than destructively deleted.
- Analysis/index fast paths use nanosecond timestamps plus Unix ctime, device and inode. They require honest filesystem metadata. Coarse/unavailable identities (including current Windows support) conservatively hash content. Dirty source reads compare identities before/after reading rather than caching bytes under a mismatched identity.
- Declaration classification uses parser-provided name locations for supported grammars, not body spans. Unsupported languages have a conservative lexical fallback; this is still not a full cross-language name resolver.

## Validation

Regression coverage includes source snapshots and equal-metadata edits, corrupt caches, binary wire compatibility, concurrent writes, child-process saturation/cleanup, Unicode and byte offsets, scope/path/Git behavior, AST decoding/hits, fallback authorization, filtered references, context errors, and complete Python bodies. AST/RGA integration tests use isolated executable fixtures because those tools are not installed on the test host. Real-tool compatibility beyond these fixtures is not claimed.

Final local validation on macOS ARM64 with Rust 1.97.1 after purpose-driven revisions: **134 core tests passed in both default and indexing configurations; 14 indexer tests passed** (148 distinct tests). Formatting, workspace/all-target compilation including desktop, correctness lint gate, and whitespace checks passed. Existing non-correctness lint warnings remain. Linux/macOS CI is configured; no remote CI run is claimed.

Purpose-driven workflow checks additionally verify that adding a Python/Rust/TypeScript language filter preserves real callees and does not add calls from unrelated functions; filtered imports remain available; regex alternation/wildcard declaration queries keep correct declaration rows. Work-counter tests verify warm analysis source reads are zero for reliable metadata and warm index source reads/hashes/writer acquisitions/commits are zero. Incremental refresh tests assert only changed/added/deleted documents are rewritten. Path-scope tests assert repeated eligibility checks resolve each distinct path only once per query. These are observable work guarantees, not an unmeasured end-to-end speedup claim.

Repeat locally:

```sh
cargo fmt --all -- --check
cargo test --locked -p swe-grep --all-targets
cargo test --locked -p swe-grep --all-targets --features indexing
cargo test --locked -p swe-grep-indexer
cargo check --locked --workspace --all-targets
cargo clippy --locked -p swe-grep -p swe-grep-indexer --all-targets --features swe-grep/indexing -- -D clippy::correctness
cargo audit
```

[Cross-platform CI](<.github/workflows/ci.yml>) runs formatting, tests, and correctness lint checks. Existing style warnings are visible rather than suppressed; a blanket `-D warnings` gate still requires a separate lint cleanup. [Scheduled security auditing](<.github/workflows/security.yml>) intentionally fails on advisories rather than ignoring them. [Dependency update automation](<.github/dependabot.yml>) opens weekly Cargo/action update PRs.

## Harness plugin integration

SWE-Grep now also ships a native DSH tool bundle, an official-SDK MCP stdio server, and a shared portable backend adapter. The three tools are swe_search, swe_context, and swe_refs; all reuse the same Rust engine. [Installation and adapter contracts](<integrations/harness/README.md>) cover prerequisites, package activation, execution-world providers, and scope/cancellation limits.

Validation: **49 JavaScript integration tests passed with the real Rust binary**, including native SDK definitions/provider mocks and official MCP clients/transports. A packed production installation without optional DSH peers successfully imported the portable API and performed real search/context calls. The production npm audit reported zero known vulnerabilities at validation time; this does not erase the existing Rust workspace advisory backlog below. The native plugin was subsequently installed and enabled in the active Harness profile; all three tool bindings were exercised successfully with the production binary. Its profile-specific binary override lives outside this repository.

The portable child environment is allowlisted to avoid passing credentials/harness controls; native execution retains provider-managed environment and filesystem identities. Calls suppress ambient CLI configuration, and Git fsmonitor hooks/optional locks are disabled. Windows local descendant cancellation and adversarial concurrent filesystem mutation remain explicitly outside the guarantees.

## Dependency security results

The workspace audit started with **11 vulnerability entries**. Compatible patches and tested core-runtime migrations removed **6**, leaving **5**; `cargo audit` still exits nonzero.

- Patched locked bytes, crossbeam-epoch, time, webbrowser, anyhow, and current memmap2 versions.
- Migrated tonic/tonic-build to 0.12.3 and prost to 0.13.5, removing old hyper/axum/h2 stacks; h2 now resolves to 0.4.20.
- Migrated the OpenTelemetry family/exporter to 0.33.1 and prometheus to 0.14.0 with protobuf-free text encoding. This removes protobuf 2.28.0 entirely. A metrics regression checks series values, labels, histograms, and resource naming. Existing counter suffix behavior is preserved.
- Remaining vulnerability entries: optional Tantivy compression uses lz4_flex 0.9.5 (RUSTSEC-2026-0041); desktop Linux accessibility/Wayland dependencies use quick-xml 0.30.0 and 0.37.5 (RUSTSEC-2026-0194 and RUSTSEC-2026-0195 on each version). These need dependency-family migrations, not an incompatible forced patch.
- Informational unsound/unmaintained advisories also remain, including core lru, rand, and bincode. They are not erased by the reduced vulnerability-entry count.

## Remaining priorities

1. Finish dependency-family migrations for the optional index and desktop stacks; audit is explicitly **not clean**.
2. Enforce total per-request memory/time budgets, byte-weighted file caches, and service concurrency/backpressure limits. Several paths still read complete source files; tool-level limits are not a single end-to-end deadline.
3. Make capped parallel embedded-search selection deterministic before truncation. Sorting afterward does not stabilize which worker wins the cap.
4. Normalize external/embedded backend behavior for non-UTF-8 ripgrep JSON byte fields and long-line truncation; strengthen real-tool CI coverage.
5. Add process-group/descendant cancellation for RGA adapters, not just direct child cleanup.
6. Extend parser declaration coverage and graph name-resolution precision without reintroducing text guesses or weakening scope checks.
7. Benchmark request-cycle snapshots, large source files, capped matching, and incremental indexing before making performance claims.
