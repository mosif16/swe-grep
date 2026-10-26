use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ContextResponse {
    pub symbol: String,
    pub resolution: SymbolResolution,
    pub analysis: std::collections::BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    pub definition: Option<Definition>,
    pub callers: Vec<Caller>,
    pub callees: Vec<Callee>,
    pub types: TypeContext,
    pub imports: ImportContext,
    pub tests: Vec<TestFile>,
    pub summary: ContextSummary,
    pub truncated: bool,
}

impl ContextResponse {
    pub fn empty(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            resolution: SymbolResolution::default(),
            analysis: std::collections::BTreeMap::new(),
            notes: Vec::new(),
            definition: None,
            callers: Vec::new(),
            callees: Vec::new(),
            types: TypeContext::default(),
            imports: ImportContext::default(),
            tests: Vec::new(),
            summary: ContextSummary::default(),
            truncated: false,
        }
    }
}

/// Definition identity is separate from the scope in which relationships are searched.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SymbolResolution {
    pub status: String,
    pub candidates: Vec<SymbolCandidate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolCandidate {
    pub symbol: String,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Definition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implements: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity: Option<SymbolComplexity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolComplexity {
    pub fields: usize,
    pub methods: usize,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Caller {
    pub symbol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Callee {
    pub symbol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TypeContext {
    pub implements: Vec<String>,
    pub extended_by: Vec<String>,
    pub contains: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportContext {
    pub exported_from: Vec<String>,
    pub imported_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestFile {
    pub path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub test_names: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ContextSummary {
    pub callers: usize,
    pub callees: usize,
    pub tests: usize,
}
