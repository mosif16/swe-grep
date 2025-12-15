# Context Retrieval: Task Breakdown for Builders

**Goal**: Make swe-grep the #1 agent search tool by enabling **instant code understanding** in a single call.

**Philosophy**: Agents don't search for text—they search for understanding. One query should return everything needed to reason about code.

---

## Architecture Overview

```
┌────────────────────────────────────────────────────────────────┐
│                     swe-grep context                            │
│                                                                 │
│  ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐  │
│  │ Symbol   │───▶│ Import   │───▶│ Call     │───▶│ Context  │  │
│  │ Discovery│    │ Graph    │    │ Graph    │    │ Assembly │  │
│  └──────────┘    └──────────┘    └──────────┘    └──────────┘  │
│       │               │               │               │         │
│       ▼               ▼               ▼               ▼         │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │              Unified Agent Response                      │   │
│  │  • Definition + signature + docstring                    │   │
│  │  • Callers (with signatures)                             │   │
│  │  • Callees (with signatures)                             │   │
│  │  • Type relationships                                    │   │
│  │  • Import chain                                          │   │
│  │  • Related tests                                         │   │
│  └─────────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────┘
```

---

## Epic 1: Import Graph Foundation

**Why**: Agents need to understand module boundaries and dependencies to retrieve relevant context.

### Task 1.1: Import Fact Extraction

**Owner**: _unassigned_
**Effort**: 2-3 days
**Priority**: P0

**Description**: Extract import/export statements from source files using tree-sitter.

**Files to Create/Modify**:
- `crates/swe-grep-core/src/analysis/imports.rs` (NEW)
- `crates/swe-grep-core/src/analysis/mod.rs` (add module)
- `crates/swe-grep-core/src/analysis/rust.rs` (add import extraction)
- `crates/swe-grep-core/src/analysis/typescript.rs` (add import extraction)
- `crates/swe-grep-core/src/analysis/swift.rs` (add import extraction)

**Data Structures**:
```rust
// imports.rs

#[derive(Debug, Clone, Serialize)]
pub struct ImportFact {
    pub file: String,           // File containing the import
    pub line: usize,
    pub kind: ImportKind,
    pub source: String,         // Module/file being imported from
    pub symbols: Vec<String>,   // Specific symbols imported (empty = wildcard)
    pub alias: Option<String>,  // `import X as Y`
}

#[derive(Debug, Clone, Serialize)]
pub enum ImportKind {
    Named,      // import { Foo } from 'bar'
    Default,    // import Foo from 'bar'
    Namespace,  // import * as Foo from 'bar'
    SideEffect, // import 'bar'
    Reexport,   // export { Foo } from 'bar'
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportFact {
    pub file: String,
    pub line: usize,
    pub symbol: String,
    pub kind: ExportKind,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
pub enum ExportKind {
    Function,
    Class,
    Type,
    Interface,
    Const,
    Enum,
    Reexport,
}
```

**Tree-sitter Queries**:

```scheme
;; TypeScript imports
(import_statement
  source: (string) @source
  (import_clause
    (named_imports
      (import_specifier name: (identifier) @symbol))?
    (identifier)? @default)) @import

;; Rust imports
(use_declaration
  argument: (scoped_identifier) @path) @use

;; Swift imports
(import_declaration
  path: (identifier) @module) @import
```

**Acceptance Criteria**:
- [ ] Extracts imports from Rust (`use`, `mod`, `extern crate`)
- [ ] Extracts imports from TypeScript/JavaScript (`import`, `require`, `export`)
- [ ] Extracts imports from Swift (`import`)
- [ ] Returns `imports: Vec<ImportFact>` in `FactBundle`
- [ ] Unit tests for each language with edge cases

**Test Command**:
```bash
cargo test -p swe-grep -- imports
```

---

### Task 1.2: Import Graph Construction

**Owner**: _unassigned_
**Effort**: 2 days
**Priority**: P0
**Depends on**: Task 1.1

**Description**: Build a queryable graph from import facts.

**Files to Create/Modify**:
- `crates/swe-grep-core/src/analysis/import_graph.rs` (NEW)

