use serde::{Deserialize, Serialize};

pub const FACTS_SCHEMA_VERSION: u32 = 4;

use super::imports::{ExportFact, ImportFact};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileFacts {
    pub version: u32,
    pub file: String,
    pub mtime: u64,
    pub size: u64,
    pub language: String,
    pub facts: FactBundle,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FactBundle {
    #[serde(default)]
    pub definitions: Vec<DefinitionFact>,
    #[serde(default)]
    pub references: Vec<ReferenceFact>,
    #[serde(default)]
    pub calls: Vec<CallFact>,
    #[serde(default)]
    pub relations: Vec<RelationFact>,
    #[serde(default)]
    pub fields: Vec<FieldFact>,
    #[serde(default)]
    pub methods: Vec<MethodFact>,
    #[serde(default)]
    pub imports: Vec<ImportFact>,
    #[serde(default)]
    pub exports: Vec<ExportFact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefinitionFact {
    pub symbol: String,
    pub kind: String,
    pub line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<(usize, usize)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceFact {
    pub symbol: String,
    pub line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallFact {
    pub caller: String,
    pub callee: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationFact {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldFact {
    pub owner: String,
    pub name: String,
    pub line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<(usize, usize)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_repr: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodFact {
    pub owner: String,
    pub name: String,
    pub line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<(usize, usize)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}
