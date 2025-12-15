use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportFact {
    pub file: String,
    pub line: usize,
    pub kind: ImportKind,
    pub source: String,
    pub symbols: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImportKind {
    Named,
    Default,
    Namespace,
    SideEffect,
    Reexport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportFact {
    pub file: String,
    pub line: usize,
    pub symbol: String,
    pub kind: ExportKind,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExportKind {
    Function,
    Class,
    Type,
    Interface,
    Const,
    Enum,
    Reexport,
}