**Data Structures**:
```rust
// import_graph.rs

pub struct ImportEdge {
    pub importer: String,       // File that imports
    pub imported: String,       // File/module being imported
    pub symbols: Vec<String>,   // Specific symbols (empty = all)
    pub line: usize,
}

pub struct ImportGraph {
    edges: Vec<ImportEdge>,
    /// file -> indices of edges where file is importer
    outgoing: HashMap<String, Vec<usize>>,
    /// file -> indices of edges where file is imported
    incoming: HashMap<String, Vec<usize>>,
    /// symbol -> files that export it
    exports: HashMap<String, Vec<String>>,
}

impl ImportGraph {
    pub fn from_facts(files: &[FileFacts]) -> Self;

    /// What does this file import?
    pub fn imports_of(&self, file: &str) -> Vec<&ImportEdge>;

    /// What files import this file?
    pub fn importers_of(&self, file: &str) -> Vec<&ImportEdge>;

    /// Where is this symbol exported from?
    pub fn find_export(&self, symbol: &str) -> Vec<&str>;

    /// Transitive imports (depth-limited)
    pub fn transitive_imports(&self, file: &str, depth: usize) -> Vec<String>;

    /// Transitive importers (who depends on this, transitively)
    pub fn transitive_importers(&self, file: &str, depth: usize) -> Vec<String>;
}
```

**Acceptance Criteria**:
- [ ] Builds graph from `FileFacts` in < 100ms for 10K files
- [ ] Resolves relative imports to absolute paths
- [ ] Handles re-exports correctly
- [ ] Provides transitive traversal with depth limit
- [ ] Memory-efficient (no full file content in graph)

---

### Task 1.3: Import Graph CLI Integration

**Owner**: _unassigned_
**Effort**: 1 day
**Priority**: P0
**Depends on**: Task 1.2

**Description**: Expose import graph via `swe-grep refs` subcommand.

**Files to Modify**:
- `crates/swe-grep-core/src/cli.rs` (add flags)
- `crates/swe-grep-core/src/main.rs` (handle in `handle_refs`)

**New Flags**:
```rust
/// Show what this file/symbol imports
#[arg(long)]
pub imports: bool,

/// Show what imports this file/symbol
#[arg(long)]
pub importers: bool,

/// Depth for transitive traversal (default: 1)
#[arg(long, default_value = "1")]
pub import_depth: usize,
```

**CLI Examples**:
```bash
# What does this file import?
swe-grep refs --imports --symbol src/main.rs --path .

# What files import AuthService?
swe-grep refs --importers --symbol AuthService --path .

# Transitive dependency chain (3 levels deep)
swe-grep refs --imports --import-depth 3 --symbol src/api/handler.rs --path .
```

**Output Schema Addition**:
```json
{
  "imports": [
    {"file": "src/auth.rs", "symbols": ["AuthService", "Token"], "line": 3}
  ],
  "importers": [
    {"file": "src/main.rs", "symbols": ["AuthService"], "line": 12}
  ]
}
```

**Acceptance Criteria**:
- [ ] `--imports` shows direct imports
- [ ] `--importers` shows direct importers
- [ ] `--import-depth N` enables transitive traversal
- [ ] Works with both file paths and symbol names
- [ ] JSON output matches schema

---

## Epic 2: Smart Context Assembly

**Why**: This is the killer feature. One call returns everything an agent needs.

### Task 2.1: Context Command Scaffold

**Owner**: _unassigned_
**Effort**: 1 day
**Priority**: P0

**Description**: Add `swe-grep context` subcommand with basic structure.

**Files to Create/Modify**:
- `crates/swe-grep-core/src/cli.rs` (add Context subcommand)
- `crates/swe-grep-core/src/main.rs` (add `handle_context` function)
- `crates/swe-grep-core/src/context/mod.rs` (NEW)

**CLI Definition**:
```rust
#[derive(Args, Debug)]
pub struct ContextArgs {
    /// Symbol to get context for
    #[arg(long, required = true)]
    pub symbol: String,

    /// Root path to search
    #[arg(long, default_value = ".")]
    pub path: PathBuf,

    /// Depth for relationship traversal (default: 1)
    #[arg(long, default_value = "1")]
    pub depth: usize,

    /// Maximum total output lines (default: 500)
    #[arg(long, default_value = "500")]
    pub max_lines: usize,

    /// Include test files in context
    #[arg(long)]
    pub include_tests: bool,

    /// Language hint
    #[arg(long)]
    pub language: Option<String>,

    /// Output format (json, summary)
    #[arg(long, default_value = "json")]
    pub format: String,
}
```

