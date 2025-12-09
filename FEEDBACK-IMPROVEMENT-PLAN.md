# SWE-Grep Feedback Improvement Plan

**Created:** 2025-12-09
**Based on:** 5 user feedback reports
**Goal:** Address critical pain points and high-value feature requests

---

## Executive Summary

| Priority | Issue | Impact | Effort |
|----------|-------|--------|--------|
| **P0** | Markdown/docs noise in results | High (5/5 reports) | Medium |
| **P1** | Concept search not semantic | High (2/5 reports) | High |
| **P1** | AST-grep failures without fallback | Medium (2/5 reports) | Low |
| **P2** | Output truncation in CLI | Medium (3/5 reports) | Low |
| **P2** | Better result ranking (code > docs) | Medium (3/5 reports) | Medium |

---

## Phase 1: Quick Wins (1-2 days)

### 1.1 Add `--code-only` Flag
**File:** `crates/swe-grep-core/src/cli.rs`

Add a new flag to exclude non-code files:

```rust
// In SearchArgs struct (around line 144)
/// Exclude non-code files (markdown, txt, json configs) from results.
#[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
pub code_only: bool,
```

**Implementation:**
- Define code extensions per language in `language.rs`
- Filter results in `engine.rs` before returning hits
- When `--language` is set, auto-enable stricter filtering

### 1.2 Add `--exclude` Glob Patterns
**File:** `crates/swe-grep-core/src/cli.rs`

```rust
// In SearchArgs struct
/// Glob patterns to exclude from results (e.g., "*.md,PLAN*").
#[arg(long = "exclude", value_delimiter = ',')]
pub exclude_patterns: Vec<String>,
```

**Implementation:**
- Build exclude GlobSet in `config.rs` (similar to `path_globs`)
- Apply filter in ripgrep invocation and result post-processing

### 1.3 AST-Grep Fallback on Failure
**File:** `crates/swe-grep-core/src/tools/ast_grep.rs`

When AST-grep fails with parsing errors:
1. Log warning with suggestion
2. Auto-fallback to regex search for simple identifiers
3. Include `"ast_fallback": true` in output metadata

```rust
// Pseudo-code
match ast_grep_result {
    Err(e) if is_simple_identifier(&symbol) => {
        warn!("AST-grep failed, falling back to regex: {}", e);
        run_regex_fallback(symbol)
    }
    other => other
}
```

### 1.4 Paginated/Limited Body Output
**File:** `crates/swe-grep-core/src/agent.rs`

- Default `--trim-body 50` in agent mode (already exists)
- Add `--body-head N` and `--body-tail N` for partial output
- Show `"body_truncated": true` indicator (already exists)

---

## Phase 2: Result Ranking & Filtering (3-5 days)

### 2.1 Prioritize Code Files Over Docs
**File:** `crates/swe-grep-core/src/search/engine.rs`

Implement scoring boost for code files:

```rust
const FILE_TYPE_SCORES: &[(&str, i32)] = &[
    // High priority (code)
    ("rs", 100), ("swift", 100), ("ts", 100), ("tsx", 100),
    ("js", 90), ("jsx", 90), ("py", 90), ("go", 90),
    // Medium priority (config/build)
    ("toml", 50), ("yaml", 50), ("json", 40),
    // Low priority (docs)
    ("md", 10), ("txt", 10), ("rst", 10),
];
```

- Apply score multiplier during hit ranking
- When `--language` is set, boost matching extensions higher

### 2.2 Definition-First Ranking
Enhance existing `--definitions-only` to also influence ranking:
- Definition sites score higher than usages
- Show definitions first, then usages (even without flag)

### 2.3 Smarter Deduplication
When same symbol found in both code and docs:
- Prefer code file hit
- Collapse doc hits into a "also mentioned in docs" summary

---

## Phase 3: Semantic Concept Search (1-2 weeks)

### 3.1 Current State Analysis
**File:** `crates/swe-grep-core/src/search/engine.rs` (line 741)

Current `--concept` implementation:
- Tokenizes phrase into keywords
- Runs OR-combined searches
- Returns keyword matches (not semantic)

### 3.2 Improved Concept Search Strategy

**Option A: Enhanced Keyword Expansion (Medium effort)**
- Use programming domain synonyms
- Expand "RAG" → "retrieval", "augmented", "generation", "embedding", "vector"
- Expand "authentication" → "auth", "login", "session", "token", "jwt"
- Build domain-specific thesaurus

