# Cache and performance guidance

[Documentation home](<../README.md>) · [Measurement procedure](<benchmark.md>)

No latency or competitor-performance guarantee is made here. Repository size, syntax, scope, disk state, tool availability, and output format all affect cost.

## Bound the work

- Prefer a narrow directory, --scope-file, or --path-glob before increasing limits.
- Use raw --text when you need fixed-string content rather than symbol expansion.
- Use --concise for bounded agent payloads; avoid full file bodies unless needed.
- --top-hits limits surfaced results; --max-matches limits collection per rewrite. Neither proves a complete count of repository occurrences.
- --timeout-secs is a per-tool CLI timeout, not a total request deadline. The adapter has a separate whole-call timeout and output-byte limit.
- Optional --disable-fd and --disable-ast-grep skip those helpers. Disabling the external AST helper does not disable the independent tree-sitter fact/graph analysis used by refs.

## Metadata-first caches

Analysis facts/graphs and file data can reuse unchanged entries based on reliable metadata identity. On Unix this includes file size, nanosecond modification/change times, device, and inode. Warm metadata hits can avoid source reads and hashes. Same-size edits, restored mtime edits, and atomic replacements are covered by regression tests.

The optimization trusts filesystem ctime/identity semantics; it is not a defense against a party able to forge metadata or tamper with caches. Non-Unix systems and unsupported/coarse identities conservatively read/hash content rather than assuming mtime alone is sufficient. Old cache formats can be invalidated and rebuilt.

Default cache resolution uses XDG_CACHE_HOME or HOME's .cache, under swe-grep and a root-derived namespace, with a repository-local fallback. Use search/refs --cache-dir for an explicit writable location. Context has no --cache-dir flag; ensure its normal cache location is writable. Cache I/O failures are not universally silent: context reports fact-extraction failure. --clear-cache removes state/index data before the requested command; it is not a read-only diagnostic.

## Optional incremental Tantivy index

The default core build has no indexing feature. To build and opt into the index path:

```sh
cargo build --release -p swe-grep --features indexing
./target/release/swe-grep --no-config --disable-telemetry search login_user fixtures/multi_lang --enable-index --json-only
```

The default index directory is .swe-grep-index under the search root; --index-dir overrides it. Index discovery is an optional search path, not a replacement for all text or semantic analysis.

Refresh compares the manifest with current files and updates added, changed, and deleted documents. On reliable Unix metadata, unchanged warm opens avoid source reads, hashing, writer acquisition, and commits. Metadata-only changes can update identity without reindexing unchanged content. Corrupt/incompatible manifests trigger recovery rather than a speed guarantee. See [incremental index tests](<../crates/swe-grep-indexer/tests/incremental.rs>).

The harness tool contract does not expose --enable-index, --index-dir, or arbitrary cache flags. Do not add unknown tool arguments to enable CLI-only features.