**Acceptance Criteria**:
- [ ] `swe-grep context --symbol Foo --path .` parses correctly
- [ ] Stub implementation returns empty context response
- [ ] Help text clearly explains purpose

---

### Task 2.2: Context Response Schema

**Owner**: _unassigned_
**Effort**: 1 day
**Priority**: P0
**Depends on**: Task 2.1

**Description**: Define the unified context response that agents will consume.

**Files to Create**:
- `crates/swe-grep-core/src/context/response.rs` (NEW)

**Data Structures**:
```rust
#[derive(Debug, Serialize)]
pub struct ContextResponse {
    pub version: u32,                    // Schema version (1)
    pub cli_version: String,
    pub symbol: String,                  // Queried symbol
    pub definition: Option<Definition>,  // Primary definition
    pub callers: Vec<Caller>,            // Who calls this
    pub callees: Vec<Callee>,            // What this calls
    pub types: TypeContext,              // Type relationships
    pub imports: ImportContext,          // Import/export chain
    pub tests: Vec<TestFile>,            // Related test files
    pub summary: ContextSummary,
    pub warnings: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct Definition {
    pub file: String,
    pub line: usize,
    pub kind: String,                    // "function", "struct", "class", etc.
    pub signature: String,               // fn foo(x: i32) -> String
    pub docstring: Option<String>,       // Doc comment if present
    pub body: Option<String>,            // Trimmed body (configurable)
    pub body_lines: usize,               // Total lines (even if trimmed)
}

#[derive(Debug, Serialize)]
pub struct Caller {
    pub file: String,
    pub line: usize,
    pub function: String,                // Calling function name
    pub signature: Option<String>,       // Caller's signature
    pub snippet: String,                 // Call site context
}

#[derive(Debug, Serialize)]
pub struct Callee {
    pub symbol: String,
    pub file: Option<String>,            // None if external/stdlib
    pub line: Option<usize>,
    pub signature: Option<String>,
    pub call_count: usize,               // How many times called
}

#[derive(Debug, Serialize)]
pub struct TypeContext {
    pub implements: Vec<TypeRelation>,   // Traits/interfaces implemented
    pub extended_by: Vec<TypeRelation>,  // Types that extend this
    pub contains: Vec<TypeRelation>,     // Fields/properties of this type
}

#[derive(Debug, Serialize)]
pub struct TypeRelation {
    pub symbol: String,
    pub file: String,
    pub line: usize,
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub struct ImportContext {
    pub exported_from: Vec<String>,      // Where symbol is exported
    pub imported_by: Vec<ImportSite>,    // Files that import this
    pub dependencies: Vec<String>,       // What this file imports
}

#[derive(Debug, Serialize)]
pub struct ImportSite {
    pub file: String,
    pub line: usize,
    pub alias: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TestFile {
    pub file: String,
    pub test_functions: Vec<String>,     // Test function names
    pub relevance: f32,                  // 0-1 relevance score
}

#[derive(Debug, Serialize)]
pub struct ContextSummary {
    pub definition_found: bool,
    pub caller_count: usize,
    pub callee_count: usize,
    pub type_relations: usize,
    pub import_sites: usize,
    pub test_files: usize,
    pub total_lines: usize,
    pub latency_ms: u64,
}
```

**Acceptance Criteria**:
- [ ] Schema serializes to clean JSON
- [ ] All fields documented with serde attributes
- [ ] Example output documented in code comments

---

### Task 2.3: Context Assembly Engine

**Owner**: _unassigned_
**Effort**: 3-4 days
**Priority**: P0
**Depends on**: Tasks 1.2, 2.2, and existing call/type graphs

**Description**: Implement the core logic that assembles context from multiple sources.

**Files to Create/Modify**:
- `crates/swe-grep-core/src/context/engine.rs` (NEW)
- `crates/swe-grep-core/src/context/mod.rs`

