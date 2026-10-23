# Current changes and migration notes

[Documentation home](<../README.md>) · [Quickstart](<quickstart.md>)

This is a behavior-oriented summary of the current working revision, not a tagged release history. Consult git log for historical commits; previous HEAD/prev labels and promotional release claims are no longer used as version identifiers.

## Integration

- Native DSH tools, an official-SDK MCP stdio server, and a reusable backend share the Rust CLI engine.
- Native DSH is tested at 0.2.1-rc.2; MCP SDK is 1.32.1; integration requires Node.js 22+.
- Build the backend binary from this revision. Calls use global --no-config to avoid ambient user configuration; an older executable may reject the flag.
- Tool contracts reject unknown arguments, confine requested paths to their configured root, and expose no arbitrary environment/argument passthrough. Context tool paths must be directories.

## Search declarations and references

Declaration/usage classification is language-aware and refreshed after edits. --definitions-only and --usages-only partition matching declaration/use locations; --kind selects declaration kinds. This is still search: a query can match a longer identifier.

**Migration:** refs rejects --definitions-only and --kind with an error explaining that refs reports usage edges. Use search for declarations, context for a definition, or refs for callers/callees. --usages-only remains accepted for refs.

Semantic reference filtering now keeps allowed call/import graph data while excluding disallowed origins/endpoints. Callees come from matching function/method scopes rather than unrelated functions in a matching file. Filtered queries should not fall back to invented file-wide dependencies.

## Context errors

**Migration:** context no longer disguises fact-extraction/cache I/O failures as an absent symbol. Invalid UTF-8 or other extraction failures cause a nonzero exit with a diagnostic and no success payload, including concise, grouped, and streaming modes. Handle this separately from successful absence (missing/null definition fields).

Request --callers/--callees or an appropriate preset when you need context relationships; empty unrequested sections are not exhaustive graph answers. The harness context contract intentionally exposes fewer options; use swe_refs for graph queries.

## Caches and index

Metadata-first reuse includes reliable Unix ctime identity, not seconds-only mtime. Non-Unix or unreliable metadata falls back to hashing. Optional Tantivy refresh is incremental for added, changed, and deleted files; unchanged reliable warm opens need no writer/commit. These are implementation properties covered by tests, not benchmark promises.

## Licensing

Current licensing is proprietary, owned by Record and Learn LLC. The root [LICENSE](<../LICENSE>) governs use. Contact [msayf@recordandlearn.info](mailto:msayf@recordandlearn.info) for licensing inquiries.
