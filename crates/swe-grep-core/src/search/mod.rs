mod concept_expansion;
mod config;
mod engine;
mod formatting;
mod fuzzy;
mod hits;
mod query;
mod state;
mod stats;

pub(crate) use config::passes_extension_filter;
pub use formatting::ContextHint;
pub use stats::{
    ConceptExpansionSummary, GitInfo, SearchSummary, StageStats, StartupStats, TopHit,
};

use anyhow::Result;

use crate::cli::SearchArgs;

/// Execute a single SWE-grep cycle using the phase-3 workflow.
pub async fn execute(args: SearchArgs) -> Result<SearchSummary> {
    let config = config::SearchConfig::try_from_args(args)?;
    let mut engine = engine::SearchEngine::new(config)?;
    engine.run_cycle().await
}
