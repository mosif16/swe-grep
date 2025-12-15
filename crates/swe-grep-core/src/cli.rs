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
    /// Explore callers/callees for a symbol.
    Refs(RefsCommand),
    /// Assemble a budgeted context bundle for a symbol (definition, callers/callees, types/imports, tests).
    Context(ContextCommand),
    /// Generate a removal/refactor plan for a symbol.
    Remove(RemoveCommand),
    /// Run benchmark scenarios and collect performance metrics.
    Bench(BenchArgs),
    /// Serve the SWE-Grep API over HTTP and gRPC.
    Serve(ServeArgs),
    /// Collect structured feedback about agent tools (saved under ./Feedback by default).
    Feedback(FeedbackCommand),
}

/// Extended CLI surface for search, including agent-focused flags.
#[derive(clap::Args, Debug)]
pub struct SearchCommand {
    #[command(flatten)]
    pub search: SearchArgs,

    /// Shorthand for clean, pipe-friendly JSON output (equivalent to --agent --format json --quiet).
    #[arg(long = "json-only", default_value_t = false, action = ArgAction::SetTrue)]
    pub json_only: bool,

    /// Enable agent mode (schema v1) with concise, structured output and sensible defaults.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub agent: bool,

    /// Output format when agent mode is enabled (json/table/compact/tree/edit/minimal/summary).
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

    /// Emit editor-friendly open commands for each hit (code -g file:line, vim +line file, etc.).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub open: bool,

    /// Override the editor command template (use {path} and {line} tokens).
    #[arg(long = "open-command")]
    pub open_command: Option<String>,

    /// Trim fetched bodies to at most N lines (agent mode only).
    #[arg(long)]
    pub trim_body: Option<usize>,

    /// Number of lines to include from the start of each body (agent mode only).
    #[arg(long = "body-head")]
    pub body_head: Option<usize>,

    /// Number of lines to include from the end of each body (agent mode only).
    #[arg(long = "body-tail")]
    pub body_tail: Option<usize>,

    /// Disable trimming for fetched bodies (agent mode only).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub full_body: bool,

    /// Attempt to extract a nearby definition/signature for each hit.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub definitions: bool,

    /// Assert that the symbol does not exist; exits with status 1 if found.
    #[arg(long = "assert-absent", default_value_t = false, action = ArgAction::SetTrue)]
    pub assert_absent: bool,
}

/// Output format for agent mode.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    Json,
    Table,
    Compact,
    Tree,
    Edit,
    Minimal,
    Summary,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum SymbolKind {
    Struct,
    Class,
    Func,
    Enum,
    Trait,
    Protocol,
    Interface,
    Const,
    Actor,
}

