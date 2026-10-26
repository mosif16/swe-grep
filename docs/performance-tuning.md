# Cache and performance guidance

[Documentation home](<../README.md>) · [Measurement procedure](<benchmark.md>)

No latency or competitor-performance guarantee is made here. Repository size, syntax, scope, disk state, tool availability, and output format all affect cost.

## Bound the work

- Prefer a narrow directory, --scope-file, or --path-glob before increasing limits.
- Use raw --text when you need fixed-string content rather than symbol expansion.
- Use --exact-symbol when longer identifiers are irrelevant. It filters during collection so substring noise does not consume the match budget.
- Request context relationships explicitly; definition-only context avoids building an unused call graph. Default unpreset relationship depth is 1; increase --depth when transitive context is needed.
- Use --concise for bounded agent payloads; avoid full file bodies unless needed.
- --top-hits limits surfaced results; --max-matches limits collection per rewrite. Neither proves a complete count of repository occurrences.
- --timeout-secs is a per-tool CLI timeout, not a total request deadline. The adapter has a separate whole-call timeout and output-byte limit.
- Optional --disable-fd and --disable-ast-grep skip those helpers. Disabling the external AST helper does not disable the independent tree-sitter fact/graph analysis used by refs.

## Metadata-first caches

Analysis facts/graphs and file data can reuse unchanged entries based on reliable metadata identity. On Unix this includes file size, nanosecond modification/change times, device, and inode. Warm metadata hits can avoid source reads and hashes. Same-size edits, restored mtime edits, and atomic replacements are covered by regression tests.

The optimization trusts filesystem ctime/identity semantics; it is not a defense against a party able to forge metadata or tamper with caches. Non-Unix systems and unsupported/coarse identities conservatively read/hash content rather than assuming mtime alone is sufficient. Old cache formats can be invalidated and rebuilt.

Default cache resolution uses XDG_CACHE_HOME or HOME's .cache, under swe-grep/managed-v1 and a root-derived namespace. The trusted environment prefix is canonicalized before managed levels are appended, so macOS /var and /tmp aliases work without accepting symlinked managed directories. When those environment paths are unavailable, a repository-local .swe-grep-cache/managed-v1 fallback is used. Previous unmarked namespaces are preserved rather than adopted; first use of the managed layout rebuilds disposable facts/state.

Use search/refs --cache-dir for an explicit writable location. Custom directories stay outside automatic collection; pointing a custom directory into an owned managed namespace is rejected to prevent bypassing its activity lease. Context has no --cache-dir flag; ensure its normal cache location is writable. Cache I/O failures are not universally silent: context reports fact-extraction failure.

## Runtime cache garbage collection

Cache users hold shared OS file leases. A collector uses nonblocking exclusive leases and skips active namespaces. After a managed operation releases its final shared lease, it can trigger a bounded sweep at most hourly, with a default age of 30 days and soft size target of 512 MiB. This lets a just-finished project participate in size collection. Sweeps inspect at most 128 namespaces and 4096 payload entries per run; persistent namespace and payload cursors prevent an early group of unknown entries from monopolizing repeated sweeps. Active payloads are not scanned, so byte totals exclude them; scan limits and busy namespaces can leave usage above the soft target.

Preview or apply collection explicitly:

```sh
./target/release/swe-grep --no-config --disable-telemetry cache gc --dry-run
./target/release/swe-grep --no-config --disable-telemetry cache gc --apply --max-age-days 30 --max-bytes 536870912
```

Dry run is the default and leaves files and timestamps unchanged. JSON reports list eligible/removed files and bytes, projected remaining bytes, busy/unmanaged/unknown skips, and scan limits. Collection deletes only recognized runtime payloads in owned namespaces: state.json, facts with generated hash names, and the current graph-cache filename. Ownership, usage and lease files stay in place; source files, symlinks, unknown files, unmarked legacy caches, custom cache directories, and Tantivy indexes are preserved. Repository-local fallback caches are not included in automatic/global sweeps.

--clear-cache now purges only the selected root's managed runtime payloads before the command, then the command rebuilds what it needs. It reports an error if an active cache user or collector prevents clearing. It preserves unknown files, legacy directories and indexes; it is not a read-only diagnostic. Index cleanup needs a separate lifecycle and is not part of this collector. Cargo target and registry data are also separate; see [build-cache cleanup](build-cache-cleanup.md).

## Optional incremental Tantivy index

The default core build has no indexing feature. To build and opt into the index path:

```sh
cargo build --release --locked -p swe-grep --features indexing
./target/release/swe-grep --no-config --disable-telemetry search login_user fixtures/multi_lang --exact-symbol --enable-index --json-only
```

The default index directory is .swe-grep-index under the search root; --index-dir overrides it. Index discovery is an optional search path, not a replacement for all text or semantic analysis.

With --enable-index and --exact-symbol, supported ASCII identifiers use index candidates before a repository scan, then verify actual source matches. Each underscore-separated alphanumeric component must contain fewer than 40 bytes. Incomplete source coverage, unsupported tokenization, more than 4096 candidate files, empty candidates, or index failures retain the scan fallback. Default substring and regex queries keep their scan semantics. Existing later index fallback remains available where applicable.

An indexed lookup's rich search hits have an rg-indexed origin; concise output omits origin. Diagnostic search logs include index_candidates and index_probe_hits stage counters. These show the path used, not a latency advantage: refreshing the index still walks/stats sources, and cold initialization can be costly. Measure equivalent cold and warm workflows before choosing indexing for a repository.

Refresh compares the manifest with current files and updates added, changed, and deleted documents. On reliable Unix metadata, unchanged warm opens avoid source reads, hashing, writer acquisition, and commits. Metadata-only changes can update identity without reindexing unchanged content. Corrupt/incompatible manifests trigger recovery rather than a speed guarantee. See [incremental index tests](<../crates/swe-grep-indexer/tests/incremental.rs>).

The harness tool contract does not expose --enable-index, --index-dir, or arbitrary cache flags. Do not add unknown tool arguments to enable CLI-only features.