**Option B: Embedding-Based Search (High effort)**
- Integrate local embedding model (e.g., Nomic, BGE-small)
- Pre-compute embeddings for function/class names and docstrings
- Query with semantic similarity

**Recommended:** Start with Option A, plan Option B as future enhancement

### 3.3 Implementation Plan (Option A)

1. Create `concept_expansion.rs` module:
```rust
pub fn expand_concept(phrase: &str) -> Vec<String> {
    let mut terms = tokenize(phrase);
    for term in terms.clone() {
        if let Some(synonyms) = PROGRAMMING_SYNONYMS.get(&term) {
            terms.extend(synonyms.iter().cloned());
        }
    }
    terms
}
```

2. Add domain-specific synonym mappings:
```rust
const PROGRAMMING_SYNONYMS: &[(&str, &[&str])] = &[
    ("rag", &["retrieval", "embedding", "vector", "semantic"]),
    ("auth", &["authentication", "login", "session", "token"]),
    ("cache", &["caching", "memoize", "store", "persist"]),
    ("error", &["exception", "failure", "catch", "handle"]),
    // ... more
];
```

3. Weight exact matches higher than synonym matches

---

## Phase 4: Advanced Features (Future)

### 4.1 Type-Aware Search
Add `--type` filter for declaration kinds:

```rust
#[derive(ValueEnum)]
pub enum DeclType {
    Struct, Class, Enum, Func, Trait, Protocol, Const, Actor
}

// In SearchArgs
#[arg(long, value_enum)]
pub decl_type: Option<DeclType>,
```

Use AST-grep patterns per language to filter.

### 4.2 Protocol/Interface Conformance Search
New subcommand or flag:

```bash
swe-grep conformances --protocol Codable --language swift
```

Uses AST-grep to find all types with `: Codable` conformance.

### 4.3 Call Graph Exploration
Enhance `refs` subcommand:
- `--callers`: Find all call sites
- `--callees`: Extract called functions from body
- Build lightweight call graph for navigation

### 4.4 Git-Aware Search
```bash
swe-grep search --symbol Foo --git-modified  # Only search changed files
swe-grep search --symbol Foo --git-staged    # Only staged files
```

### 4.5 Negative/Verification Search
```bash
swe-grep verify --not-in-code "OldSymbol"  # Exit 0 if NOT found in code
```

Useful for CI/CD to verify removals.

---

## Implementation Order

```
Week 1:
├── Phase 1.1: --code-only flag
├── Phase 1.2: --exclude patterns
├── Phase 1.3: AST-grep fallback
└── Phase 1.4: Body pagination improvements

Week 2:
├── Phase 2.1: File type scoring
├── Phase 2.2: Definition-first ranking
└── Phase 2.3: Smarter deduplication

Week 3-4:
├── Phase 3.1: Concept expansion module
├── Phase 3.2: Domain synonym mappings
└── Phase 3.3: Integration & testing

Future:
├── Phase 4.1: Type-aware search
├── Phase 4.2: Conformance search
├── Phase 4.3: Call graph
├── Phase 4.4: Git integration
└── Phase 4.5: Negative search
```

---

## Success Metrics

| Metric | Current | Target |
|--------|---------|--------|
| Relevance rating | 4.0 | 4.5+ |
| Accuracy rating | 4.2 | 4.7+ |
| "Code before docs" complaints | 5/5 | 0/5 |
| Concept search satisfaction | Low | High |

---

## Files to Modify

| File | Changes |
|------|---------|
| `cli.rs` | Add `--code-only`, `--exclude`, `--decl-type` |
| `config.rs` | Add exclude patterns, code-only logic |
| `engine.rs` | File type scoring, ranking improvements |
| `ast_grep.rs` | Fallback on failure |
| `agent.rs` | Output format tweaks |
| `language.rs` | Define code vs non-code extensions |
| NEW: `concept_expansion.rs` | Semantic expansion logic |

---

## Testing Plan

1. **Unit tests** for new filters and scoring
2. **Integration tests** with sample repos containing mixed code/docs
3. **Regression tests** to ensure speed stays at 5/5
4. **User acceptance** re-run feedback scenarios after fixes
