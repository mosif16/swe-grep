# Context capabilities and limits

[Documentation home](<../README.md>) · [JSON output](<agent-mode.md>) · [Migration notes](<changelog.md>)

This page replaces the former context-extraction task checklist. It describes implemented behavior, not promised roadmap work; the filename is retained for existing links.

## Search snippets versus symbol context

Search returns matching locations and snippets. CLI options --context-before/--context-after expand nearby lines; --body requests file bodies and --body-limit limits bytes. Agent output may trim bodies, and --concise omits rich fields. A search body is not necessarily a symbol's definition body.

Context uses repository facts to select a definition and defaults to definition only. It can attach its body, caller/callee information, type/import relationships, and related tests when requested. Without a preset or explicit --depth, relationships are direct (depth 1); presets retain their own depth. --explain adds available definition metadata; --concise deliberately omits richer detail. --grouped/--budget and --max-lines are output controls, not guarantees of complete analysis or exact tokenizer budgets. See context --help for available presets and section flags.

resolution identifies resolved, ambiguous, or not_found declarations. --target-file selects a file relative to the analysis directory; --target-line selects its 1-based declaration start line and requires --target-file. These selectors are available on context and refs and do not narrow the caller scope to the selected file. A selector that does not resolve the declaration does not fall back to another definition. Context --language bounds fact extraction by language.

The analysis object marks each section not_requested, complete, or incomplete. complete means the requested static pass finished within its selected scope and budgets, not that names were compiler-resolved or runtime behavior is exhaustive. Analysis/output truncation, unresolved identity, unavailable analysis, and ambiguous caller attribution can make a section incomplete. Import context remains a placeholder and is marked incomplete when requested. Inspect warnings, notes, and truncated as well as section status.

## Language coverage

The fact-extraction code covers Rust, Swift, TypeScript/TSX, JavaScript/JSX, Go, and Python. Text search can find content beyond those languages; that does not imply semantic graph support for every file type. Language syntax, dynamic dispatch, aliases, generated code, macros, imports, and ambiguous short names can limit results. Static analysis is not compiler type resolution, a cross-language linker, or runtime tracing.

Definition selection and declaration filters are best-effort. Default literal queries can match longer names; opt into --exact-symbol (tool exactSymbol) for complete identifier matching. This removes substring noise without providing compiler identity resolution. Graph relationships and inferred test associations must be checked against source; an empty result is not proof of absence outside the selected scope.

## Semantic references

refs derives callees from calls inside the matching function/method scope, not from every function in a matching file. Caller declarations are not call sites. Rust constructor/variant calls such as Some and Ok are omitted from refs callee output unless --include-constructors is requested. Context and refs may format or select information differently; they are not interchangeable schemas.

Selected callees are restricted to the declaration's scope, including nested same-name declarations. The name-based graph cannot always distinguish incoming calls to duplicate declarations. For a selected declaration these uncertain callers are omitted and analysis.callers is incomplete. Unselected refs can merge names and report that ambiguity; inspect resolution and warnings before treating those edges as attributable to one definition.

Scope-file, path-glob, excludes, language, test exclusions, and Git scopes constrain graph results while retaining allowed semantic edges. Cached unfiltered graphs must not leak excluded callers. --definitions-only and --kind are invalid for refs because refs reports usage edges; --usages-only is accepted.

## Failure and cache behavior

Context fact-extraction errors, including invalid UTF-8 in an analyzed source file or cache I/O failures, produce a nonzero exit and no success JSON, including concise/grouped/stream modes. A genuinely absent symbol can still return a successful object without a def field. refs can report incomplete graph analysis in warnings instead of claiming complete results.

Metadata-first warm reuse avoids rereading unchanged source only where file identity is considered reliable. Unix identity includes size, nanosecond mtime, ctime, device, and inode. Unsupported/coarse identities and non-Unix systems use conservative content hashing. This trusts filesystem metadata, not adversarially forged identities; it is not an integrity guarantee. See [cache guidance](<performance-tuning.md>).

## Regression evidence

Current tests cover [context failures and filtered graphs](<../crates/swe-grep-core/tests/pipeline_context.rs>), [declaration classification](<../crates/swe-grep-core/tests/pipeline_definitions.rs>), [scoped semantic references](<../crates/swe-grep-core/tests/semantic_refs.rs>), and [declaration selection and completeness](<../crates/swe-grep-core/tests/selection_review.rs>). Test coverage documents specific cases, not universal semantic correctness.
