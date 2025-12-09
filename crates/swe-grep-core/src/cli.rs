use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand, ValueEnum};

/// Top-level CLI definition for swe-grep.
#[derive(Parser, Debug)]
#[command(name = "swe-grep")]
#[command(about = "Rust-native search agent for blazing-fast code retrieval", long_about = None)]
pub struct Cli {
    /// Disable telemetry exporters for this invocation.
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub disable_telemetry: bool,

    /// Clear swe-grep caches (state + index) before running the command.
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub clear_cache: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Search the repository for occurrences of a symbol.
    Search(SearchCommand),
    /// Visualize dependencies/usages for a symbol.
    Usages(UsageCommand),
    /// Generate a removal/refactor plan for a symbol.
    Remove(RemoveCommand),
    /// Run benchmark scenarios and collect performance metrics.
    Bench(BenchArgs),
    /// Serve the SWE-Grep API over HTTP and gRPC.
    Serve(ServeArgs),
}

/// Extended CLI surface for search, including agent-focused flags.
#[derive(clap::Args, Debug)]
pub struct SearchCommand {
    #[command(flatten)]
    pub search: SearchArgs,

    /// Enable agent mode (schema v1) with concise, structured output and sensible defaults.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub agent: bool,

    /// Output format when agent mode is enabled (json or table).
    #[arg(long, default_value = "json", value_enum)]
    pub format: OutputFormat,

    /// Minimize agent output (omit context, origin, scores, and bodies).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub concise: bool,

    /// Pretty-print agent JSON output (ignored for streaming).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub json_pretty: bool,

    /// Comma-separated list of symbols to search (batch mode).
    #[arg(long)]
    pub symbols: Option<String>,

    /// Read symbols from a file (one per line) for batch mode.
    #[arg(long)]
    pub symbols_file: Option<PathBuf>,

    /// Read symbols from stdin (one per line) for batch mode.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub stdin_symbols: bool,

    /// Stream NDJSON results per symbol (batch-friendly).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub stream: bool,

    /// Emit per-symbol progress updates on stderr.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub progress: bool,

    /// Silence logs on stderr (errors still surface).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub quiet: bool,

    /// Trim fetched bodies to at most N lines (agent mode only).
    #[arg(long)]
    pub trim_body: Option<usize>,

    /// Attempt to extract a nearby definition/signature for each hit.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub definitions: bool,
}

/// Output format for agent mode.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    Json,
    Table,
    Compact,
    Tree,
    Edit,
}

/// Shared tool toggles for search execution.
#[derive(clap::Args, Debug, Clone)]
pub struct ToolArgs {
    /// Enable Tantivy-backed micro-indexing for the current repository.
    #[arg(long, default_value_t = false)]
    pub enable_index: bool,

    /// Enable the ripgrep-all fallback for documentation and config files.
    #[arg(long, default_value_t = false)]
    pub enable_rga: bool,

    /// Disable fd-based discovery for this search.
    #[arg(long = "disable-fd", action = ArgAction::SetFalse, default_value_t = true)]
    pub use_fd: bool,

    /// Disable AST-Grep disambiguation for this search.
    #[arg(long = "disable-ast-grep", action = ArgAction::SetFalse, default_value_t = true)]
    pub use_ast_grep: bool,
}

/// Arguments for the `search` subcommand.
#[derive(clap::Args, Debug, Clone)]
pub struct SearchArgs {
    /// Symbol or identifier to search for.
    #[arg(long, default_value_t = String::new())]
    pub symbol: String,

    /// Conceptual/semantic search phrase (e.g. "text to speech"); substitutes for --symbol when provided.
    #[arg(long)]
    pub concept: Option<String>,

    /// Treat --symbol as a raw regular expression (disables automatic escaping).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub regex: bool,

    /// Allow shell-style wildcards (*, ?) inside --symbol (converted to regex).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub wildcard: bool,

    /// Include localization/string resources (xcstrings/strings/arb/json) in the search surface.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub include_strings: bool,

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

    #[command(flatten)]
    pub tools: ToolArgs,

    /// Number of neighbouring lines to include before each match when expanding snippets.
    #[arg(long = "context-before", default_value_t = 0)]
    pub context_before: usize,

    /// Number of neighbouring lines to include after each match when expanding snippets.
    #[arg(long = "context-after", default_value_t = 0)]
    pub context_after: usize,

    /// Retrieve full file bodies for each surfaced hit.
    #[arg(long = "body", action = ArgAction::SetTrue, default_value_t = false)]
    pub body: bool,

    /// Override the default path for the Tantivy index directory.
    #[arg(long)]
    pub index_dir: Option<PathBuf>,

    /// Directory used to persist symbol hints and directory cache data.
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,

    /// Directory to append structured search logs (JSON Lines).
    #[arg(long)]
    pub log_dir: Option<PathBuf>,
}

/// Arguments for the `usages` subcommand.
#[derive(clap::Args, Debug)]
pub struct UsageCommand {
    #[command(flatten)]
    pub search: SearchArgs,

    /// Output style for the usage graph.
    #[arg(long, value_enum, default_value = "tree")]
    pub format: UsageFormat,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum UsageFormat {
    Tree,
    Json,
    Compact,
}

/// Arguments for the `remove` subcommand.
#[derive(clap::Args, Debug)]
pub struct RemoveCommand {
    #[command(flatten)]
    pub search: SearchArgs,

    /// Emit a plan without touching files.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub dry_run: bool,

    /// Apply deletions for obvious symbol-owned files (best-effort, cautious).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub apply: bool,
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

    #[command(flatten)]
    pub tools: ToolArgs,

    /// Override the default path for the Tantivy index directory.
    #[arg(long)]
    pub index_dir: Option<PathBuf>,

    /// Directory used to persist symbol hints and directory cache data.
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,

    /// Directory to append structured search logs (JSON Lines).
    #[arg(long)]
    pub log_dir: Option<PathBuf>,
}
