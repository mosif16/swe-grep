# SWE-Grep Implementation Plan

Based on analysis of 5 feedback reports from Dec 9, 2025, this plan prioritizes user-requested features by impact and implementation complexity.

---

## Executive Summary

| Priority | Feature | Effort | Impact |
|----------|---------|--------|--------|
| P0 | Case-insensitive search | Small | High |
| P0 | Definitions-only filter | Small | High |
| P0 | Usages-only filter | Small | High |
| P1 | Full body option | Trivial | Medium |
| P1 | Minimal output format | Small | Medium |
| P2 | Fuzzy matching suggestions | Medium | Medium |
| P2 | Type-aware search (`--kind`) | Medium | Medium |
| P3 | Cross-reference / call graph | Large | High |
| P3 | Git change detection | Medium | Low |

---

## P0: Critical Features (Quick Wins)

### 1. Case-Insensitive Search (`--ignore-case` / `-i`)

**User Pain Point:** "First search without language filter returned no results because the symbol casing didn't match"

**Files to Modify:**
- `crates/swe-grep-core/src/cli.rs` - Add flag
- `crates/swe-grep-core/src/search/config.rs` - Add `ignore_case: bool` to SearchConfig
- `crates/swe-grep-core/src/search/query.rs` - Inject `(?i)` prefix when enabled
- `crates/swe-grep-core/src/tools/rg.rs` - Pass `-i` flag to ripgrep

**Implementation Steps:**

1. **cli.rs:124** - Add flag to `SearchArgs`:
   ```rust
   /// Perform case-insensitive matching.
   #[arg(short = 'i', long = "ignore-case", default_value_t = false, action = ArgAction::SetTrue)]
   pub ignore_case: bool,
   ```

2. **config.rs:19** - Add field to `SearchConfig`:
   ```rust
   pub(crate) ignore_case: bool,
   ```

3. **config.rs:134** - Set from args:
   ```rust
   ignore_case: args.ignore_case,
   ```

4. **query.rs:67** - Modify `build_symbol_queries` to prepend `(?i)` for all patterns when `ignore_case` is enabled. Pass `ignore_case` to `QueryRewriter::for_symbol()`.

5. **rg.rs** - Add `-i` flag to ripgrep invocation when `ignore_case` is set.

**Tests:**
- `cargo test --package swe-grep-core -- ignore_case`
- Manual: `swe-grep search --symbol "searchengine" -i --path .`

---

### 2. Definitions-Only Filter (`--definitions-only`)

**User Pain Point:** "Would be nice to have a `--usages-only` flag to find only call sites (excluding definitions)" and "Definition-only mode"

**Files to Modify:**
- `crates/swe-grep-core/src/cli.rs` - Add `--definitions-only` flag
- `crates/swe-grep-core/src/search/config.rs` - Add `definitions_only: bool`
- `crates/swe-grep-core/src/search/engine.rs` - Filter hits post-collection

**Implementation Steps:**

1. **cli.rs:124** - Add flag:
   ```rust
   /// Only show definition sites (struct, fn, class, etc.), excluding usages.
   #[arg(long = "definitions-only", default_value_t = false, action = ArgAction::SetTrue)]
   pub definitions_only: bool,
   ```

2. **config.rs** - Add field and pass through.

3. **engine.rs** - After collecting hits, filter to keep only those where snippet contains definition keywords:
   ```rust
   fn is_definition_site(snippet: &str, symbol: &str, languages: &[String]) -> bool {
       // Rust: fn, struct, enum, trait, impl, type, const, static, mod
       // Swift: func, class, struct, enum, protocol, extension, actor
       // TypeScript: function, class, interface, type, const, let, var, enum
       let def_patterns = [
           format!("fn {symbol}"),
           format!("struct {symbol}"),
           format!("enum {symbol}"),
           format!("trait {symbol}"),
           format!("impl {symbol}"),
           format!("class {symbol}"),
           format!("interface {symbol}"),
           format!("func {symbol}"),
           format!("protocol {symbol}"),
           format!("type {symbol}"),
           format!("const {symbol}"),
           format!("function {symbol}"),
       ];
       def_patterns.iter().any(|p| snippet.contains(p))
   }
   ```

4. Apply filter in `rank_and_deduplicate()` or after hit collection.

**Tests:**
- `swe-grep search --symbol SearchEngine --definitions-only --path .`
- Should return only `pub(crate) struct SearchEngine` and `impl SearchEngine`, not usages.

---

### 3. Usages-Only Filter (`--usages-only`)

**Implementation:** Inverse of `--definitions-only`. Same files, opposite filter logic.

1. **cli.rs** - Add flag:
   ```rust
   /// Only show usage sites, excluding definitions.
   #[arg(long = "usages-only", default_value_t = false, action = ArgAction::SetTrue)]
   pub usages_only: bool,
   ```

2. **config.rs** - Validate mutual exclusivity with `--definitions-only`.

3. **engine.rs** - Filter hits to exclude definition sites.

**Tests:**
- `swe-grep search --symbol SearchEngine --usages-only --path .`
- Should NOT return struct/impl definitions.

---

## P1: Medium Priority

### 4. Full Body Option (`--body-limit 0` or `--full-body`)

**User Pain Point:** "Some file bodies were truncated at ~500 lines"

**Files to Modify:**
- `crates/swe-grep-core/src/cli.rs` - Modify `--trim-body` semantics or add `--full-body`
- `crates/swe-grep-core/src/search/config.rs` - Set `max_body_bytes` to `usize::MAX` when enabled

