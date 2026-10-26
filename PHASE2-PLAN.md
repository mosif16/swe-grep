# Phase 2: Intelligence & Analysis Layer

> Historical design/review record. Status, commands, measurements and licensing
> descriptions may refer to an older revision or proposal. Use the
> [documentation map](docs/README.md), [contributor guide](CONTRIBUTING.md) and
> [LICENSE](LICENSE) for current guidance.

## Objective

Build a reusable "analysis layer" that extracts code facts (definitions, references, calls, type-relations) with a stable JSON schema. Surface these facts through existing commands (`refs`, `usages`, `remove`) and new commands (`types`, `watch`) with consistent output formats (`json|tree|compact`).

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      CLI Commands                           │
│  search | refs | usages | remove | types | watch            │
└─────────────────────────────┬───────────────────────────────┘
                              │
┌─────────────────────────────▼───────────────────────────────┐
│                   Analysis Layer                            │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │ Facts Cache │  │ Call Graph  │  │ Type Relations      │  │
│  │ (per-file)  │  │ (adjacency) │  │ (inheritance/impl)  │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
└─────────────────────────────┬───────────────────────────────┘
                              │
┌─────────────────────────────▼───────────────────────────────┐
│                   Extractors (per-language)                 │
│  ┌───────────┐  ┌───────────┐  ┌───────────┐  ┌──────────┐  │
│  │   Rust    │  │   Swift   │  │  TS/TSX   │  │  Python  │  │
│  └───────────┘  └───────────┘  └───────────┘  └──────────┘  │
└─────────────────────────────────────────────────────────────┘
```

## JSON Schema (v1)

```json
{
  "version": 1,
  "file": "src/engine.rs",
  "mtime": 1702500000,
  "language": "rust",
  "facts": {
    "definitions": [
      {"symbol": "SearchEngine", "kind": "struct", "line": 62, "span": [62, 85]}
    ],
    "references": [
      {"symbol": "SearchConfig", "line": 63, "context": "field"}
    ],
    "calls": [
      {"caller": "SearchEngine::new", "callee": "RipgrepTool::new", "line": 99}
    ],
    "relations": [
      {"from": "SearchEngine", "to": "Default", "kind": "impl_trait", "line": 150}
    ]
  }
}
```

---

## Week 1: Facts Extraction & Cache

### Tasks

- [ ] Create `crates/swe-grep-core/src/analysis/mod.rs` module
- [ ] Define `FileFacts` struct matching JSON schema above
- [ ] Implement per-file cache in `.swe-grep-cache/facts/` keyed by `{hash(path)}.json`
- [ ] Cache invalidation: compare `(mtime, size)` before reusing
- [ ] Implement Rust extractor using tree-sitter patterns:
  - Definitions: `struct_item`, `fn_item`, `impl_item`, `trait_item`, `enum_item`
  - References: `identifier` nodes in non-definition contexts
  - Calls: `call_expression` with callee resolution
- [ ] Implement Swift extractor:
  - Definitions: `class_declaration`, `struct_declaration`, `protocol_declaration`, `func_declaration`
  - References: `simple_identifier` in usage contexts
  - Calls: `call_expression`
- [ ] Implement TypeScript/TSX extractor:
  - Definitions: `class_declaration`, `function_declaration`, `interface_declaration`, `type_alias_declaration`
  - References: `identifier` nodes
  - Calls: `call_expression`
- [ ] Add `--extract-facts` hidden flag to `search` for debugging
- [ ] Write tests with fixture files for each language

### Deliverables

```bash
# Debug command to see extracted facts
swe-grep search --symbol Foo --extract-facts --path .

# Output includes:
{
  "hits": [...],
  "facts_extracted": 42,
  "facts_cached": 38,
  "facts_cache_hit_rate": 0.90
}
```

---

## Week 2: Call Graph & Type Relations

### Call Graph Tasks

- [ ] Create `analysis/callgraph.rs` module
- [ ] Build in-memory graph: `HashMap<Symbol, Vec<CallEdge>>`
- [ ] Implement `swe-grep refs --callers --symbol Foo` using graph
- [ ] Implement `swe-grep refs --callees --symbol Foo` using graph
- [ ] Add `--depth N` for transitive closure (default: 1)
- [ ] Add `--max-nodes N` to limit output (default: 50)
- [ ] Add `--include-tests` flag (excluded by default)
- [ ] Add `--focus-file path` to filter graph to single file's perspective
- [ ] Output formats: `--format json|tree|compact`

```bash
# Example: find all callers of run_cycle, 2 levels deep
swe-grep refs --callers --symbol run_cycle --depth 2 --format tree --path .