**Core Algorithm**:
```rust
pub struct ContextEngine {
    call_graph: CallGraph,
    type_graph: TypeGraph,
    import_graph: ImportGraph,
    file_cache: FileCache,
}

impl ContextEngine {
    pub async fn assemble(
        &self,
        symbol: &str,
        config: &ContextConfig,
    ) -> Result<ContextResponse> {
        // 1. Find definition
        let definition = self.find_definition(symbol).await?;

        // 2. Get callers (from call graph)
        let callers = self.call_graph
            .caller_edges_for(symbol)
            .into_iter()
            .take(config.max_callers)
            .map(|e| self.enrich_caller(e))
            .collect();

        // 3. Get callees (from call graph)
        let callees = self.call_graph
            .callee_edges_for(symbol)
            .into_iter()
            .take(config.max_callees)
            .map(|e| self.enrich_callee(e))
            .collect();

        // 4. Get type relationships (from type graph)
        let types = self.type_graph
            .view(symbol, TypeDirection::Both, config.depth, config.max_types);

        // 5. Get import context (from import graph)
        let imports = self.build_import_context(symbol, &definition);

        // 6. Find related tests
        let tests = self.find_related_tests(symbol, &definition).await?;

        // 7. Apply line budget and truncate if needed
        self.apply_budget(/* ... */)
    }

    fn enrich_caller(&self, edge: &CallEdge) -> Caller {
        // Read caller function signature from file
        // Extract snippet around call site
    }

    fn enrich_callee(&self, edge: &CallEdge) -> Callee {
        // Resolve callee definition if local
        // Get signature if available
    }

    async fn find_related_tests(&self, symbol: &str, def: &Definition) -> Vec<TestFile> {
        // 1. Look for *_test.rs, *.spec.ts, *Test.swift
        // 2. Grep for symbol name in test files
        // 3. Rank by relevance (symbol mention count, file proximity)
    }
}
```

**Acceptance Criteria**:
- [ ] Assembles complete context in < 500ms for typical symbols
- [ ] Respects `max_lines` budget (truncates intelligently)
- [ ] Falls back gracefully when graphs are incomplete
- [ ] Enriches callers/callees with signatures
- [ ] Finds related tests with relevance ranking

---

### Task 2.4: Budget-Aware Truncation

**Owner**: _unassigned_
**Effort**: 1 day
**Priority**: P1
**Depends on**: Task 2.3

**Description**: Implement intelligent truncation to fit within token/line budgets.

**Files to Modify**:
- `crates/swe-grep-core/src/context/engine.rs`
- `crates/swe-grep-core/src/context/budget.rs` (NEW)

**Algorithm**:
```rust
pub struct BudgetConfig {
    pub max_lines: usize,
    pub definition_weight: f32,    // 0.4 - definition gets 40% of budget
    pub callers_weight: f32,       // 0.25
    pub callees_weight: f32,       // 0.15
    pub types_weight: f32,         // 0.1
    pub imports_weight: f32,       // 0.05
    pub tests_weight: f32,         // 0.05
}

impl BudgetAllocator {
    pub fn allocate(&self, response: &mut ContextResponse, config: &BudgetConfig) {
        let total = config.max_lines;

        // Allocate lines per section
        let def_budget = (total as f32 * config.definition_weight) as usize;
        let callers_budget = (total as f32 * config.callers_weight) as usize;
        // ...

        // Truncate definition body if over budget
        if let Some(ref mut def) = response.definition {
            def.body = self.truncate_body(&def.body, def_budget);
        }

        // Limit callers to budget
        let lines_per_caller = callers_budget / response.callers.len().max(1);
        response.callers.truncate(callers_budget / 5); // ~5 lines per caller

        // Mark truncated if we cut anything
        response.truncated = /* ... */;
    }
}
```

**Acceptance Criteria**:
- [ ] Never exceeds `max_lines` budget
- [ ] Prioritizes definition and callers over other sections
- [ ] Sets `truncated: true` when content is cut
- [ ] Truncates bodies, not just counts

---

### Task 2.5: Test Discovery

**Owner**: _unassigned_
**Effort**: 1-2 days
**Priority**: P1
**Depends on**: Task 2.3

