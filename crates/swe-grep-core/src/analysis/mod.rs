mod cache;
mod extract;
mod facts;
mod rust;
mod swift;
mod types;
mod typescript;

pub use cache::{AnalysisCache, CacheStats};
pub use extract::{extract_repo_facts, ExtractFactsOptions};
pub use facts::{CallFact, DefinitionFact, FactBundle, FileFacts, ReferenceFact, RelationFact};
mod callgraph;
pub use callgraph::{CallEdge, CallGraph, CallGraphView, CallTreeNode, normalize_symbol};
pub use types::{TypeDirection, TypeGraph, TypeGraphView, TypeRelationEdge, TypeTreeNode};