# Output:
run_cycle
├── execute_search (search/mod.rs:45)
│   ├── main (main.rs:12)
│   └── bench_search (bench.rs:89)
└── run_batch (batch.rs:23)
    └── main (main.rs:15)
```

### Type Relations Tasks

- [ ] Create `analysis/types.rs` module
- [ ] New command: `swe-grep types --symbol Foo`
- [ ] Extract relations by language:
  - **Rust:** `impl Trait for Type`, `impl Type`, trait bounds
  - **Swift:** `class X: Parent`, `struct X: Protocol`, `extension X`
  - **TS:** `class X extends Y`, `class X implements I`, `interface I extends J`
- [ ] Relation kinds: `extends`, `implements`, `conforms_to`, `impl_trait`, `extension_of`
- [ ] Add `--direction up|down|both` (inheritance direction)
- [ ] Output formats: `--format json|tree|compact`

```bash
# Example: find all types conforming to CloudKitRecordConvertible
swe-grep types --symbol CloudKitRecordConvertible --direction down --format tree --path .

# Output:
CloudKitRecordConvertible (protocol)
├── AINote (conforms_to, Models/AINote.swift:12)
├── FlashCardDeck (conforms_to, Models/FlashCard.swift:45)
└── QuizLesson (conforms_to, Models/Quiz.swift:23)
```

### Tests

- [ ] Rust: test trait impl detection, generic bounds
- [ ] Swift: test protocol conformance, class inheritance, extensions
- [ ] TS: test class extends, implements, interface extends
- [ ] Graph: test cycle detection, depth limiting, pruning

---

## Week 3: Semantic Search

### Decision Required

**Option A: Offline Embeddings (Recommended)**
- Embed `fastembed` or `ort` (ONNX runtime) with `all-MiniLM-L6-v2` model
- ~25MB binary size increase
- Zero network dependency, works offline
- Index stored in `.swe-grep-cache/vectors/`

**Option B: Online Provider**
- Support OpenAI, Anthropic, or local Ollama
- Requires `--embedding-provider` config
- Smaller binary, network dependent

### Tasks (assuming Option A)

- [ ] Add `fastembed` or `ort` dependency
- [ ] Create `analysis/semantic.rs` module
- [ ] Implement AST-aware chunking:
  - Chunk by function/method/class boundaries
  - Fallback to 512-token sliding window with 128 overlap
  - Store: `{chunk_id, file, start_line, end_line, embedding}`
- [ ] Build HNSW index using `usearch` or `hnsw` crate
- [ ] Persist index to `.swe-grep-cache/vectors.idx`
- [ ] Index invalidation: rebuild chunks for changed files only
- [ ] New flag: `search --semantic "query"`
- [ ] Hybrid ranking: `0.7 * embedding_score + 0.3 * keyword_score`
- [ ] Add `--explain` to show why chunks matched (top tokens)

```bash
# Example: semantic search
swe-grep search --semantic "how does authentication work" --path .