/// High-level search strategy override.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum SearchMode {
    /// Treat the symbol as a literal identifier (default).
    Literal,
    /// Treat the symbol as a raw regular expression.
    Regex,
    /// Use AST-Grep only (no ripgrep/index/rga fallbacks).
    Ast,
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

    /// Search for literal/raw text (no query expansion; disables AST disambiguation).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub text: bool,

    /// Conceptual/semantic search phrase (e.g. "text to speech"); substitutes for --symbol when provided.
    #[arg(long)]
    pub concept: Option<String>,

    /// Emit the expanded concept terms that were searched.
    #[arg(long = "concept-expand", default_value_t = false, action = ArgAction::SetTrue)]
    pub concept_expand: bool,

    /// Treat --symbol as a raw regular expression (disables automatic escaping).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub regex: bool,

    /// Keep the provided regex exactly as-is (no query expansion / case variants).
    #[arg(long = "literal-regex", default_value_t = false, action = ArgAction::SetTrue)]
    pub literal_regex: bool,

    /// Allow shell-style wildcards (*, ?) inside --symbol (converted to regex).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub wildcard: bool,

    /// Search for decorator/attribute usages like @Model or @Component.
    #[arg(long, alias = "decorator", default_value_t = false, action = ArgAction::SetTrue)]
    pub attribute: bool,

    /// Search literal/string content (adds quoted variants; disables AST disambiguation).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub string: bool,

    /// Emit AST-Grep diagnostics and parse warnings instead of silently falling back to regex.
    #[arg(long = "verbose-ast", default_value_t = false, action = ArgAction::SetTrue)]
    pub verbose_ast: bool,

    /// Include localization/string resources (xcstrings/strings/arb/json) in the search surface.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub include_strings: bool,

    /// Exclude non-code files (markdown, txt, json configs) from results.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub code_only: bool,

    /// Perform case-insensitive matching.
    #[arg(short = 'i', long = "ignore-case", default_value_t = false, action = ArgAction::SetTrue)]
    pub ignore_case: bool,

    /// Only show definition sites (struct, fn, class, etc.), excluding usages.
    #[arg(long = "definitions-only", default_value_t = false, action = ArgAction::SetTrue)]
    pub definitions_only: bool,

    /// Only show usage sites, excluding definitions.
    #[arg(long = "usages-only", default_value_t = false, action = ArgAction::SetTrue)]
    pub usages_only: bool,

    /// Root directory of the repository; defaults to the current working directory.
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Restrict search to a single file (relative to --path when not absolute).
    #[arg(long = "scope-file")]
    pub scope_file: Option<PathBuf>,

    /// Glob patterns to limit the search scope (e.g. \"src/**/*.tsx\"); can be comma-separated.
    #[arg(long = "path-glob", value_delimiter = ',')]
    pub path_glob: Vec<String>,

    /// Glob patterns to exclude from results (e.g., \"*.md,PLAN*\").
    #[arg(long = "exclude", value_delimiter = ',')]
    pub exclude_patterns: Vec<String>,

    /// Exclude common test files and directories.
    #[arg(long = "exclude-tests", default_value_t = false, action = ArgAction::SetTrue)]
    pub exclude_tests: bool,

    /// Optional explicit language hint for AST-Grep (e.g. rust, tsx, swift, auto-swift-ts).
    #[arg(long, value_name = "LANGUAGE")]
    pub language: Option<String>,

    /// Explicitly control search strategy (literal|regex|ast).
    #[arg(long, value_enum)]
    pub mode: Option<SearchMode>,

    /// Filter results by declaration type.
    #[arg(long, value_enum)]
    pub kind: Option<SymbolKind>,

    /// Timeout applied per tool invocation (seconds).
    #[arg(long, default_value_t = 3)]
    pub timeout_secs: u64,

    /// Maximum number of ripgrep matches to collect per query rewrite.
    #[arg(long, default_value_t = 20)]
    pub max_matches: usize,

    /// Maximum number of hits to include in the output.
    #[arg(long = "top-hits", default_value_t = 20)]
    pub top_hits: usize,

    /// Maximum number of columns to capture per match line (0 = unlimited).
    #[arg(long = "max-columns", default_value_t = 200)]
    pub max_columns: usize,

    /// Maximum number of concurrent tool invocations (defaults to 8 workers).
    #[arg(long, default_value_t = 8)]
    pub concurrency: usize,

    #[command(flatten)]
    pub tools: ToolArgs,

    /// Shorthand to set both --context-before and --context-after.
    #[arg(long, value_name = "N", conflicts_with_all = ["context_before", "context_after"])]
    pub context: Option<usize>,

    /// Number of neighbouring lines to include before each match when expanding snippets.
    #[arg(long = "context-before", default_value_t = 0)]
    pub context_before: usize,

    /// Number of neighbouring lines to include after each match when expanding snippets.
    #[arg(long = "context-after", default_value_t = 0)]
    pub context_after: usize,

    /// Retrieve full file bodies for each surfaced hit.
    #[arg(long = "body", action = ArgAction::SetTrue, default_value_t = false)]
    pub body: bool,

    /// Limit attached bodies to at most N bytes (0 = omit body text).
    #[arg(long = "body-limit")]
    pub body_limit: Option<usize>,

    /// Disable all truncation limits (max columns = unlimited; body limit = unlimited).
    #[arg(long = "no-truncate", default_value_t = false, action = ArgAction::SetTrue)]
    pub no_truncate: bool,

    /// Restrict search to files changed in the working tree.
    #[arg(long = "git-modified", default_value_t = false, action = ArgAction::SetTrue)]
    pub git_modified: bool,

    /// Restrict search to files changed since a commit (git diff --name-only <commit>).
    #[arg(long = "git-since")]
    pub git_since: Option<String>,

    /// Attach git metadata (last commit hash/date) for each hit when available.
    #[arg(long = "git-info", action = ArgAction::SetTrue, default_value_t = false)]
    pub git_info: bool,

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

/// Arguments for the `refs` subcommand.
#[derive(clap::Args, Debug)]
pub struct RefsCommand {
    #[command(flatten)]
    pub search: SearchArgs,

    /// Include locations that call the symbol.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub callers: bool,

    /// Extract callees from the symbol's body.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub callees: bool,

    /// Show what this file/symbol imports.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub imports: bool,

    /// Show what imports this file/symbol.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub importers: bool,

    /// Depth for transitive traversal (default: 1).
    #[arg(long, default_value_t = 1)]
    pub import_depth: usize,