**Description**: Auto-discover test files related to a symbol.

**Files to Create**:
- `crates/swe-grep-core/src/context/tests.rs` (NEW)

**Algorithm**:
```rust
pub struct TestDiscovery;

impl TestDiscovery {
    pub async fn find_tests(
        symbol: &str,
        definition_file: &str,
        path: &Path,
    ) -> Vec<TestFile> {
        // 1. Find test file patterns
        let test_patterns = [
            // Rust
            "*_test.rs", "**/tests/**/*.rs",
            // TypeScript
            "*.spec.ts", "*.test.ts", "**/__tests__/**/*.ts",
            // Swift
            "*Tests.swift", "**/*Tests/**/*.swift",
        ];

        // 2. Find candidate files
        let candidates = glob_files(path, &test_patterns);

        // 3. Grep for symbol in candidates
        let mentions = grep_symbol_in_files(symbol, &candidates);

        // 4. Rank by relevance
        //    - Same directory as definition: +0.3
        //    - Symbol appears in function name: +0.2
        //    - Symbol appears multiple times: +0.1 per mention

        // 5. Extract test function names from top files
        let test_functions = extract_test_functions(&top_files);

        mentions.into_iter()
            .map(|(file, count)| TestFile {
                file,
                test_functions: test_functions.get(&file),
                relevance: calculate_relevance(/* ... */),
            })
            .collect()
    }
}
```

**Acceptance Criteria**:
- [ ] Finds `*_test.rs` for Rust symbols
- [ ] Finds `*.spec.ts` for TypeScript symbols
- [ ] Finds `*Tests.swift` for Swift symbols
- [ ] Ranks by relevance (proximity, mention count)
- [ ] Extracts test function names

---

## Epic 3: Language Expansion

**Why**: Go and Python are heavily used. Limited coverage blocks adoption.

### Task 3.1: Go Fact Extraction

**Owner**: _unassigned_
**Effort**: 2-3 days
**Priority**: P1

**Description**: Add tree-sitter based fact extraction for Go.

**Files to Create/Modify**:
- `crates/swe-grep-core/src/analysis/go.rs` (NEW)
- `crates/swe-grep-core/src/analysis/extract.rs` (add Go)
- `Cargo.toml` (add `tree-sitter-go`)

**Facts to Extract**:
- Function definitions (func, method receivers)
- Type definitions (struct, interface)
- Imports (import statements)
- Calls (function calls, method invocations)

**Tree-sitter Queries**:
```scheme
;; Function definition
(function_declaration
  name: (identifier) @name
  parameters: (parameter_list) @params
  result: (_)? @return) @func

;; Method definition
(method_declaration
  receiver: (parameter_list) @receiver
  name: (field_identifier) @name) @method

;; Type definition
(type_declaration
  (type_spec
    name: (type_identifier) @name
    type: (_) @type)) @typedef

;; Import
(import_declaration
  (import_spec
    path: (interpreted_string_literal) @path)) @import
```

**Acceptance Criteria**:
- [ ] Extracts function/method definitions with signatures
- [ ] Extracts struct/interface definitions
- [ ] Extracts import statements
- [ ] Extracts function calls
- [ ] Unit tests with Go code samples

---

### Task 3.2: Python Fact Extraction

**Owner**: _unassigned_
**Effort**: 2-3 days
**Priority**: P1

**Description**: Add tree-sitter based fact extraction for Python.

**Files to Create/Modify**:
- `crates/swe-grep-core/src/analysis/python.rs` (NEW)
- `crates/swe-grep-core/src/analysis/extract.rs` (add Python)
- `Cargo.toml` (add `tree-sitter-python`)

**Facts to Extract**:
- Function/method definitions (def, async def)
- Class definitions
- Imports (import, from...import)
- Calls (function calls, method calls)
- Decorators (@decorator)

**Acceptance Criteria**:
- [ ] Extracts function definitions with parameters
- [ ] Extracts class definitions with bases
- [ ] Extracts import statements (all forms)
- [ ] Extracts function calls
- [ ] Handles decorators
- [ ] Unit tests with Python code samples

---

### Task 3.3: Type Graph Language Expansion

