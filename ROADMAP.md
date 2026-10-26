# SWE-Grep Roadmap

> Historical design/review record. Status, commands, measurements and licensing
> descriptions may refer to an older revision or proposal. Use the
> [documentation map](docs/README.md), [contributor guide](CONTRIBUTING.md) and
> [LICENSE](LICENSE) for current guidance.

## Phase 1: Polish & Quick Wins (1-2 weeks)

Foundation improvements that require minimal architectural changes.

### 1.1 Config File Support
- [ ] Add `~/.swe-grep.toml` for persistent defaults
- [ ] Support project-local `.swe-grep.toml` override
- [ ] Config options: default language, cache-dir, body-limit, exclude patterns

```toml
# ~/.swe-grep.toml
[defaults]
language = "swift"
cache_dir = "~/.cache/swe-grep"
body_limit = 100000
code_only = true

[exclude]
patterns = ["*.md", "PLAN*", "node_modules"]
```

### 1.2 Editor Integration
- [ ] `--open` flag to launch editor at hit location
- [ ] `--open-command` template: `code -g {path}:{line}`, `nvim +{line} {path}`
- [ ] Detect `$EDITOR` environment variable as fallback

### 1.3 Shell Completions
- [ ] Generate completions for bash, zsh, fish via `clap_complete`
- [ ] `swe-grep completions --shell zsh > _swe-grep`

### 1.4 Output Formats
- [ ] `--format markdown` for documentation/reports
- [ ] `--format csv` for spreadsheet analysis
- [ ] `--json-lines` streaming (one JSON object per hit)

### 1.5 Smarter Fuzzy Suggestions
- [ ] Improve "Did you mean" with Levenshtein + frequency ranking
- [ ] Show top 3 suggestions with match percentage
- [ ] Auto-suggest when 0 hits and close matches exist

---

## Phase 2: Intelligence & Analysis (3-4 weeks)

Add semantic understanding and code relationship features.

### 2.1 Call Graph Analysis ✅ COMPLETED
- [x] `swe-grep refs X . --callers` - find all call sites
- [x] `swe-grep refs X . --callees` - find all functions X calls
- [x] `swe-grep refs X .` - bidirectional references
- [x] Cache call graph in `.swe-grep-cache/` (per-file fact caching)
- [x] **Qualified name preservation** - `Foo::new` matches only `Foo::new`, not all `new()` calls
- [x] **Dual indexing** - supports both qualified (`Type::method`) and simple (`method`) lookups

**Implementation (DONE):**
```
1. Tree-sitter parse all files (rust.rs, swift.rs, typescript.rs)
2. Extract function definitions + call expressions with full qualification
3. Build adjacency list with dual indexing (qualified + simple names)
4. Per-file caching with modification-time invalidation
```

**Key files:**
- `analysis/callgraph.rs` - CallGraph and CallGraphView structures
- `analysis/rust.rs` - Rust-specific callee extraction with `extract_qualified_callee()`
- `analysis/extract.rs` - Repository-wide fact extraction

### 2.2 Type Hierarchy Search ✅ COMPLETED
- [x] TypeGraph for tracking type relationships (conformance, inheritance, implementation)
- [x] Support for Swift protocol conformance
- [x] Support for TypeScript extends/implements
- [x] Support for Rust traits (via relations extraction)

**Key files:**
- `analysis/types.rs` - TypeGraph and TypeGraphView structures
- `analysis/typescript.rs` - TypeScript relation extraction (extends, implements)
- `analysis/swift.rs` - Swift conformance extraction

### 2.3 Semantic/Embeddings Search
- [ ] Integrate local embedding model (all-MiniLM-L6-v2 or similar)
- [ ] Index code chunks with 384-dim embeddings
- [ ] `--semantic "how does auth work"` for similarity search
- [ ] Hybrid ranking: keyword score + embedding similarity

**Implementation:**
```
1. Chunk files by function/class boundaries
2. Generate embeddings via ONNX runtime (no network)
3. Store in HNSW index for fast ANN search
4. Query: embed query -> find top-k similar chunks
```