**Implementation Steps:**

1. **cli.rs** - Add flag:
   ```rust
   /// Retrieve full file bodies without truncation (overrides --trim-body).
   #[arg(long = "full-body", default_value_t = false, action = ArgAction::SetTrue)]
   pub full_body: bool,
   ```

2. **config.rs** - When `full_body` is set:
   ```rust
   max_body_bytes: if args.full_body { usize::MAX } else { DEFAULT_MAX_BODY_BYTES },
   ```

3. Also support `--trim-body 0` as equivalent to `--full-body`.

---

### 5. Minimal Output Format (`--format minimal`)

**User Pain Point:** "Verbose JSON output... clutter the results"

**Files to Modify:**
- `crates/swe-grep-core/src/cli.rs` - Add `Minimal` to `OutputFormat` enum
- `crates/swe-grep-core/src/main.rs` or output formatting code - Implement minimal format

**Implementation Steps:**

1. **cli.rs:94** - Add variant:
   ```rust
   #[derive(Copy, Clone, Debug, ValueEnum)]
   pub enum OutputFormat {
       Json,
       Table,
       Compact,
       Tree,
       Edit,
       Minimal,  // NEW: path:line:snippet only
   }
   ```

2. **Output formatting** - When `Minimal` is selected:
   ```
   src/search/engine.rs:37:pub(crate) struct SearchEngine {
   src/search/engine.rs:57:impl SearchEngine {
   ```

   No JSON, no metadata, just `path:line:snippet` per line.

---

## P2: Lower Priority

### 6. Fuzzy Matching with Suggestions

**User Pain Point:** "Suggestion of similar symbols when exact match not found"

**Files to Modify:**
- `crates/swe-grep-core/src/search/engine.rs` - Add fallback search
- New: `crates/swe-grep-core/src/search/fuzzy.rs` - Levenshtein distance helper

**Implementation Steps:**

1. When search returns 0 hits:
   - Run a broader search (e.g., first 3 chars of symbol)
   - Compute Levenshtein distance to original symbol
   - Return top 5 suggestions in `hints` field

2. Use `strsim` crate for distance calculations:
   ```toml
   strsim = "0.11"
   ```

3. Add to output JSON:
   ```json
   {
     "hits": [],
     "suggestions": ["SearchEngine", "SearchConfig", "SearchCache"]
   }
   ```

---

### 7. Type-Aware Search (`--kind`)

**User Pain Point:** "Search for 'all async functions' or 'all actors'"

**Files to Modify:**
- `crates/swe-grep-core/src/cli.rs` - Add `--kind` flag with enum
- `crates/swe-grep-core/src/search/engine.rs` - Filter by kind

**Implementation Steps:**

1. **cli.rs** - Add enum and flag:
   ```rust
   #[derive(Copy, Clone, Debug, ValueEnum)]
   pub enum SymbolKind {
       Struct,
       Class,
       Func,
       Enum,
       Trait,
       Protocol,
       Interface,
       Const,
       Actor,
   }

   /// Filter results by declaration type.
   #[arg(long, value_enum)]
   pub kind: Option<SymbolKind>,
   ```

2. **engine.rs** - Use AST-grep or snippet analysis to filter by kind.

3. For Swift-specific kinds like `actor`, check for `actor MySymbol` pattern.

---

## P3: Larger Features (Future)

### 8. Cross-Reference / Call Graph

**User Pain Point:** "Show callers/callees of a function", "Call graph visualization"

**Approach:**
1. Use AST-grep to find all call sites
2. Build a dependency graph
3. Output in tree or JSON format

**New Subcommand:**
```bash
swe-grep refs --symbol MyFunction --callers --path .
swe-grep refs --symbol MyFunction --callees --path .
```

**Effort:** ~2-3 days of work. Requires AST analysis for each language.

---

### 9. Git Change Detection

**User Pain Point:** "Flag if a symbol has been modified recently"

**Approach:**
1. Run `git log --oneline -n 1 -- <file>` for each hit
2. Add `last_modified` field to output
3. Optional `--recent` flag to only show recently modified files

**New Flag:**
```bash
swe-grep search --symbol MySymbol --git-info --path .
```

**Output:**
```json
{
  "file": "src/foo.rs",
  "line": 42,
  "last_commit": "abc1234",
  "last_modified": "2025-12-01"
}
```

---

## Implementation Order

```
Week 1:
├── P0.1: --ignore-case / -i
├── P0.2: --definitions-only
├── P0.3: --usages-only
└── P1.1: --full-body

Week 2:
├── P1.2: --format minimal
├── P2.1: Fuzzy suggestions (basic)
└── Tests & documentation

Week 3+:
├── P2.2: --kind filter
├── P3.1: Cross-reference (refs subcommand)
└── P3.2: Git integration
```

---

## Testing Strategy

1. **Unit tests** for each new config field
2. **Integration tests** with sample codebases
3. **Benchmark validation** - ensure no performance regression
4. **Manual testing** with real Swift/Rust/TypeScript projects

---

## Documentation Updates

After implementation, update:
- `CLAUDE.md` - Add new flags to quick reference
- `docs/benchmark.md` - Add performance notes for new features
- `--help` output - Already handled via clap annotations

---

## Metrics for Success

| Metric | Target |
|--------|--------|
| Case-insensitive search latency | < 2ms (same as current) |
| Definition filter accuracy | > 95% precision |
| Fuzzy suggestion relevance | Top suggestion correct > 80% |
| No performance regression | Mean latency within 5% of baseline |