**Owner**: _unassigned_
**Effort**: 1-2 days
**Priority**: P1
**Depends on**: Tasks 3.1, 3.2

**Description**: Update type graph to support Go and Python.

**Files to Modify**:
- `crates/swe-grep-core/src/analysis/types.rs`

**Go Specifics**:
- Interface implementation (implicit, by method set)
- Struct embedding

**Python Specifics**:
- Class inheritance (single and multiple)
- Protocol implementation (typing.Protocol)

**Acceptance Criteria**:
- [ ] Go interface implementation detected
- [ ] Go struct embedding detected
- [ ] Python class inheritance detected
- [ ] Python Protocol patterns detected

---

## Epic 4: Performance & Reliability

### Task 4.1: Parallel Graph Construction

**Owner**: _unassigned_
**Effort**: 2 days
**Priority**: P2

**Description**: Build call/type/import graphs in parallel for large repos.

**Files to Modify**:
- `crates/swe-grep-core/src/analysis/extract.rs`
- `crates/swe-grep-core/src/main.rs` (refs handler)

**Approach**:
```rust
// Use rayon for parallel file processing
let facts: Vec<FileFacts> = files
    .par_iter()
    .map(|f| extract_facts(f))
    .collect();

// Build graphs concurrently
let (call_graph, type_graph, import_graph) = rayon::join(
    || CallGraph::from_facts(&facts),
    || rayon::join(
        || TypeGraph::from_facts(&facts),
        || ImportGraph::from_facts(&facts),
    ),
);
```

**Acceptance Criteria**:
- [ ] Graph construction scales linearly with cores
- [ ] < 2s for 10K files on 8-core machine
- [ ] No race conditions or data corruption

---

### Task 4.2: Incremental Graph Updates

**Owner**: _unassigned_
**Effort**: 3-4 days
**Priority**: P2

**Description**: Cache graphs and update incrementally based on file changes.

**Files to Create**:
- `crates/swe-grep-core/src/analysis/cache.rs` (NEW)

**Approach**:
```rust
pub struct GraphCache {
    call_graph: CallGraph,
    type_graph: TypeGraph,
    import_graph: ImportGraph,
    file_hashes: HashMap<String, u64>,  // path -> content hash
    last_updated: SystemTime,
}

impl GraphCache {
    pub fn load_or_build(path: &Path) -> Self;

    pub fn update(&mut self, changed_files: &[PathBuf]) {
        // 1. Re-extract facts for changed files
        // 2. Remove old edges involving changed files
        // 3. Insert new edges
        // 4. Update hashes
    }

    pub fn save(&self, path: &Path);
}
```

**Cache Location**: `$CACHE_DIR/.swe-grep-cache/graphs/`

**Acceptance Criteria**:
- [ ] Persists graphs to disk (bincode/messagepack)
- [ ] Detects changed files via content hash
- [ ] Incremental update < 100ms for single file change
- [ ] Falls back to full rebuild if cache corrupted

---

### Task 4.3: Graceful Degradation

**Owner**: _unassigned_
**Effort**: 1 day
**Priority**: P2

**Description**: Ensure context command works even when graphs are incomplete.

**Files to Modify**:
- `crates/swe-grep-core/src/context/engine.rs`

**Scenarios**:
1. Call graph fails → Return callers as empty, warn
2. Type graph fails → Return types as empty, warn
3. Import graph fails → Return imports as empty, warn
4. Definition not found → Still return callers/callees if available

**Acceptance Criteria**:
- [ ] Never fails entirely due to single graph failure
- [ ] Warnings indicate which data is missing
- [ ] Partial results are clearly marked

---

## Epic 5: Agent Experience Polish

### Task 5.1: Concise Context Mode

**Owner**: _unassigned_
**Effort**: 0.5 days
**Priority**: P1
**Depends on**: Task 2.3

**Description**: Add `--concise` flag for minimal token output.

**Concise Output**:
```json
{
  "symbol": "AuthService",
  "def": "pub struct AuthService { ... }",
  "callers": ["handle_login:42", "verify_token:89"],
  "callees": ["hash_password", "validate_email"],
  "tests": ["auth_service_test.rs"]
}
```