### 2.4 Git Integration
- [ ] `--git-blame` - attach author/date to each hit
- [ ] `--git-since "1 week ago"` - search only recent changes
- [ ] `--git-branch feature` - search files changed on branch
- [ ] Show diff hunks when `--git-modified` is used

### 2.5 Watch Mode
- [ ] `swe-grep watch --assert-absent OldSymbol --path .`
- [ ] Continuous file watching with debounce
- [ ] Exit 0 when assertion passes, exit 1 while symbol exists
- [ ] Useful for refactoring validation

---

## Phase 3: Platform & Ecosystem (4-6 weeks)

Transform swe-grep into a platform for code intelligence.

### 3.1 Interactive TUI
- [ ] `swe-grep search --interactive` launches TUI
- [ ] Arrow keys navigate hits, Enter expands body
- [ ] `e` opens in editor, `u` shows usages, `d` shows definition
- [ ] `/` to filter results, `q` to quit
- [ ] Built with `ratatui` crate

### 3.2 LSP Server Mode
- [ ] `swe-grep lsp --port 9999` or stdio mode
- [ ] Implement `textDocument/references`
- [ ] Implement `textDocument/definition`
- [ ] Implement `workspace/symbol`
- [ ] VSCode extension wrapper

### 3.3 Result Pagination & Cursors
- [ ] `--cursor N` to resume from offset
- [ ] Response includes `next_cursor` for pagination
- [ ] `--limit` separate from `--max-matches` for total cap
- [ ] Streaming mode with backpressure

### 3.4 Benchmark Dashboard
- [ ] `swe-grep bench --output report.html`
- [ ] Compare against ripgrep, ast-grep, grep
- [ ] Latency histograms, P50/P95/P99
- [ ] Repository size scaling analysis

### 3.5 Plugin System
- [ ] Language plugins for custom AST patterns
- [ ] Output format plugins
- [ ] Custom rankers/scorers
- [ ] WASM-based plugin runtime

### 3.6 Cloud/Team Features (Optional)
- [ ] `swe-grep index --upload` to shared cache
- [ ] Team-wide symbol index for monorepos
- [ ] Search across multiple repositories

---

## Success Metrics

| Phase | Metric | Target |
|-------|--------|--------|
| 1 | User config adoption | 50% of users have .swe-grep.toml |
| 1 | Editor integration | --open used in 30% of searches |
| 2 | Call graph queries | 20% of searches use callers/callees |
| 2 | Semantic search accuracy | 80% relevance in top-5 |
| 3 | TUI adoption | 15% of sessions use interactive mode |
| 3 | LSP integration | 3+ editor plugins |

---

## Dependencies by Phase

**Phase 1:**
- `clap_complete` (shell completions)
- `dirs` (config file paths)
- `toml` (config parsing)

**Phase 2:**
- `tree-sitter` + language grammars
- `ort` (ONNX runtime for embeddings)
- `hnsw` or `usearch` (vector index)
- `notify` (file watching)

**Phase 3:**
- `ratatui` + `crossterm` (TUI)
- `tower-lsp` (LSP server)
- `plotters` (benchmark charts)
- `wasmtime` (plugin runtime)

---

## Prioritization Matrix

```
                    HIGH IMPACT
                        │
    ┌───────────────────┼───────────────────┐
    │                   │                   │
    │  Config File      │  Call Graph       │
    │  Editor --open    │  Semantic Search  │
    │  Fuzzy Suggest    │  Type Hierarchy   │
    │                   │                   │
LOW ├───────────────────┼───────────────────┤ HIGH
EFFORT                  │                   EFFORT
    │                   │                   │
    │  Shell Complete   │  TUI Interactive  │
    │  --format md/csv  │  LSP Server       │
    │  --json-lines     │  Plugin System    │
    │                   │                   │
    └───────────────────┼───────────────────┘
                        │
                    LOW IMPACT
```

**Recommendation:** Start with top-left quadrant (high impact, low effort), then move to top-right.
