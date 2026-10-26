# SWE-Grep Feedback Implementation Plan

> Historical design/review record. Status, commands, measurements and licensing
> descriptions may refer to an older revision or proposal. Use the
> [documentation map](docs/README.md), [contributor guide](CONTRIBUTING.md) and
> [LICENSE](LICENSE) for current guidance.

**Generated:** 2025-12-10
**Feedback Sources:** 6 feedback files (Dec 9-10, 2025)
**Status:** Plan Only (No Implementation)

---

## Executive Summary

Analyzed 6 feedback files from users performing real-world tasks: Swift iOS refactoring, RAG system unification, WhisperKit debugging, and semantic search implementation. Performance ratings consistently excellent (Speed: 5/5), with improvement opportunities in semantic search and result filtering.

---

## Consolidated Feedback Analysis

### What's Working Well (Keep)
- **Blazing fast performance** (1-14ms consistently)
- **`--body` flag** for full file context retrieval
- **`--language` filtering** for relevance
- **`--agent` mode** with structured JSON output
- **`usages` command** for refactoring dependency graphs
- **Batch search** (`--symbols`) for efficiency
- **`suggested_symbols`** field for pivoting searches

### Already Implemented (Confirmed Working)
| Feature | CLI Flag | Status |
|---------|----------|--------|
| Code-only filter | `--code-only` | ✅ Implemented |
| Exclude patterns | `--exclude "*.md"` | ✅ Implemented |
| Definitions-only | `--definitions-only` | ✅ Implemented |
| Usages-only | `--usages-only` | ✅ Implemented |
| Concept expansion | `--concept` | ✅ Implemented (synonym-based) |
| Symbol kind filter | `--kind struct/class/func` | ✅ Implemented |

---

## Prioritized Implementation Plan

### Priority 1: High Impact / Frequently Requested

#### 1.1 Improve Concept Search Accuracy
**Feedback:** "Concept search relies on keyword matching rather than true semantic understanding"

**Current State:** `concept_expansion.rs` has synonym expansion but searches still return irrelevant results for phrases like "RAG indexing after transcription"

**Implementation:**
1. Add programming-domain concept mappings in `concept_expansion.rs`:
   - `"auth"` → `authenticate, login, logout, session, jwt, token, oauth, credentials`
   - `"RAG"` → `embedding, vector, retrieval, semantic, index, chunk, context`
   - `"cache"` → `warm, invalidate, ttl, lru, memoize, persist`
2. Implement phrase parsing for multi-word concepts
3. Weight primary terms higher than synonyms in ranking
4. Add `--concept-expand` flag to show which terms were searched

**Files to modify:**
- `crates/swe-grep-core/src/search/concept_expansion.rs`
- `crates/swe-grep-core/src/search/engine.rs` (ranking logic)

---

#### 1.2 Add `--exclude-tests` Flag
**Feedback:** "A way to exclude test files from results"

**Implementation:**
1. Add `--exclude-tests` CLI flag to `SearchArgs`
2. Pre-populate exclude patterns with common test patterns:
   - `*_test.rs`, `*_test.go`, `*.test.ts`, `*.test.tsx`, `*.spec.ts`
   - `*Tests.swift`, `*Spec.swift`
   - `tests/`, `__tests__/`, `test/`, `spec/`
3. Combine with existing `exclude_patterns` logic

**Files to modify:**
- `crates/swe-grep-core/src/cli.rs` (add flag)
- `crates/swe-grep-core/src/search/config.rs` (pattern expansion)

---

#### 1.3 Add `--path-pattern` for Specific File Patterns
**Feedback:** "Support for searching within specific file patterns (e.g., `--path \"*/Views/*.swift\"`)"

**Current State:** `--path-glob` exists but may not be prominently documented

**Implementation:**
1. Verify `--path-glob` handles this use case
2. If not, enhance glob support to handle `*/Views/*.swift` patterns
3. Add examples to help text

**Files to modify:**
- `crates/swe-grep-core/src/cli.rs` (help text)
- `crates/swe-grep-core/src/search/config.rs` (glob handling if needed)

---

### Priority 2: Developer Experience Improvements

#### 2.1 Add Hit Count Summary in `--concise` Mode
**Feedback:** "Would be nice to see hit count even in minimal output mode"

**Implementation:**
1. Modify concise output to always include `summary.count`
2. Keep bodies/context/scores stripped, but preserve count

**Files to modify:**
- `crates/swe-grep-core/src/agent.rs` (concise output formatting)

---

#### 2.2 Add Per-File Hit Summary Mode
**Feedback:** "Compact mode that summarizes hit counts per file before full snippets"

**Implementation:**
1. Add `--format summary` output format
2. Output structure:
   ```json
   {
     "files": [
       {"path": "src/auth.rs", "hits": 5},
       {"path": "src/login.rs", "hits": 3}
     ],
     "total": 8
   }
   ```
3. Useful for broad queries to understand distribution before drilling down

