# SWE-Grep: Critical Improvements

## Executive Summary

After deep analysis, I've identified **three fundamental issues** that prevent swe-grep from being a truly exceptional code intelligence tool:

1. **Call Graph Accuracy**: The call graph loses type qualifiers, making it match unrelated calls
2. **No Symbol Intelligence**: The tool finds text matches but doesn't understand what symbols *are*
3. **No Smart Defaults for AI Agents**: Output optimized for humans, not for LLM consumption

Fixing these would transform swe-grep from "fast grep" to "instant code understanding."

---

## Issue 1: Call Graph Loses Type Qualifiers

### Problem

When extracting calls, the tool normalizes `SearchEngine::new()` to just `new()`. This causes:

```bash
$ swe-grep refs --symbol "SearchEngine::new" --callers --path .
# Returns: Vec::new(), HashSet::new(), String::new()... (all false positives!)
```

**Root cause**: `normalize_symbol()` in `callgraph.rs:202` strips the type prefix:

```rust
// Current: "SearchEngine::new" → "new"
// Should be: "SearchEngine::new" → "SearchEngine::new"
```

### Solution

Preserve qualified names in the call graph. Match by suffix when the query is unqualified.

```rust
// Call storage:
//   caller: "SearchEngine::run_cycle"
//   callee: "SearchConfig::try_from_args"  // preserve full path

// Query matching:
//   "new" → matches "SearchEngine::new", "Vec::new", etc.
//   "SearchEngine::new" → matches only "SearchEngine::new"
//   "SearchEngine" → matches callers that USE SearchEngine (constructors, methods)
```

### Implementation

```rust
// In rust.rs, when extracting call:
fn callee_text(source: &str, call_node: &Node) -> Option<String> {
    if call_node.kind() == "call_expression" {
        if let Some(func) = call_node.child_by_field_name("function") {
            // Extract FULL path: SearchEngine::new, not just new
            return extract_full_call_path(source, func);
        }
    }
    // ... rest
}

fn extract_full_call_path(source: &str, node: Node) -> Option<String> {
    match node.kind() {
        "scoped_identifier" => {
            // Type::method format
            let path = node.child_by_field_name("path")?;
            let name = node.child_by_field_name("name")?;
            Some(format!("{}::{}", node_text(source, path)?, node_text(source, name)?))
        }
        "field_expression" => {
            // self.method() or object.method()
            let field = node.child_by_field_name("field")?;
            node_text(source, field)
        }
        _ => node_text(source, node)
    }
}
```

### Impact

- **Before**: Call graph is 90%+ false positives
- **After**: Call graph shows actual relationships

---

## Issue 2: No Symbol Intelligence

### Problem

When I search for `SearchEngine`, I want to understand *what it is*, not just where it appears:

```bash
# Current: Returns 5 text matches
$ swe-grep search --symbol SearchEngine --path .

# Desired: Returns structured understanding
$ swe-grep explain --symbol SearchEngine --path .
```

### Solution

Add an `explain` command that provides structured symbol intelligence:

```json
{
  "symbol": "SearchEngine",
  "kind": "struct",
  "file": "crates/swe-grep-core/src/search/engine.rs",
  "line": 62,
  "visibility": "pub(crate)",
  "doc": "A search engine combining multiple search strategies...",
  "fields": [
    {"name": "config", "type": "SearchConfig"},
    {"name": "rg_tool", "type": "RipgrepTool"},
    {"name": "ast_tool", "type": "Option<AstGrepTool>"}
  ],
  "methods": [
    {"name": "new", "signature": "fn new(config: SearchConfig) -> Result<Self>"},
    {"name": "run_cycle", "signature": "async fn run_cycle(&mut self) -> Result<SearchSummary>"}
  ],
  "implements": [],
  "used_in_files": ["agent.rs", "main.rs", "service/server.rs"],
  "created_by": ["execute_search:agent.rs:85"],
  "complexity": {
    "fields": 15,
    "methods": 12,
    "lines": 2100
  }
}
```

### Implementation

1. **Extend fact extraction** to capture:
   - Field definitions for structs
   - Method signatures
   - Doc comments
   - Visibility modifiers

2. **New `explain` command**:
   ```rust
   #[derive(Parser)]
   pub struct ExplainCommand {
       #[arg(long)]
       symbol: String,
       #[arg(long, default_value = ".")]
       path: PathBuf,
       #[arg(long, default_value = "json")]
       format: OutputFormat,
   }
   ```