# Output:
{
  "hits": [
    {
      "file": "src/auth/login.rs",
      "line": 45,
      "snippet": "pub async fn authenticate_user(...)",
      "score": 0.89,
      "explanation": "matched: authenticate, user, login, session"
    }
  ]
}
```

### Indexing Command

- [ ] Add `swe-grep index --semantic --path .` to pre-build index
- [ ] Show progress: `Indexing 234 files... [████████░░] 80%`
- [ ] Add `--reindex` to force full rebuild

---

## Week 4: Git Integration & Watch Mode

### Git Blame Tasks

- [ ] Add `--git-blame` flag to `search` and `refs`
- [ ] For each hit, run `git blame -L {line},{line} --porcelain {file}`
- [ ] Parse: author, author-mail, author-time, summary (commit message)
- [ ] Add to hit JSON: `"blame": {"author": "alice", "date": "2025-12-10", "commit": "abc123", "message": "Fix auth bug"}`
- [ ] Batch blame calls for performance (group by file)

### Git Branch Tasks

- [ ] Add `--git-branch <branch>` flag
- [ ] Compute files changed since branch point: `git diff --name-only $(git merge-base HEAD <branch>)..HEAD`
- [ ] Filter results to only those files
- [ ] Add metadata: `"git": {"branch": "feature-x", "files_changed": 12}`

### Watch Mode Tasks

- [ ] New command: `swe-grep watch <subcommand> [args]`
- [ ] Use `notify` crate for filesystem events
- [ ] Debounce: 100ms after last change before re-running
- [ ] Incremental: only re-extract facts for changed files
- [ ] Stream output as NDJSON (compatible with `--stream`)
- [ ] Support: `watch search`, `watch refs`, `watch remove --assert-absent`

```bash
# Watch for symbol removal during refactoring
swe-grep watch search --symbol OldClassName --path .
# Streams updates as files change, exits 0 when symbol gone

# Watch call graph as you refactor
swe-grep watch refs --callers --symbol handleAuth --path .
```

### Polish & Performance

- [ ] Benchmark all new commands against 10k+ file repos
- [ ] Add `--parallel` control for fact extraction
- [ ] Profile and optimize hot paths (tree-sitter parsing, cache I/O)
- [ ] Add integration tests for full workflows
- [ ] Update README and `--help` text
- [ ] Add examples to CLAUDE.md

---

## File Structure

```
crates/swe-grep-core/src/
├── analysis/
│   ├── mod.rs           # FileFacts, AnalysisCache
│   ├── extract.rs       # Language-agnostic extraction coordinator
│   ├── rust.rs          # Rust-specific tree-sitter patterns
│   ├── swift.rs         # Swift-specific patterns
│   ├── typescript.rs    # TS/TSX patterns
│   ├── callgraph.rs     # Graph building and traversal
│   ├── types.rs         # Type relation extraction
│   └── semantic.rs      # Embedding + vector index
├── commands/
│   ├── refs.rs          # Updated with --callers/--callees/--depth
│   ├── types.rs         # New command
│   └── watch.rs         # New command
└── git/
    ├── blame.rs         # Git blame integration
    └── branch.rs        # Branch-aware filtering
```

---

## Dependencies to Add

```toml
[dependencies]
# Tree-sitter for AST parsing
tree-sitter = "0.22"
tree-sitter-rust = "0.21"
tree-sitter-swift = "0.4"  # or custom grammar
tree-sitter-typescript = "0.21"

# Vector search (Option A)
fastembed = "3"  # or ort = "2" for ONNX
usearch = "2"    # HNSW index

# File watching
notify = "6"
notify-debouncer-mini = "0.4"

# Git integration (optional, can use Command)
# git2 = "0.18"  # Later optimization
```

---

## Success Criteria

| Feature | Metric | Target |
|---------|--------|--------|
| Facts cache | Hit rate after warm-up | > 90% |
| Call graph | Query latency (depth=1) | < 50ms |
| Call graph | Query latency (depth=3) | < 200ms |
| Type relations | Accuracy on test fixtures | > 95% |
| Semantic search | Relevance (top-5) | > 80% |
| Semantic indexing | Throughput | > 100 files/sec |
| Git blame | Per-hit overhead | < 5ms |
| Watch mode | Re-run latency after edit | < 100ms |

---

## Questions Resolved

1. **Primary language focus:** Swift first (user's main use case), then Rust, then TS/TSX
2. **Semantic search:** Offline embeddings (Option A) for best UX

---

## Getting Started

```bash
# Start with the analysis module skeleton
mkdir -p crates/swe-grep-core/src/analysis
touch crates/swe-grep-core/src/analysis/{mod,extract,rust,swift,typescript,callgraph,types,semantic}.rs

# Add tree-sitter dependencies
cargo add tree-sitter tree-sitter-rust

# Run existing tests to ensure nothing breaks
cargo test

# Begin Week 1: FileFacts struct and cache
```
