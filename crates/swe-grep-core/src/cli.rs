use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand};

/// Top-level CLI definition for swe-grep.
#[derive(Parser, Debug)]
#[command(name = "swe-grep")]
#[command(about = "Rust-native search agent for blazing-fast code retrieval", long_about = None)]
pub struct Cli {
    /// Disable telemetry exporters for this invocation.
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub disable_telemetry: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Search the repository for occurrences of a symbol.
    Search(SearchArgs),
    /// Run benchmark scenarios and collect performance metrics.
    Bench(BenchArgs),
    /// Serve the SWE-Grep API over HTTP and gRPC.
    Serve(ServeArgs),
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

    /// Optional explicit language hint for AST-Grep (e.g. rust, tsx, swift, auto-swift-ts).
    #[arg(long, value_name = "LANGUAGE")]
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

    /// Number of neighbouring lines to include before each match when expanding snippets.
    #[arg(long = "context-before", default_value_t = 0)]
    pub context_before: usize,

    /// Number of neighbouring lines to include after each match when expanding snippets.
    #[arg(long = "context-after", default_value_t = 0)]
    pub context_after: usize,

    /// Retrieve full file bodies for each surfaced hit.
    #[arg(long = "body", action = ArgAction::SetTrue, default_value_t = false)]
    pub body: bool,

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

    /// Directory to append structured search logs (JSON Lines).
    #[arg(long)]
    pub log_dir: Option<PathBuf>,

    /// Disable fd-based discovery for this search.
    #[arg(long = "disable-fd", action = ArgAction::SetFalse, default_value_t = true)]
    pub use_fd: bool,

    /// Disable AST-Grep disambiguation for this search.
    #[arg(long = "disable-ast-grep", action = ArgAction::SetFalse, default_value_t = true)]
    pub use_ast_grep: bool,
}

/// Arguments for the `bench` subcommand.
#[derive(clap::Args, Debug)]
pub struct BenchArgs {
    /// Path to a benchmark scenario file (JSON). Defaults to benchmarks/default.json.
    #[arg(long)]
    pub scenario: Option<PathBuf>,

    /// Optional file to append the benchmark summary as JSON.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Number of iterations per scenario (averaged in the summary).
    #[arg(long, default_value_t = 1)]
    pub iterations: usize,

    /// Enable Tantivy-backed indexing during benchmarks.
    #[arg(long, default_value_t = false)]
    pub enable_index: bool,

    /// Enable ripgrep-all fallback during benchmarks.
    #[arg(long, default_value_t = false)]
    pub enable_rga: bool,

    /// Directory used for caching during benchmarks.
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,

    /// Directory to write per-run cycle logs during benchmarks.
    #[arg(long)]
    pub log_dir: Option<PathBuf>,
}

/// Arguments for the `serve` subcommand.
#[derive(clap::Args, Debug)]
pub struct ServeArgs {
    /// Address to bind the HTTP API server.
    #[arg(long, default_value = "127.0.0.1:8080")]
    pub http_addr: SocketAddr,

    /// Address to bind the gRPC server.
    #[arg(long, default_value = "127.0.0.1:50051")]
    pub grpc_addr: SocketAddr,

    /// Root directory of the repository to index; defaults to the current working directory.
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Timeout applied per tool invocation (seconds).
    #[arg(long, default_value_t = 3)]
    pub timeout_secs: u64,

    /// Maximum number of ripgrep matches to collect per query rewrite.
    #[arg(long, default_value_t = 20)]
    pub max_matches: usize,

    /// Maximum number of concurrent tool invocations (defaults to 8 workers).
    #[arg(long, default_value_t = 8)]
    pub concurrency: usize,

    /// Enable Tantivy-backed micro-indexing by default.
    #[arg(long, default_value_t = false)]
    pub enable_index: bool,

    /// Enable the ripgrep-all fallback by default.
    #[arg(long, default_value_t = false)]
    pub enable_rga: bool,

    /// Override the default path for the Tantivy index directory.
    #[arg(long)]
    pub index_dir: Option<PathBuf>,

    /// Directory used to persist symbol hints and directory cache data.
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,

    /// Directory to append structured search logs (JSON Lines).
    #[arg(long)]
    pub log_dir: Option<PathBuf>,

    /// Disable fd-based discovery by default.
    #[arg(long = "disable-fd", action = ArgAction::SetFalse, default_value_t = true)]
    pub use_fd: bool,

    /// Disable AST-Grep disambiguation by default.
    #[arg(long = "disable-ast-grep", action = ArgAction::SetFalse, default_value_t = true)]
    pub use_ast_grep: bool,
}