3. **Symbol aggregation**:
   ```rust
   fn explain_symbol(symbol: &str, facts: &[FileFacts]) -> SymbolExplanation {
       // Find definition
       let definition = facts.iter()
           .flat_map(|f| f.facts.definitions.iter())
           .find(|d| d.symbol == symbol);

       // Find methods (for struct/class)
       let methods = if definition.map(|d| d.kind == "struct").unwrap_or(false) {
           find_impl_methods(symbol, facts)
       } else {
           vec![]
       };

       // Find usages
       let usages = find_usages(symbol, facts);

       // Build explanation
       SymbolExplanation { definition, methods, usages, ... }
   }
   ```

### Impact

- **Before**: AI agent gets 5 file:line snippets, must read each
- **After**: AI agent gets complete symbol understanding in one call

---

## Issue 3: Output Not Optimized for AI Agents

### Problem

Current `--agent` mode returns raw hits. For LLM consumption, we need:

1. **Fewer tokens** - Remove redundant context
2. **Structured data** - Easy to parse and reason about
3. **Actionable info** - What can the AI *do* with this?

### Current Output (verbose, repetitive)

```json
{
  "hits": [
    {"file": "engine.rs", "line": 62, "snippet": "pub(crate) struct SearchEngine {", "body": "...2000 chars..."},
    {"file": "engine.rs", "line": 85, "snippet": "impl SearchEngine {", "body": "...2000 chars..."},
    {"file": "engine.rs", "line": 2162, "snippet": "let mut engine = super::SearchEngine::new(config)...", "body": "..."}
  ]
}
```

### Proposed Output (structured, minimal)

```json
{
  "symbol": "SearchEngine",
  "definition": {
    "kind": "struct",
    "file": "engine.rs:62",
    "preview": "pub(crate) struct SearchEngine { config: SearchConfig, ... }"
  },
  "implementation": {
    "file": "engine.rs:85",
    "methods": ["new", "run_cycle", "verify", "disambiguate"],
    "method_count": 15
  },
  "usages": {
    "count": 12,
    "files": ["agent.rs", "main.rs", "server.rs"],
    "top_usage": "agent.rs:85: let mut engine = SearchEngine::new(config)?;"
  },
  "relations": {
    "uses": ["SearchConfig", "RipgrepTool", "AstGrepTool"],
    "used_by": ["execute_search", "run_batch"]
  }
}
```

### Implementation

Add `--smart` flag to search that enables intelligent aggregation:

```bash
swe-grep search --symbol SearchEngine --smart --path .
```

The `--smart` flag:
1. Groups hits by semantic role (definition, impl, usage)
2. Extracts relationships automatically
3. Reduces output size by 80%
4. Provides actionable structure

---

## Priority Order

### P0: Call Graph Accuracy (2-3 days)
- Fix `normalize_symbol()` to preserve type qualifiers
- Update call extraction to capture full paths
- Add tests for qualified method matching

### P1: Smart Search Mode (3-4 days)
- Add `--smart` flag to search
- Implement hit aggregation by role
- Extract relations from hits
- Reduce output verbosity

### P2: Explain Command (4-5 days)
- Extend fact extraction for fields/methods
- Add `explain` command
- Build symbol aggregation logic
- Add format options (json, markdown)

---

## Quick Wins (< 1 day each)

### 1. Better Ranking
Currently definitions get a small bonus. Make it definitive:
```rust
const DEFINITION_PRIORITY: i32 = 1000;  // Always first
const IMPL_PRIORITY: i32 = 500;         // Second
const DIRECT_USAGE_PRIORITY: i32 = 100; // Third
const STRING_MENTION_PRIORITY: i32 = 1; // Last
```

### 2. Exclude String Literals by Default
Most searches don't want to find `"SearchEngine"` in error messages:
```bash
# Current: includes string matches
# Proposed: exclude by default, add --include-strings to opt-in
```

### 3. Show Method Context for Struct Searches
When searching for a struct, also show its key methods:
```
SearchEngine (struct, engine.rs:62)
├── new(config: SearchConfig) -> Result<Self>
├── run_cycle(&mut self) -> Result<SearchSummary>
└── 13 more methods...
```

### 4. Fuzzy Symbol Suggestions
When no exact match, suggest similar symbols:
```
$ swe-grep search --symbol SearchEngin --path .
No exact matches. Did you mean:
  1. SearchEngine (92% match)
  2. SearchConfig (78% match)
```

---

## Success Metrics

| Metric | Current | Target |
|--------|---------|--------|
| Call graph accuracy | ~10% | >95% |
| Tokens per search result | ~5000 | <1000 |
| Time to understand symbol | 3-4 searches | 1 search |
| False positive rate | ~40% | <5% |

---

## Conclusion

These improvements transform swe-grep from a "fast text search" tool into an "instant code understanding" tool. The key insight is that AI agents don't need more hits—they need *understanding*.

The call graph fix is critical and should be done first. It's blocking the `refs` and `types` commands from being useful.
