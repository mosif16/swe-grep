use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Top-level CLI definition for swe-grep.
#[derive(Parser, Debug)]
#[command(name = "swe-grep")]
#[command(about = "Rust-native search agent for blazing-fast code retrieval", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Search the repository for occurrences of a symbol.
    Search(SearchArgs),
}

/// Arguments for the `search` subcommand.
#[derive(clap::Args, Debug)]
pub struct SearchArgs {
    /// Symbol or identifier to search for.
    #[arg(long)]
    pub symbol: String,

    /// Root directory of the repository; defaults to the current working directory.
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Optional explicit language hint for AST-Grep (e.g. rust, tsx, swift).
    #[arg(long)]
    pub language: Option<String>,

    /// Timeout applied per tool invocation (seconds).
    #[arg(long, default_value_t = 3)]
    pub timeout_secs: u64,

    /// Maximum number of ripgrep matches to collect per query rewrite.
    #[arg(long, default_value_t = 20)]
    pub max_matches: usize,

    /// Maximum number of concurrent tool invocations (defaults to 8 workers).
    #[arg(long, default_value_t = 8)]
    pub concurrency: usize,

    /// Enable Tantivy-backed micro-indexing for the current repository.
    #[arg(long, default_value_t = false)]
    pub enable_index: bool,

    /// Override the default path for the Tantivy index directory.
    #[arg(long)]
    pub index_dir: Option<PathBuf>,

    /// Enable the ripgrep-all fallback for documentation and config files.
    #[arg(long, default_value_t = false)]
    pub enable_rga: bool,

    /// Directory used to persist symbol hints and directory cache data.
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,
}
