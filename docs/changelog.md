# Current changes and migration notes

[Documentation home](<../README.md>) · [Quickstart](<quickstart.md>)

This is a behavior-oriented summary of the current working revision, not a tagged release history. Consult git log for historical commits; previous HEAD/prev labels and promotional release claims are no longer used as version identifiers.

## Integration

- Native DSH tools, an official-SDK MCP stdio server, and a reusable backend share the Rust CLI engine.
- Native DSH is tested at 0.2.1-rc.2; MCP SDK is 1.32.1; integration requires Node.js >=22.19.
- Build the backend binary from this revision. Calls use global --no-config to avoid ambient user configuration; an older executable may reject the flag.
- Tool contracts reject unknown arguments and expose no arbitrary environment/argument passthrough. The portable/native backend confines paths to its configured root. Context tool paths must be directories.
- The [Codex router](../integrations/codex/README.md) adds explicit node/absolute-path inputs, identity checks, a bundled skill and relocatable marketplace packaging. It intentionally permits OS-readable paths on the selected node rather than confining analysis to one project.

## Search declarations and references

Declaration/usage classification is language-aware and refreshed after edits. --definitions-only and --usages-only partition matching declaration/use locations; --kind selects declaration kinds. This is still search: a query can match a longer identifier.

Opt into --exact-symbol (swe_search exactSymbol) to require a complete identifier and remove substring noise during match collection. It uses identifier boundaries including Unicode continuation characters and `$`; it is not compiler identity resolution. Existing default literal search remains unchanged, and exact matching requires literal mode.

**Migration:** refs rejects --definitions-only and --kind with an error explaining that refs reports usage edges. Use search for declarations, context for a definition, or refs for callers/callees. --usages-only remains accepted for refs.

Semantic reference filtering now keeps allowed call/import graph data while excluding disallowed origins/endpoints. Callees come from matching function/method scopes rather than unrelated functions in a matching file. Filtered queries should not fall back to invented file-wide dependencies.

## Context errors

**Migration:** context no longer disguises fact-extraction/cache I/O failures as an absent symbol. Invalid UTF-8 or other extraction failures cause a nonzero exit with a diagnostic and no success payload, including concise, grouped, and streaming modes. Handle this separately from successful absence (missing/null definition fields).

**Migration:** unpreset context defaults to definition only; explicitly request --callers/--callees or a preset for relationships. Requested unpreset relationships default to direct depth 1; use --depth for transitive context. The tool exposes optional direction and language without arbitrary CLI flags.

Context and refs return resolution candidates and per-section analysis status (not_requested, complete, incomplete). complete describes the requested static pass within its scope and budgets, not a compiler or runtime proof. Truncated, unresolved, or unavailable analysis is marked incomplete. Empty unrequested sections are not exhaustive graph answers.

Select an ambiguous declaration with --target-file/--target-line, or tool targetFile/targetLine. Paths are relative to the analysis directory and lines are 1-based declaration start lines; targetLine requires targetFile. Selection does not restrict caller scope, and a missing selector match does not fall back to another definition. Selected outgoing calls are isolated from other same-name methods/nested declarations. Uncertain incoming calls to duplicate declarations are omitted and marked incomplete.

## Caches and index

Metadata-first reuse includes reliable Unix ctime identity, not seconds-only mtime. Non-Unix or unreliable metadata falls back to hashing. Optional Tantivy refresh is incremental for added, changed, and deleted files; unchanged reliable warm opens need no writer/commit. These are implementation properties covered by tests, not benchmark promises.

Opt-in --enable-index --exact-symbol uses index-first candidate discovery for supported ASCII identifiers, then verifies source. Unsupported or incomplete candidates fall back to scanning; default substring and regex behavior remains unchanged. See [index limits](performance-tuning.md). The plugin does not expose index/cache CLI controls.

## Licensing

Current licensing is proprietary, owned by Record and Learn LLC. The root [LICENSE](<../LICENSE>) governs use. Contact [msayf@recordandlearn.info](mailto:msayf@recordandlearn.info) for licensing inquiries.
