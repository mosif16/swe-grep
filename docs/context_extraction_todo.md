# Context capabilities and limits

[Documentation home](<../README.md>) · [JSON output](<agent-mode.md>) · [Migration notes](<changelog.md>)

This page replaces the former context-extraction task checklist. It describes implemented behavior, not promised roadmap work; the filename is retained for existing links.

## Search snippets versus symbol context

Search returns matching locations and snippets. CLI options --context-before/--context-after expand nearby lines; --body requests file bodies and --body-limit limits bytes. Agent output may trim bodies, and --concise omits rich fields. A search body is not necessarily a symbol's definition body.

Context uses repository facts to select a definition and can attach its body, caller/callee information, type/import relationships, and related tests when requested. --explain adds available definition metadata; --concise deliberately omits richer detail. --grouped/--budget and --max-lines are output controls, not guarantees of complete analysis or exact tokenizer budgets. See context --help for available presets and section flags.

## Language coverage

The fact-extraction code covers Rust, Swift, TypeScript/TSX, JavaScript/JSX, Go, and Python. Text search can find content beyond those languages; that does not imply semantic graph support for every file type. Language syntax, dynamic dispatch, aliases, generated code, macros, imports, and ambiguous short names can limit results. Static analysis is not compiler type resolution, a cross-language linker, or runtime tracing.

Definition selection and declaration filters are best-effort. Literal queries can match longer names. Graph relationships and inferred test associations must be checked against source; an empty result is not proof of absence outside the selected scope.

## Semantic references

refs derives callees from calls inside the matching function/method scope, not from every function in a matching file. Caller declarations are not call sites. Rust constructor/variant calls such as Some and Ok are omitted from refs callee output unless --include-constructors is requested. Context and refs may format or select information differently; they are not interchangeable schemas.

Scope-file, path-glob, excludes, language, test exclusions, and Git scopes constrain graph results while retaining allowed semantic edges. Cached unfiltered graphs must not leak excluded callers. --definitions-only and --kind are invalid for refs because refs reports usage edges; --usages-only is accepted.

## Failure and cache behavior

Context fact-extraction errors, including invalid UTF-8 in an analyzed source file or cache I/O failures, produce a nonzero exit and no success JSON, including concise/grouped/stream modes. A genuinely absent symbol can still return a successful object without a def field. refs can report incomplete graph analysis in warnings instead of claiming complete results.

Metadata-first warm reuse avoids rereading unchanged source only where file identity is considered reliable. Unix identity includes size, nanosecond mtime, ctime, device, and inode. Unsupported/coarse identities and non-Unix systems use conservative content hashing. This trusts filesystem metadata, not adversarially forged identities; it is not an integrity guarantee. See [cache guidance](<performance-tuning.md>).

## Regression evidence

Current tests cover [context failures and filtered graphs](<../crates/swe-grep-core/tests/pipeline_context.rs>), [declaration classification](<../crates/swe-grep-core/tests/pipeline_definitions.rs>), and [scoped semantic references](<../crates/swe-grep-core/tests/semantic_refs.rs>). Test coverage documents specific cases, not universal semantic correctness.