    /// Output style for references.
    #[arg(long, value_enum, default_value = "json")]
    pub format: RefsFormat,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum RefsFormat {
    Json,
    Tree,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ContextPreset {
    /// Callers + callees + types (refactoring-oriented).
    Refactor,
    /// Callers + definition body + tests (debugging-oriented).
    Debug,
    /// Everything (budgeted and summarized).
    Understand,
}

/// Arguments for the `context` subcommand.
#[derive(clap::Args, Debug)]
pub struct ContextCommand {
    /// Symbol to assemble context for.
    #[arg(long)]
    pub symbol: String,

    /// Preset configuration for common workflows (refactor|debug|understand).
    #[arg(long, value_enum)]
    pub preset: Option<ContextPreset>,

    /// Minimize context output (single compact JSON object; omits bodies and signatures).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue, conflicts_with = "stream")]
    pub concise: bool,

    /// Stream NDJSON sections as they are computed.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue, conflicts_with = "concise")]
    pub stream: bool,

    /// Include locations that call the symbol.
    #[arg(long, action = ArgAction::SetTrue)]
    pub callers: Option<bool>,

    /// Include callees extracted from the symbol's definition body.
    #[arg(long, action = ArgAction::SetTrue)]
    pub callees: Option<bool>,

    /// Include type relationships (best-effort).
    #[arg(long, action = ArgAction::SetTrue)]
    pub types: Option<bool>,

    /// Include import relationships (best-effort; may be empty).
    #[arg(long, action = ArgAction::SetTrue)]
    pub imports: Option<bool>,

    /// Include related test files (best-effort).
    #[arg(long, alias = "include-tests", action = ArgAction::SetTrue)]
    pub tests: Option<bool>,

    /// Include a definition body when available.
    #[arg(long = "body", action = ArgAction::SetTrue)]
    pub include_body: Option<bool>,

    /// Graph expansion depth (e.g. callers-of-callers when depth=2).
    #[arg(long)]
    pub depth: Option<usize>,

    /// Maximum total output lines before truncation.
    #[arg(long = "max-lines")]
    pub max_lines: Option<usize>,

    /// Root directory to scan; defaults to the current working directory.
    #[arg(long)]
    pub path: Option<PathBuf>,
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

/// Collect structured feedback for agent tools (starting with shell_command).
#[derive(clap::Args, Debug)]
pub struct FeedbackCommand {
    #[command(subcommand)]
    pub command: FeedbackSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum FeedbackSubcommand {
    /// Ask for improvements to the `shell_command` tool and persist the response.
    Shell(ShellFeedbackArgs),
}

#[derive(clap::Args, Debug)]
pub struct ShellFeedbackArgs {
    /// The exact command string the shell tool executed.
    #[arg(long)]
    pub command: String,

    /// Working directory used for the command.
    #[arg(long)]
    pub cwd: Option<String>,

    /// Exit code returned by the command (if known).
    #[arg(long)]
    pub exit_code: Option<i32>,

    /// Whether the command succeeded (if known).
    #[arg(long)]
    pub success: Option<bool>,

    /// Duration of the command (milliseconds, if known).
    #[arg(long)]
    pub duration_ms: Option<u64>,

    /// Improvements text. If omitted, `swe-grep` reads from stdin (pipe) or prompts (TTY).
    #[arg(long)]
    pub improvements: Option<String>,

    /// Optional rating (1-5).
    #[arg(long)]
    pub rating: Option<u8>,

    /// Optional comma-separated tags (e.g. "timeouts,streaming,redaction").
    #[arg(long)]
    pub tags: Option<String>,

    /// Output directory for feedback artifacts (defaults to ./Feedback).
    #[arg(long)]
    pub out_dir: Option<PathBuf>,

    /// Only emit the prompt (do not read feedback or write files).
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub prompt_only: bool,

    /// Emit prompt output as JSON instead of plain text.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub json: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_context_preset_and_flags() {
        let cli = Cli::try_parse_from([
            "swe-grep",
            "context",
            "--preset",
            "refactor",
            "--symbol",
            "Foo",
            "--path",
            ".",
            "--types",
            "--concise",
        ])
        .expect("expected context args to parse");

        match cli.command {
            Commands::Context(cmd) => {
                assert_eq!(cmd.symbol, "Foo");
                assert!(matches!(cmd.preset, Some(ContextPreset::Refactor)));
                assert_eq!(cmd.path, Some(PathBuf::from(".")));
                assert_eq!(cmd.types, Some(true));
                assert!(cmd.concise);
            }
            _ => panic!("expected context command"),
        }
    }
}