**Files to modify:**
- `crates/swe-grep-core/src/cli.rs` (add format variant)
- `crates/swe-grep-core/src/agent.rs` (implement summary output)

---

#### 2.3 Editor Integration / Quick Open
**Feedback:** "Quick flag to auto-open top hits in an editor"

**Implementation:**
1. Add `--open` flag that prints editor-compatible output:
   - VS Code: `code -g file:line`
   - Vim: `vim +line file`
   - Default: respects `$EDITOR`
2. Add `--open-command` to customize
3. For non-interactive use, output format compatible with `xargs`

**Files to modify:**
- `crates/swe-grep-core/src/cli.rs` (add flags)
- `crates/swe-grep-core/src/main.rs` (open logic)

---

### Priority 3: Advanced Features

#### 3.1 Negative Search / Absence Verification
**Feedback:** "Verify a symbol does NOT exist in code files (useful for confirming cleanup)"

**Implementation:**
1. Add `--assert-absent` flag
2. Exit code 0 if symbol NOT found, exit code 1 if found
3. Useful for CI/CD to verify removals

**Files to modify:**
- `crates/swe-grep-core/src/cli.rs`
- `crates/swe-grep-core/src/main.rs`

---

#### 3.2 Git-Aware / Diff-Based Search
**Feedback:** "Search only in files that have been modified (integrate with git status)"

**Implementation:**
1. Add `--git-modified` flag to limit search to uncommitted changes
2. Add `--git-since <commit>` to search files changed since a commit
3. Integrate with existing git metadata support

**Files to modify:**
- `crates/swe-grep-core/src/cli.rs`
- `crates/swe-grep-core/src/search/engine.rs`

---

#### 3.3 Protocol/Interface Conformance Search (Swift-specific)
**Feedback:** "Find all types conforming to a protocol"

**Implementation:**
1. Add `--conforms-to <Protocol>` flag for Swift
2. Use AST-grep patterns: `class $NAME: $PROTOCOL`, `struct $NAME: $PROTOCOL`
3. Handle extension conformances: `extension $TYPE: $PROTOCOL`

**Files to modify:**
- `crates/swe-grep-core/src/cli.rs`
- AST-grep pattern configuration

---

#### 3.4 Improved Duplicate Handling
**Feedback:** "Multi-cycle searches occasionally produced duplicate snippets"

**Implementation:**
1. Add content-hash deduplication in result aggregation
2. Keep highest-scored instance when duplicates found
3. Add `--dedup-strategy` flag: `none`, `exact`, `fuzzy`

**Files to modify:**
- `crates/swe-grep-core/src/search/engine.rs`

---

### Priority 4: Nice-to-Have / Future

#### 4.1 True Semantic/Embedding Search
**Feedback:** "Integrate embeddings for concept search to understand programming intent"

**Implementation (Future):**
1. Generate code embeddings using local model (e.g., CodeBERT, StarCoder)
2. Store embeddings in vector index alongside text index
3. Add `--semantic` flag for embedding-based similarity search
4. Consider integration with existing RAG patterns

**Complexity:** High - requires ML dependencies, model management, index storage

---

#### 4.2 Call Graph Exploration
**Feedback:** "Find all callers of this method" or "Find all methods this calls"

**Current State:** `refs` subcommand exists with `--callers` and `--callees` flags

**Implementation:**
1. Verify `refs` command works as expected
2. Document usage in help/README
3. Potentially merge functionality into `search` with flags

---

#### 4.3 Inheritance/Extension Search
**Feedback:** "Find all extensions of AINote" or "Find all subclasses of X"

**Implementation:**
1. Add `--extends <Type>` flag
2. Add `--extensions-of <Type>` flag for Swift extensions
3. Use AST-grep patterns for language-specific inheritance syntax

---

## Implementation Order Recommendation

| Phase | Items | Effort |
|-------|-------|--------|
| **Phase 1** | 1.1, 1.2, 2.1 | Low-Medium |
| **Phase 2** | 1.3, 2.2, 2.3 | Medium |
| **Phase 3** | 3.1, 3.2, 3.4 | Medium |
| **Phase 4** | 3.3, 4.2, 4.3 | Medium-High |
| **Future** | 4.1 | High |

---

## Testing Plan

For each implementation:
1. Add unit tests in `crates/swe-grep-core/src/` modules
2. Add integration tests with real-world patterns from feedback
3. Run existing benchmark suite to verify no performance regression
4. Test with the exact queries from feedback files

---

## Metrics to Track

Post-implementation, measure:
- Concept search precision (% relevant results in top 10)
- Average latency per search type
- User-reported "Relevance of Results" rating (target: 5/5)

---

## Notes

- All existing features have Speed: 5/5 - maintain this baseline
- `--body` flag is highly valued - consider making it default in agent mode
- `suggested_symbols` field saves users time - continue improving fuzzy matching
- AST-grep errors should be suppressed or explained better (exit status 8 warnings)
