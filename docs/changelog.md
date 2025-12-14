# Recent Changes (last 10 commits)

Latest-first snapshot of the recent work so docs stay in sync with code:

- `e7b30ff` — **Call Graph & Type Graph Analysis**
  - Added CallGraph with qualified name preservation (`Foo::new` matches only `Foo::new`, not all `new()`)
  - Dual indexing: supports both qualified (`Type::method`) and simple (`method`) lookups
  - TypeGraph for tracking type relationships (conformance, inheritance, implementation)
  - Integrated call graph into `refs --callers` command
  - Tree-sitter parsing for Rust, Swift, and TypeScript
  - Per-file fact caching with modification-time invalidation
  - Fixed tree-sitter ABI version (upgraded to 0.25 for version 15 compatibility)

- `f7e1c0f` — Add automated feedback processing workflow and related scripts.
- `19363a6` — Add feedback collection feature for shell command tool.
- `c3222f0` — Add support for attribute searches and enhance search configuration options.
- `853d3fc` — Add advanced search features and marketing docs.
- `d05840d` — Add code-only and exclude filters, concept expansion, and result ranking.
- `174299a` — Add advanced search filters and output formats.
- `1ff1c9f` — Add string and path glob search support.
- `683babb` — Add discovery and refactor helpers to swe-grep.
- `ee681f0` — Add agent mode with structured output and batch search.
- `cc6d745` — Refactor search tool configuration and add language utilities.
- `d801031` — Add file cache and embedded ripgrep search.
- `375badd` — Misc fixes and cleanup.
- `549f925` — Add warnings field to `SearchSummary`/`SearchEngine` for error tracking.
- `4e46ebf` — Remove outdated agent integration guide from agents.md.
- `7374171` — Enhance agent integration guide and usage documentation.
- `4d06d8d` — Remove outdated agent checklist sections from agents.md.

For a deeper history, run `git log` or browse the repo history.