**Acceptance Criteria**:
- [ ] Omits full bodies, signatures, docstrings
- [ ] Uses compact `file:line` format for references
- [ ] < 50 lines output for typical symbol

---

### Task 5.2: Streaming Context

**Owner**: _unassigned_
**Effort**: 1 day
**Priority**: P2
**Depends on**: Task 2.3

**Description**: Stream context sections as they're computed (NDJSON).

**Output Format**:
```ndjson
{"type":"definition","data":{...}}
{"type":"callers","data":[...]}
{"type":"callees","data":[...]}
{"type":"types","data":{...}}
{"type":"imports","data":{...}}
{"type":"tests","data":[...]}
{"type":"summary","data":{...}}
```

**Acceptance Criteria**:
- [ ] `--stream` flag enables NDJSON output
- [ ] Each section emitted as soon as computed
- [ ] Summary always emitted last

---

### Task 5.3: Context Presets

**Owner**: _unassigned_
**Effort**: 0.5 days
**Priority**: P2

**Description**: Add presets for common agent use cases.

**Presets**:
```bash
# Refactoring: Need callers + callees + types
swe-grep context --preset refactor --symbol Foo --path .

# Debugging: Need callers + definition body + tests
swe-grep context --preset debug --symbol Foo --path .

# Understanding: Need everything, summarized
swe-grep context --preset understand --symbol Foo --path .
```

**Preset Configurations**:
```rust
match preset {
    "refactor" => ContextConfig {
        include_callers: true,
        include_callees: true,
        include_types: true,
        include_imports: false,
        include_tests: false,
        max_lines: 300,
    },
    "debug" => ContextConfig {
        include_callers: true,
        include_callees: false,
        include_types: false,
        include_imports: false,
        include_tests: true,
        max_lines: 200,
    },
    // ...
}
```

**Acceptance Criteria**:
- [ ] `--preset refactor|debug|understand` works
- [ ] Presets documented in help text
- [ ] Can override preset options with explicit flags

---

## Implementation Order

```
Phase 1: Foundation (Week 1-2)
├── Task 1.1: Import Fact Extraction ──────────┐
├── Task 1.2: Import Graph Construction ───────┤
├── Task 1.3: Import Graph CLI ────────────────┘
├── Task 2.1: Context Command Scaffold
└── Task 2.2: Context Response Schema

Phase 2: Core Feature (Week 3-4)
├── Task 2.3: Context Assembly Engine ─────────┐
├── Task 2.4: Budget-Aware Truncation ─────────┤
├── Task 2.5: Test Discovery ──────────────────┘
└── Task 5.1: Concise Context Mode

Phase 3: Language Expansion (Week 5-6)
├── Task 3.1: Go Fact Extraction
├── Task 3.2: Python Fact Extraction
└── Task 3.3: Type Graph Expansion

Phase 4: Polish (Week 7-8)
├── Task 4.1: Parallel Graph Construction
├── Task 4.2: Incremental Graph Updates
├── Task 4.3: Graceful Degradation
├── Task 5.2: Streaming Context
└── Task 5.3: Context Presets
```

---

## Success Metrics

| Metric | Target |
|--------|--------|
| Context assembly latency | < 500ms (warm), < 2s (cold) |
| Token efficiency | 50% reduction vs manual search chain |
| Language coverage | 6 languages (Rust, Swift, TS, JS, Go, Python) |
| Graph build time | < 2s for 10K files |
| Agent adoption | Used in 3+ internal agents |

---

## Dependencies

**External Crates to Add**:
```toml
tree-sitter-go = "0.20"
tree-sitter-python = "0.20"
rayon = "1.8"          # Parallel iteration
bincode = "1.3"        # Graph serialization
```

---

## Notes for Builders

1. **Start with existing infrastructure**: The call graph and type graph already exist. Study `analysis/callgraph.rs` and `analysis/types.rs` before starting.

2. **Use the agent output schema**: All new commands should use the `AgentOutput` pattern from `agent.rs`.

3. **Test with real repos**: Use the fixture repos in `fixtures/` but also test on real codebases.

4. **Performance matters**: Agents are latency-sensitive. Profile with `cargo flamegraph` if latency exceeds targets.

5. **Fail gracefully**: Incomplete data is better than an error. Always return partial results with warnings.
