mod concept_expansion;
mod config;
mod declarations;
mod engine;
mod formatting;
mod fuzzy;
mod hits;
mod query;
mod scope;
pub use scope::SearchScope;
mod state;
mod stats;

pub(crate) use config::passes_extension_filter;
pub use formatting::ContextHint;
pub(crate) use hits::{
    HitSemanticKind, SemanticClassification, UsageContextKind, classify_semantic,
};
pub use stats::{
    ConceptExpansionSummary, FuzzyMatch, GitInfo, SearchSummary, StageStats, StartupStats, TopHit,
};

use anyhow::Result;

use crate::cli::SearchArgs;

pub fn default_cache_dir_for_root(root: &std::path::Path) -> std::path::PathBuf {
    config::default_cache_dir_for_root(root)
}

/// Execute a single SWE-grep cycle using the phase-3 workflow.
pub async fn execute(args: SearchArgs) -> Result<SearchSummary> {
    execute_with_scope(args).await.map(|(summary, _)| summary)
}

/// Execute text search and return the identical normalized scope for semantic enrichment.
/// Sharing the scope avoids duplicate Git queries and repeated per-hit path resolution.
pub async fn execute_with_scope(args: SearchArgs) -> Result<(SearchSummary, SearchScope)> {
    let config = config::SearchConfig::try_from_args(args)?;
    let mut engine = engine::SearchEngine::new(config)?;
    let summary = engine.run_cycle().await?;
    Ok((summary, engine.scope.clone()))
}
