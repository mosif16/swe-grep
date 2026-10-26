use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use clap::{CommandFactory, Parser};
use regex::Regex;
use serde::Serialize;
use strsim::normalized_levenshtein;

use swe_grep::agent::{
    AgentResult, AgentSettings, aggregate_output, render_compact, render_edit, render_minimal,
    render_table, render_tree_grouped, stream_entry,
};
use swe_grep::analysis::{
    CallGraph, CallTreeNode, ExtractFactsOptions, FileFacts, GraphCache, ImportGraph,
    extract_repo_facts, normalize_symbol,
};
use swe_grep::bench;
use swe_grep::cli::{
    CacheSubcommand, Cli, Commands, ContextCommand, ContextPreset, OutputFormat, RefsCommand,
    RefsFormat, RemoveCommand, SearchArgs, SearchCommand, UsageCommand, UsageFormat,
};
use swe_grep::cli::{FeedbackCommand, FeedbackSubcommand, ShellFeedbackArgs};
use swe_grep::context::selection::{
    resolve_symbol, resolve_target_file, select_symbol_facts_in_place, target_has_ambiguous_callers,
};
use swe_grep::context::{
    Callee, Caller, ContextResponse, ContextSummary, Definition, ImportContext, SymbolComplexity,
    SymbolResolution, TestFile, TypeContext,
};
use swe_grep::feedback::{
    DEFAULT_FEEDBACK_DIR, SHELL_TOOL_NAME, ShellFeedback, ShellInvocation,
    build_shell_feedback_prompt, new_shell_feedback_record, read_feedback_text_from_stdin,
    write_shell_feedback,
};
use swe_grep::search;
use swe_grep::service;
use swe_grep::telemetry;

#[derive(Debug)]
enum AppError {
    Usage(String),
    AssertionFailed(String),
    Runtime(anyhow::Error),
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Runtime(err)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Runtime(err.into())
    }
}

impl From<io::Error> for AppError {
    fn from(err: io::Error) -> Self {
        AppError::Runtime(err.into())
    }
}

#[tokio::main]
async fn main() {
    let raw_args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let cli = match Cli::try_parse_from(&raw_args) {
        Ok(cli) => cli,
        Err(err) => {
            let _ = err.print();
            maybe_print_did_you_mean_for_unknown_flags(&raw_args, &err);
            let code = if matches!(
                err.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                0
            } else {
                1
            };
            std::process::exit(code);
        }
    };

    let exit_code = match run(cli).await {
        Ok(()) => 0,
        Err(AppError::Usage(msg)) => {
            eprintln!("{msg}");
            1
        }
        Err(AppError::AssertionFailed(msg)) => {
            eprintln!("{msg}");
            1
        }
        Err(AppError::Runtime(err)) => {
            eprintln!("{err}");
            2
        }
    };

    std::process::exit(exit_code);
}

#[derive(Clone, Debug)]
struct FlagOrigin {
    flag: String,
    command_path: Vec<String>,
}

fn maybe_print_did_you_mean_for_unknown_flags(raw_args: &[std::ffi::OsString], err: &clap::Error) {
    if !matches!(err.kind(), clap::error::ErrorKind::UnknownArgument) {
        return;
    }

    let message = err.to_string();
    let Some(unknown) = extract_unknown_flag_from_error(&message) else {
        return;
    };
    if !unknown.starts_with('-') {
        return;
    }

    let suggestions = suggest_flags(&unknown, 3);
    if suggestions.is_empty() {
        return;
    }

    let invoked_subcommand = detect_invoked_subcommand(raw_args);
    for suggestion in suggestions {
        if suggestion.command_path.is_empty() {
            eprintln!("Did you mean `{}`?", suggestion.flag);
            continue;
        }

        if invoked_subcommand
            .as_deref()
            .is_some_and(|name| suggestion.command_path.first().is_some_and(|p| p == name))
        {
            eprintln!("Did you mean `{}`?", suggestion.flag);
            continue;
        }

        let owner = format!("swe-grep {}", suggestion.command_path.join(" "));
        eprintln!(
            "Did you mean `{}`? (flag for: `{}`)",
            suggestion.flag, owner
        );
        if let Some(example) = example_usage_for_flag(&suggestion.command_path, &suggestion.flag) {
            eprintln!("Example: {example}");
        }
    }
}

fn extract_unknown_flag_from_error(message: &str) -> Option<String> {
    // Example: "error: unexpected argument '--cont' found"
    let re = Regex::new("unexpected argument '([^']+)' found").ok()?;
    re.captures(message)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

fn detect_invoked_subcommand(raw_args: &[std::ffi::OsString]) -> Option<String> {
    // Best-effort: look for the first non-flag token that matches a subcommand name.
    let cmd = Cli::command();
    let subcommands: HashSet<&str> = cmd.get_subcommands().map(|s| s.get_name()).collect();
    for arg in raw_args.iter().skip(1) {
        let text = arg.to_string_lossy();
        if text.starts_with('-') {
            continue;
        }
        if subcommands.contains(text.as_ref()) {
            return Some(text.to_string());
        }
        // If the first positional isn't a subcommand, we assume simple mode.
        break;
    }
    None
}

fn suggest_flags(unknown: &str, limit: usize) -> Vec<FlagOrigin> {
    let all_flags = collect_unique_flags();
    let mut scored: Vec<(f64, FlagOrigin)> = all_flags
        .into_iter()
        .map(|entry| (normalized_levenshtein(unknown, &entry.flag), entry))
        .collect();

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.flag.cmp(&b.1.flag))
    });

    let threshold = 0.6;
    scored
        .into_iter()
        .filter(|(score, _)| *score >= threshold)
        .take(limit)
        .map(|(_, entry)| entry)
        .collect()
}

fn collect_unique_flags() -> Vec<FlagOrigin> {
    let mut all: Vec<FlagOrigin> = Vec::new();
    let cmd = Cli::command();
    collect_flags_recursive(&cmd, Vec::new(), &mut all);

    // Many flags are repeated across subcommands (e.g. --path). Prefer the shallowest owner.
    let mut best_by_flag: std::collections::HashMap<String, FlagOrigin> =
        std::collections::HashMap::new();
    for entry in all {
        best_by_flag
            .entry(entry.flag.clone())
            .and_modify(|existing| {
                if entry.command_path.len() < existing.command_path.len() {
                    *existing = entry.clone();
                }
            })
            .or_insert(entry);
    }
    best_by_flag.into_values().collect()
}

fn collect_flags_recursive(cmd: &clap::Command, path: Vec<String>, out: &mut Vec<FlagOrigin>) {
    for arg in cmd.get_arguments() {
        if let Some(long) = arg.get_long() {
            out.push(FlagOrigin {
                flag: format!("--{long}"),
                command_path: path.clone(),
            });
        }
        if let Some(short) = arg.get_short() {
            out.push(FlagOrigin {
                flag: format!("-{short}"),
                command_path: path.clone(),
            });
        }
    }

    for sub in cmd.get_subcommands() {
        let mut next = path.clone();
        next.push(sub.get_name().to_string());
        collect_flags_recursive(sub, next, out);
    }
}

fn example_usage_for_flag(command_path: &[String], flag: &str) -> Option<String> {
    let Some(first) = command_path.first().map(|s| s.as_str()) else {
        return Some(format!("swe-grep <SYMBOL> . {flag}"));
    };
    let joined = command_path.join(" ");
    match first {
        "search" | "context" | "refs" | "usages" | "remove" => {
            Some(format!("swe-grep {joined} <SYMBOL> . {flag}"))
        }
        "bench" | "serve" => Some(format!("swe-grep {joined} {flag}")),
        "feedback" => Some(format!("swe-grep {joined} {flag}")),
        _ => None,
    }
}

/// Emit a compact system prompt to stderr for AI agents
fn emit_agent_system_prompt() {
    let prompt = r#"<swe-grep-system>
swe-grep: semantic code search for AI agents

COMMANDS:
  swe-grep <symbol> .              → Basic search (file:line | code)
  swe-grep context Foo . --concise → Definition + callers/callees (JSON)
  swe-grep refs Foo . --callers    → Find all callers
  swe-grep refs Foo . --callees    → Find all callees

KEY FLAGS:
  -l N, --limit N      → Max results (default: 20)
  -i, --ignore-case    → Case-insensitive
  -f, --files-only     → Output unique file paths only
  -c, --count          → Print match count only
  --tracked-only       → Git-tracked files only (excludes lockfiles)
  --exclude "*.lock"   → Exclude patterns
  --ext swift          → Filter by extension
  --text               → Literal text search (not symbol)
  --json-only          → JSON output

EXAMPLES:
  swe-grep AuthService src/              → Find AuthService in src/
  swe-grep fetch . --tracked-only -f     → Files with 'fetch', no lockfiles
  swe-grep context UserStore . --concise → Get UserStore call graph
</swe-grep-system>
"#;
    eprintln!("{}", prompt);
}

fn print_agent_hint(format: &str) {
    let hint_md = r#"# swe-grep Agent Guide

**swe-grep** is a semantic code search tool optimized for AI agents.

## Quick Start
```bash
swe-grep <symbol[,symbol...]> [path]        # Basic symbol search
swe-grep context Foo . --concise  # Get definition with callers/callees
swe-grep refs Foo . --callers     # Find all callers of Foo
```

## Core Commands

| Command | Description | Example |
|---------|-------------|---------|
| `swe-grep <symbol> .` | Basic symbol search | `swe-grep AuthService .` |
| `swe-grep context` | Definition + call graph | `swe-grep context Foo . --concise` |
| `swe-grep refs --callers` | Find callers | `swe-grep refs Foo . --callers` |
| `swe-grep refs --callees` | Find callees | `swe-grep refs Foo . --callees` |
| `swe-grep search` | Advanced search | `swe-grep search Foo . --agent` |

## Key Flags (work in simple mode: `swe-grep <symbol> .`)

### Filtering
- `-l, --limit N` — Max results (default: 20)
- `-i, --ignore-case` — Case-insensitive search
- `-e, --ext EXT` — Filter by extension (e.g., `--ext swift`)
- `--path-glob GLOB` — Include glob patterns (e.g., `src/**/*.ts`)
- `--exclude GLOB` — Exclude glob patterns (e.g., `*.md,*.lock`)
- `--exclude-tests` — Exclude test files
- `--code-only` — Exclude docs/configs
- `-d, --definitions-only` — Only show definitions

### Git Integration
- `--git-modified` — Only modified files
- `--git-since COMMIT` — Files changed since commit
- `--tracked-only` — Only git-tracked files (excludes lockfiles/generated)

### Output Control
- `-f, --files-only` — Output unique file paths only
- `-c, --count` — Output match counts only
- `-C N` — Context lines before/after
- `--json-only` — Pipe-friendly JSON output
- `--regex` — Treat symbol as regex
- `--text` — Literal string search (not just symbols)
- `--string` — Search inside string literals

## Common Patterns

```bash
# Find all usages of a function
swe-grep MyFunction src/

# Get definition + callers (structured JSON)
swe-grep context MyClass . --concise

# Search only in Swift files, exclude tests
swe-grep ViewModel . --ext swift --exclude-tests

# Find in git-tracked files only (no lockfiles)
swe-grep fetch . --tracked-only

# List only affected files
swe-grep deprecated . --files-only

# Case-insensitive with limit
swe-grep error . -i -l 5

# Literal text search (not symbol)
swe-grep "TODO:" . --text

# JSON output for parsing
swe-grep AuthService . --json-only
```

## Output Format
Default: `file:line | code snippet`
JSON: Use `--json-only` for structured output
"#;

    let hint_json = serde_json::json!({
        "tool": "swe-grep",
        "version": env!("CARGO_PKG_VERSION"),
        "commands": {
            "simple": {
                "syntax": "swe-grep <symbol[,symbol...]> [path]",
                "description": "Basic symbol search",
                "example": "swe-grep AuthService ."
            },
            "context": {
                "syntax": "swe-grep context <symbol> <path> --concise",
                "description": "Get definition with callers/callees as JSON",
                "example": "swe-grep context Foo . --concise"
            },
            "refs": {
                "syntax": "swe-grep refs <symbol> <path> --callers|--callees",
                "description": "Find callers or callees of a symbol",
                "example": "swe-grep refs Foo . --callers"
            },
            "search": {
                "syntax": "swe-grep search <symbol> <path> [options]",
                "description": "Advanced search with agent mode",
                "example": "swe-grep search Foo . --agent --format json"
            }
        },
        "flags": {
            "filtering": {
                "--limit, -l": "Max results (default: 20)",
                "--ignore-case, -i": "Case-insensitive search",
                "--ext, -e": "Filter by extension",
                "--path-glob": "Include glob patterns",
                "--exclude": "Exclude glob patterns",
                "--exclude-tests": "Exclude test files",
                "--code-only": "Exclude docs/configs",
                "--definitions-only, -d": "Only definitions"
            },
            "git": {
                "--git-modified": "Only modified files",
                "--git-since": "Files changed since commit",
                "--tracked-only": "Only git-tracked files"
            },
            "output": {
                "--files-only, -f": "Output unique file paths only",
                "--count, -c": "Output match counts only",
                "--context, -C": "Context lines",
                "--json-only": "JSON output",
                "--regex": "Regex mode",
                "--text": "Literal text search",
                "--string": "Search in string literals"
            }
        },
        "tips": [
            "Use --tracked-only to exclude lockfiles and generated files",
            "Use --files-only for quick impact assessment",
            "Use --concise with context for structured call graph data",
            "Use --json-only for pipe-friendly structured output"
        ]
    });

    match format.to_lowercase().as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&hint_json).unwrap()),
        _ => println!("{}", hint_md),
    }
}

async fn run(cli: Cli) -> Result<(), AppError> {
    let disable_telemetry =
        cli.disable_telemetry || (!cli.no_config && telemetry_default_disabled());
    if cli.clear_cache {
        clear_cache_for_command(&cli)?;
    }

    // Emit agent hint and exit early
    if let Some(format) = &cli.agent_hint {
        print_agent_hint(format);
        return Ok(());
    }

    // Emit compact agent hint to stderr (agents will see this) unless suppressed
    let quiet_hint = cli.quiet_hint
        || env_flag_enabled("SWE_GREP_QUIET_HINT")
        || env_flag_enabled("SWE_GREP_NO_BANNER");
    if !quiet_hint {
        emit_agent_system_prompt();
    }

    // Simple mode: `swe-grep <symbol> [language|path]`
    if cli.command.is_none() {
        if let Some(symbol) = cli.symbol.clone() {
            return handle_simple_search(symbol, &cli, disable_telemetry).await;
        }
        // No symbol provided, show help
        return Err(AppError::Usage(
            "Usage: swe-grep <symbol> [language|path]\n\nExamples:\n  swe-grep AuthService\n  swe-grep AuthService rust\n  swe-grep AuthService src/\n\nSubcommands also accept positional shorthand:\n  swe-grep search AuthService src/\n  swe-grep context AuthService .\n  swe-grep refs AuthService src/ --callers".to_string()
        ));
    }

    match cli.command.unwrap() {
        Commands::Search(cmd) => handle_search(cmd, disable_telemetry).await,
        Commands::Usages(cmd) => handle_usages(cmd, disable_telemetry).await,
        Commands::Refs(cmd) => handle_refs(cmd, disable_telemetry).await,
        Commands::Context(cmd) => handle_context(cmd, disable_telemetry).await,
        Commands::Remove(cmd) => handle_remove(cmd, disable_telemetry).await,
        Commands::Feedback(cmd) => handle_feedback(cmd).await,
        Commands::Cache(cmd) => {
            telemetry::silence_tracing();
            match cmd.command {
                CacheSubcommand::Gc(args) => {
                    let base = swe_grep::cache_gc::managed_base().ok_or_else(|| {
                        AppError::Usage(
                            "Cannot resolve the managed cache base from XDG_CACHE_HOME or HOME"
                                .into(),
                        )
                    })?;
                    let seconds = args
                        .max_age_days
                        .checked_mul(86400)
                        .ok_or_else(|| AppError::Usage("--max-age-days is too large".into()))?;
                    let policy = swe_grep::cache_gc::GcPolicy {
                        max_age: std::time::Duration::from_secs(seconds),
                        max_bytes: args.max_bytes,
                        ..Default::default()
                    };
                    let report = swe_grep::cache_gc::collect(&base, &policy, args.apply)?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                    Ok(())
                }
            }
        }
        Commands::Bench(args) => {
            if !disable_telemetry {
                telemetry::init()?;
            }
            bench::run(args).await?;
            Ok(())
        }
        Commands::Serve(args) => {
            if !disable_telemetry {
                telemetry::init()?;
            }
            service::serve(args).await?;
            Ok(())
        }
    }
}

/// Simple mode: `swe-grep <symbol> [language]`
/// Output: file:line | code context
async fn handle_simple_search(
    symbol_input: String,
    cli: &Cli,
    _disable_telemetry: bool,
) -> Result<(), AppError> {
    use swe_grep::cli::{SearchArgs, ToolArgs};

    // Simple mode is always quiet - no telemetry, no logs

    // Parse lang_or_path: could be a language (rust, swift, ts) or a path (src/ or a file)
    let (language, path, scope_file) = match cli.lang_or_path.as_deref() {
        Some(arg) => match parse_lang_or_path_hint(arg) {
            LangOrPathHint::Language(lang) => (Some(lang), PathBuf::from("."), None),
            LangOrPathHint::Path(path) => {
                if path.is_file() {
                    let canonical = path.canonicalize().unwrap_or(path);
                    let parent = canonical
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| PathBuf::from("."));
                    (None, parent, Some(canonical))
                } else {
                    (None, path, None)
                }
            }
        },
        None => (None, PathBuf::from("."), None),
    };

    // Resolve --path flag (takes precedence over positional)
    let resolved_path = cli.path.clone().unwrap_or(path.clone());

    // Handle --ext flag by converting to path_glob
    let mut path_glob = cli.path_glob.clone();
    if let Some(ext) = &cli.ext {
        path_glob.push(format!("**/*.{}", ext));
    }

    // Calculate context lines
    let (context_before, context_after) = if let Some(c) = cli.context {
        (c, c)
    } else {
        (
            cli.context_before.unwrap_or(0),
            cli.context_after.unwrap_or(0),
        )
    };

    let split_commas = !cli.regex && !cli.text;
    let symbols = parse_simple_symbols(&symbol_input, split_commas);
    if symbols.is_empty() {
        return Err(AppError::Usage("symbol is required".to_string()));
    }

    let max_matches = if cli.count { 10_000 } else { 50 };
    let top_hits = if cli.count {
        1
    } else {
        cli.limit.unwrap_or(20)
    };

    let base_args = SearchArgs {
        symbol: String::new(),
        path: Some(resolved_path.clone()),
        scope_file,
        language,
        max_matches,
        top_hits,
        context_before,
        context_after,
        body: false,
        timeout_secs: 10,
        ignore_case: cli.ignore_case,
        fuzzy_threshold: 0.7,
        regex: cli.regex,
        text: cli.text,
        max_columns: 200,
        concurrency: 8,
        tools: ToolArgs {
            enable_index: false,
            enable_rga: false,
            use_fd: true,
            use_ast_grep: true,
        },
        git_modified: cli.git_modified,
        git_since: cli.git_since.clone(),
        path_glob,
        exclude_patterns: cli.exclude_patterns.clone(),
        definitions_only: cli.definitions_only,
        exact_symbol: cli.exact_symbol,
        exclude_tests: cli.exclude_tests,
        code_only: cli.code_only,
        tracked_only: cli.tracked_only,
        ..Default::default()
    };

    let display_root = resolved_path;

    let run = |symbol: &str| {
        let symbol = symbol.to_string();
        let mut args = base_args.clone();
        args.symbol = symbol;
        async move { search::execute(args).await.map_err(AppError::from) }
    };

    // --count: print match count only
    if cli.count {
        if symbols.len() == 1 {
            let summary = run(&symbols[0]).await?;
            if cli.json_only {
                println!("{}", serde_json::to_string(&summary.deduped)?);
            } else {
                println!("{}", summary.deduped);
            }
            return Ok(());
        }

        if cli.json_only {
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for symbol in &symbols {
                let summary = run(symbol).await?;
                counts.insert(symbol.clone(), summary.deduped);
            }
            println!("{}", serde_json::to_string(&counts)?);
            return Ok(());
        }

        for symbol in &symbols {
            let summary = run(symbol).await?;
            println!("{}\t{}", symbol, summary.deduped);
        }
        return Ok(());
    }

    // JSON output mode
    if cli.json_only {
        if cli.files_only {
            let mut seen: HashSet<String> = HashSet::new();
            let mut files: Vec<String> = Vec::new();
            for symbol in &symbols {
                let summary = run(symbol).await?;
                for hit in &summary.top_hits {
                    let file_path = Path::new(&hit.path);
                    let display_path = file_path
                        .strip_prefix(&display_root)
                        .unwrap_or(file_path)
                        .to_string_lossy()
                        .to_string();
                    if seen.insert(display_path.clone()) {
                        files.push(display_path);
                    }
                }
            }
            println!("{}", serde_json::to_string(&files)?);
            return Ok(());
        }

        if symbols.len() == 1 {
            let summary = run(&symbols[0]).await?;
            let json = serde_json::to_string(&summary.top_hits)?;
            println!("{}", json);
            return Ok(());
        }

        let mut hits_by_symbol: BTreeMap<String, Vec<search::TopHit>> = BTreeMap::new();
        for symbol in &symbols {
            let summary = run(symbol).await?;
            hits_by_symbol.insert(symbol.clone(), summary.top_hits);
        }
        println!("{}", serde_json::to_string(&hits_by_symbol)?);
        return Ok(());
    }

    // --files-only: unique file paths only (across all symbols)
    if cli.files_only {
        let mut seen: HashSet<String> = HashSet::new();
        for symbol in &symbols {
            let summary = run(symbol).await?;
            for hit in &summary.top_hits {
                let file_path = Path::new(&hit.path);
                let display_path = file_path
                    .strip_prefix(&display_root)
                    .unwrap_or(file_path)
                    .to_string_lossy()
                    .to_string();
                if seen.insert(display_path.clone()) {
                    println!("{}", display_path);
                }
            }
        }
        return Ok(());
    }

    let wants_context = context_before > 0 || context_after > 0;

    for (index, symbol) in symbols.iter().enumerate() {
        let summary = run(symbol).await?;
        if symbols.len() > 1 {
            println!("== {} ==", symbol);
        }

        // Output: file:line | snippet
        if summary.top_hits.is_empty() {
            if symbols.len() == 1 {
                println!("No results for '{}'", symbol);
            } else {
                println!("No results");
            }
            print_simple_no_hit_suggestions(&summary);
        } else {
            for hit in &summary.top_hits {
                // Strip root prefix for cleaner output
                let file_path = Path::new(&hit.path);
                let display_path = file_path.strip_prefix(&display_root).unwrap_or(file_path);
                if wants_context {
                    if let Some(expanded) = hit.expanded_snippet.as_deref() {
                        for line in expanded.lines() {
                            let mut parts = line.splitn(2, ' ');
                            let line_no = parts.next().unwrap_or("").trim();
                            let text = parts.next().unwrap_or("").trim_end();
                            if line_no.is_empty() {
                                continue;
                            }
                            let parsed_line = line_no.parse::<usize>().unwrap_or(hit.line);
                            println!("{}:{} | {}", display_path.display(), parsed_line, text);
                        }
                        continue;
                    }
                }

                let snippet = hit.snippet.as_deref().unwrap_or("");
                let trimmed = snippet.trim();
                println!("{}:{} | {}", display_path.display(), hit.line, trimmed);
            }

            if summary.deduped > summary.top_hits.len() {
                let remaining = summary.deduped - summary.top_hits.len();
                if summary.truncated {
                    println!(
                        "... {}+ more results not shown (use --limit N or swe-grep search --max-matches)",
                        remaining
                    );
                } else {
                    println!("... {} more results not shown (use --limit N)", remaining);
                }
            }
        }

        if symbols.len() > 1 && index + 1 != symbols.len() {
            println!();
        }
    }

    Ok(())
}

fn parse_simple_symbols(value: &str, split_commas: bool) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();

    let parts: Vec<&str> = if split_commas && value.contains(',') {
        value.split(',').collect()
    } else {
        vec![value]
    };

    for part in parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }

    out
}

fn print_simple_no_hit_suggestions(summary: &search::SearchSummary) {
    if !summary.fuzzy_matches.is_empty() {
        let suggestions: Vec<String> = summary
            .fuzzy_matches
            .iter()
            .take(3)
            .map(|m| format!("{} ({})", m.symbol, m.file))
            .collect();
        println!("Did you mean: {}", suggestions.join(", "));
    } else if !summary.suggested_symbols.is_empty() {
        let suggestions: Vec<String> = summary.suggested_symbols.iter().take(3).cloned().collect();
        println!("Did you mean: {}", suggestions.join(", "));
    }
}

fn env_flag_enabled(key: &str) -> bool {
    match std::env::var(key) {
        Ok(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => false,
    }
}

#[derive(Debug, Clone)]
enum LangOrPathHint {
    Language(String),
    Path(PathBuf),
}

fn is_language_hint(value: &str) -> bool {
    // Keep this intentionally permissive for ergonomics; if the user passes an unrecognized
    // token it will be treated as a path instead.
    matches!(
        value,
        "rust"
            | "rs"
            | "swift"
            | "typescript"
            | "ts"
            | "tsx"
            | "javascript"
            | "js"
            | "go"
            | "python"
            | "py"
            | "css"
            | "scss"
            | "auto"
            | "auto-swift-ts"
    )
}

fn parse_lang_or_path_hint(value: &str) -> LangOrPathHint {
    let normalized = value.trim().to_lowercase();
    if is_language_hint(normalized.as_str()) {
        return LangOrPathHint::Language(value.to_string());
    }
    LangOrPathHint::Path(PathBuf::from(value))
}

fn apply_positional_query_to_search_args(
    args: &mut SearchArgs,
    positional_symbol: Option<&str>,
    positional_lang_or_path: Option<&str>,
) -> Result<(), AppError> {
    if let Some(positional_symbol) = positional_symbol {
        if !args.symbol.trim().is_empty() {
            return Err(AppError::Usage(
                "symbol specified twice (use either --symbol or the positional SYMBOL)".to_string(),
            ));
        }
        args.symbol = positional_symbol.to_string();
    }

    if let Some(positional_lang_or_path) = positional_lang_or_path {
        match parse_lang_or_path_hint(positional_lang_or_path) {
            LangOrPathHint::Language(language) => {
                if args.language.is_some() {
                    return Err(AppError::Usage(
                        "language specified twice (use either --language or the positional LANG_OR_PATH)"
                            .to_string(),
                    ));
                }
                args.language = Some(language);
            }
            LangOrPathHint::Path(path) => {
                if args.path.is_some() || args.scope_file.is_some() {
                    return Err(AppError::Usage(
                        "path specified twice (use either --path or the positional LANG_OR_PATH)"
                            .to_string(),
                    ));
                }
                if path.is_file() {
                    let canonical = path.canonicalize().unwrap_or(path);
                    args.scope_file = Some(canonical.clone());
                    args.path = Some(
                        canonical
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| PathBuf::from(".")),
                    );
                } else {
                    args.path = Some(path);
                }
            }
        }
    }

    Ok(())
}

async fn handle_feedback(cmd: FeedbackCommand) -> Result<(), AppError> {
    match cmd.command {
        FeedbackSubcommand::Shell(args) => handle_shell_feedback(args),
    }
}

fn handle_shell_feedback(args: ShellFeedbackArgs) -> Result<(), AppError> {
    let invocation = ShellInvocation {
        tool: SHELL_TOOL_NAME.to_string(),
        command: args.command,
        cwd: args.cwd,
        exit_code: args.exit_code,
        success: args.success,
        duration_ms: args.duration_ms,
    };

    let prompt = build_shell_feedback_prompt(&invocation);
    if args.prompt_only {
        if args.json {
            let payload = serde_json::json!({
                "tool": SHELL_TOOL_NAME,
                "prompt": prompt,
                "response_shape": {"improvements":"...","rating":1,"tags":["..."]},
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        } else {
            print!("{prompt}");
        }
        return Ok(());
    }

    let mut rating = args.rating;
    let mut tags: Vec<String> = args
        .tags
        .unwrap_or_default()
        .split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();

    let input = match args.improvements {
        Some(improvements) => improvements,
        None => {
            if let Some(piped) = read_feedback_text_from_stdin()? {
                piped
            } else {
                eprintln!("{prompt}");
                eprintln!("Paste feedback JSON or text, then press Ctrl-D:");
                let mut buf = String::new();
                io::stdin().read_to_string(&mut buf)?;
                buf.trim().to_string()
            }
        }
    };

    let improvements = if let Ok(parsed) = serde_json::from_str::<ShellFeedback>(&input) {
        if rating.is_none() {
            rating = parsed.rating;
        }
        if tags.is_empty() {
            tags = parsed.tags;
        }
        parsed.improvements
    } else {
        input
    };

    if improvements.trim().is_empty() {
        return Err(AppError::Usage(
            "feedback text is required (use --improvements or pipe via stdin)".to_string(),
        ));
    }

    let out_dir = args.out_dir.unwrap_or_else(|| {
        invocation
            .cwd
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(DEFAULT_FEEDBACK_DIR)
    });
    let record = new_shell_feedback_record(invocation, improvements, rating, tags)?;
    let record_path = write_shell_feedback(out_dir, &record)?;
    println!("{}", record_path.display());
    Ok(())
}

async fn handle_search(mut cmd: SearchCommand, disable_telemetry: bool) -> Result<(), AppError> {
    apply_positional_query_to_search_args(
        &mut cmd.search,
        cmd.positional.symbol_pos.as_deref(),
        cmd.positional.lang_or_path_pos.as_deref(),
    )?;
    // Positional args are only an input convenience; from this point on we operate on the resolved
    // SearchArgs to avoid double-counting `symbol` in downstream validation/collection.
    cmd.positional = swe_grep::cli::PositionalQueryArgs::default();
    let symbols = collect_symbols(&cmd)?;

    if cmd.json_only {
        cmd.agent = true;
        cmd.format = OutputFormat::Json;
        cmd.quiet = true;
        cmd.progress = false;
        cmd.stream = false;
    }

    if cmd.progress && !cmd.agent {
        return Err(AppError::Usage(
            "--progress is only available in --agent mode".to_string(),
        ));
    }
    if cmd.concise && !cmd.agent {
        return Err(AppError::Usage(
            "--concise is only available in --agent mode".to_string(),
        ));
    }
    if cmd.json_pretty && !cmd.agent {
        return Err(AppError::Usage(
            "--json-pretty is only available in --agent mode".to_string(),
        ));
    }
    if cmd.json_pretty && cmd.stream {
        return Err(AppError::Usage(
            "--json-pretty cannot be combined with --stream".to_string(),
        ));
    }
    if cmd.grouped && !cmd.agent {
        return Err(AppError::Usage(
            "--grouped is only available in --agent mode".to_string(),
        ));
    }
    if cmd.budget.is_some() && !cmd.grouped {
        return Err(AppError::Usage("--budget requires --grouped".to_string()));
    }
    if cmd.grouped && !matches!(cmd.format, OutputFormat::Json) {
        return Err(AppError::Usage(
            "--grouped currently only supports --format json".to_string(),
        ));
    }
    if cmd.grouped && cmd.open {
        return Err(AppError::Usage(
            "--grouped cannot be combined with --open".to_string(),
        ));
    }
    if cmd.json_pretty
        && matches!(
            cmd.format,
            OutputFormat::Table
                | OutputFormat::Compact
                | OutputFormat::Tree
                | OutputFormat::Edit
                | OutputFormat::Minimal
        )
    {
        return Err(AppError::Usage(
            "--json-pretty requires --format json".to_string(),
        ));
    }
    if cmd.trim_body.is_some() && !cmd.agent {
        return Err(AppError::Usage(
            "--trim-body is only available in --agent mode".to_string(),
        ));
    }
    if (cmd.body_head.is_some() || cmd.body_tail.is_some()) && !cmd.agent {
        return Err(AppError::Usage(
            "--body-head/--body-tail are only available in --agent mode".to_string(),
        ));
    }
    if cmd.full_body && !cmd.agent {
        return Err(AppError::Usage(
            "--full-body is only available in --agent mode".to_string(),
        ));
    }
    if cmd.full_body && cmd.trim_body.is_some() {
        return Err(AppError::Usage(
            "--full-body cannot be combined with --trim-body".to_string(),
        ));
    }
    if cmd.full_body && (cmd.body_head.is_some() || cmd.body_tail.is_some()) {
        return Err(AppError::Usage(
            "--full-body cannot be combined with --body-head/--body-tail".to_string(),
        ));
    }
    if (cmd.body_head.is_some() || cmd.body_tail.is_some()) && cmd.trim_body.is_some() {
        return Err(AppError::Usage(
            "--body-head/--body-tail cannot be combined with --trim-body".to_string(),
        ));
    }
    if cmd.stream && !cmd.agent {
        return Err(AppError::Usage(
            "--stream is only available in --agent mode".to_string(),
        ));
    }
    if cmd.stream && !matches!(cmd.format, OutputFormat::Json) {
        return Err(AppError::Usage(
            "--stream currently only supports --format json".to_string(),
        ));
    }
    if cmd.open && cmd.stream {
        return Err(AppError::Usage(
            "--open cannot be combined with --stream".to_string(),
        ));
    }

    if cmd.quiet || cmd.agent {
        telemetry::silence_tracing();
    }

    let init_telemetry = !disable_telemetry && !cmd.quiet && !cmd.agent;
    if init_telemetry {
        telemetry::init()?;
    }

    if cmd.agent {
        run_agent_search(
            cmd.search,
            symbols,
            cmd.format,
            cmd.stream,
            cmd.grouped,
            cmd.budget,
            cmd.definitions,
            cmd.trim_body,
            cmd.body_head,
            cmd.body_tail,
            cmd.full_body,
            cmd.concise,
            cmd.json_pretty,
            cmd.quiet,
            cmd.progress,
            cmd.open,
            cmd.open_command.clone(),
            cmd.assert_absent,
        )
        .await
    } else {
        run_default_search(
            cmd.search,
            symbols,
            cmd.assert_absent,
            cmd.open,
            cmd.open_command,
        )
        .await
    }
}

async fn run_default_search(
    mut args: SearchArgs,
    symbols: Vec<String>,
    assert_absent: bool,
    open: bool,
    open_command: Option<String>,
) -> Result<(), AppError> {
    if symbols.is_empty() {
        return Err(AppError::Usage("symbol is required".to_string()));
    }
    let open_command = open_command.as_deref();
    if symbols.len() == 1 {
        if args.symbol.is_empty() {
            args.symbol = symbols[0].clone();
        }
        let mut summary = search::execute(args).await?;
        if open {
            summary.open_commands =
                build_open_commands_from_top_hits(&summary.top_hits, open_command);
        }
        if assert_absent && !summary.top_hits.is_empty() {
            return Err(AppError::AssertionFailed(format!(
                "symbol `{}` found ({} hits)",
                summary.symbol, summary.deduped
            )));
        }
        let json = serde_json::to_string_pretty(&summary)?;
        println!("{json}");
        return Ok(());
    }

    let mut summaries = Vec::new();
    let mut found_hits = false;
    for symbol in symbols {
        let mut per_symbol_args = args.clone();
        per_symbol_args.symbol = symbol;
        let mut summary = search::execute(per_symbol_args).await?;
        found_hits |= !summary.top_hits.is_empty();
        if open {
            summary.open_commands =
                build_open_commands_from_top_hits(&summary.top_hits, open_command);
        }
        summaries.push(summary);
    }

    let json = serde_json::to_string_pretty(&summaries)?;
    println!("{json}");

    if assert_absent && found_hits {
        return Err(AppError::AssertionFailed(
            "symbol found while --assert-absent is enabled".to_string(),
        ));
    }

    Ok(())
}

async fn run_agent_search(
    mut args: SearchArgs,
    symbols: Vec<String>,
    format: OutputFormat,
    stream: bool,
    grouped: bool,
    budget: Option<usize>,
    definitions: bool,
    trim_body: Option<usize>,
    body_head: Option<usize>,
    body_tail: Option<usize>,
    full_body: bool,
    concise: bool,
    json_pretty: bool,
    quiet: bool,
    progress: bool,
    open: bool,
    open_command: Option<String>,
    assert_absent: bool,
) -> Result<(), AppError> {
    let mut trim_body = trim_body;
    apply_agent_defaults(&mut args, &mut trim_body, full_body, body_head, body_tail);
    let settings = AgentSettings {
        include_definitions: definitions,
        trim_body,
        body_head,
        body_tail,
        concise,
    };

    let mut results = Vec::new();
    let mut root_hint: Option<PathBuf> = None;
    let mut had_error = false;
    let mut found_hits = false;
    let total = symbols.len();
    let open_command = open_command.as_deref();

    for (idx, symbol) in symbols.iter().enumerate() {
        let mut per_symbol_args = args.clone();
        per_symbol_args.symbol = symbol.clone();

        if progress && !quiet {
            eprintln!("[{}/{}] Searching {}...", idx + 1, total, symbol);
        }

        match search::execute(per_symbol_args).await {
            Ok(summary) => {
                found_hits |= !summary.top_hits.is_empty();
                root_hint.get_or_insert(summary.root.clone());
                let result = AgentResult::from_summary(symbol, &summary, &settings);
                if stream {
                    if grouped {
                        emit_json_line(&swe_grep::agent::grouped_stream_entry(
                            &result,
                            idx + 1 == symbols.len(),
                            budget,
                        ))?;
                    } else {
                        emit_json_line(&stream_entry(&result, idx + 1 == symbols.len()))?;
                    }
                } else {
                    results.push(result);
                }
            }
            Err(err) => {
                had_error = true;
                let mut result = AgentResult::default();
                result.symbol = symbol.clone();
                result.errors.push(err.to_string());

                if stream {
                    if grouped {
                        emit_json_line(&swe_grep::agent::grouped_stream_entry(
                            &result,
                            idx + 1 == symbols.len(),
                            budget,
                        ))?;
                    } else {
                        emit_json_line(&stream_entry(&result, idx + 1 == symbols.len()))?;
                    }
                } else {
                    results.push(result);
                }
            }
        }
    }

    if stream {
        if had_error {
            return Err(AppError::Runtime(anyhow!("one or more searches failed")));
        }
        if assert_absent && found_hits {
            return Err(AppError::AssertionFailed(
                "symbol found while --assert-absent is enabled".to_string(),
            ));
        }
        return Ok(());
    }

    let root = root_hint.unwrap_or(resolve_root(&args)?);
    if grouped {
        let output = swe_grep::agent::aggregate_grouped_output(&root, &symbols, &results, budget);
        if assert_absent && output.summary.count > 0 {
            return Err(AppError::AssertionFailed(format!(
                "symbol(s) found ({} hits)",
                output.summary.count
            )));
        }
        let rendered = if json_pretty {
            serde_json::to_string_pretty(&output)?
        } else {
            serde_json::to_string(&output)?
        };
        println!("{rendered}");
        return Ok(());
    }

    let mut output = aggregate_output(&root, &symbols, &results);
    if open {
        output.open_commands = build_open_commands_from_agent_hits(&output.hits, open_command);
    }
    if assert_absent && output.summary.count > 0 {
        return Err(AppError::AssertionFailed(format!(
            "symbol(s) found ({} hits)",
            output.summary.count
        )));
    }

    match format {
        OutputFormat::Json => {
            let rendered = if json_pretty {
                serde_json::to_string_pretty(&output)?
            } else {
                serde_json::to_string(&output)?
            };
            println!("{rendered}");
        }
        OutputFormat::Table => {
            let table = render_table(&output.hits);
            println!("{table}");
        }
        OutputFormat::Compact => {
            let compact = render_compact(&output.hits);
            println!("{compact}");
        }
        OutputFormat::Tree => {
            let tree = render_tree_grouped(&output.hits);
            println!("{tree}");
        }
        OutputFormat::Edit => {
            let edit = render_edit(&output.hits);
            println!("{edit}");
        }
        OutputFormat::Minimal => {
            println!("hits: {}", output.summary.count);
            let minimal = render_minimal(&output.hits);
            println!("{minimal}");
        }
        OutputFormat::Summary => {
            let summary_view = swe_grep::agent::summarize_hits(&output.hits);
            let rendered = if json_pretty {
                serde_json::to_string_pretty(&summary_view)?
            } else {
                serde_json::to_string(&summary_view)?
            };
            println!("{rendered}");
        }
    }

    if !matches!(format, OutputFormat::Json) && output.summary.count == 0 {
        if !output.suggested_symbols.is_empty() {
            println!("Did you mean: {}", output.suggested_symbols.join(", "));
        }
        if !output.fd_candidates.is_empty() {
            println!("Candidate files:");
            for candidate in &output.fd_candidates {
                println!("  - {candidate}");
            }
        }
    }

    if open
        && !output.open_commands.is_empty()
        && !matches!(format, OutputFormat::Json | OutputFormat::Summary)
    {
        for cmd in &output.open_commands {
            println!("{cmd}");
        }
    }

    if had_error {
        return Err(AppError::Runtime(anyhow!("one or more searches failed")));
    }

    Ok(())
}

async fn handle_usages(cmd: UsageCommand, disable_telemetry: bool) -> Result<(), AppError> {
    if !disable_telemetry {
        telemetry::init()?;
    }
    let mut cmd = cmd;
    apply_positional_query_to_search_args(
        &mut cmd.search,
        cmd.positional.symbol_pos.as_deref(),
        cmd.positional.lang_or_path_pos.as_deref(),
    )?;
    let mut args = cmd.search.clone();
    if args.context_before == 0 && args.context_after == 0 {
        args.context_before = 2;
        args.context_after = 2;
    }
    if !args.body {
        args.body = true;
    }
    let summary = search::execute(args).await?;
    let output = render_usage_output(&summary, cmd.format)?;
    println!("{output}");
    Ok(())
}

async fn handle_context(cmd: ContextCommand, _disable_telemetry: bool) -> Result<(), AppError> {
    // Silence tracing by default for context (outputs structured JSON)
    telemetry::silence_tracing();

    if cmd.budget.is_some() && !cmd.grouped {
        return Err(AppError::Usage("--budget requires --grouped".to_string()));
    }
    if cmd.grouped && cmd.stream {
        return Err(AppError::Usage(
            "--grouped cannot be combined with --stream".to_string(),
        ));
    }

    let symbol = match (&cmd.symbol, &cmd.symbol_pos) {
        (Some(_), Some(_)) => {
            return Err(AppError::Usage(
                "symbol specified twice (use either --symbol or the positional SYMBOL)".to_string(),
            ));
        }
        (Some(symbol), None) => symbol.clone(),
        (None, Some(symbol)) => symbol.clone(),
        (None, None) => {
            return Err(AppError::Usage(
                "symbol is required (use --symbol or the positional SYMBOL)".to_string(),
            ));
        }
    };
    let symbol = symbol.trim().to_string();
    if symbol.is_empty() {
        return Err(AppError::Usage("symbol is required".to_string()));
    }

    if cmd.path.is_some() && cmd.path_pos.is_some() && cmd.path != cmd.path_pos {
        return Err(AppError::Usage(
            "path specified twice (use either --path or the positional PATH)".to_string(),
        ));
    }
    let path = cmd.path.as_ref().or(cmd.path_pos.as_ref());

    let selection = ContextSelection::from_command(&cmd);
    let root = resolve_context_root(path)?;
    let cache_dir = search::default_cache_dir_for_root(&root);
    let _cache_lease =
        swe_grep::cache_gc::CacheLease::acquire(&root, &cache_dir).map_err(|err| {
            anyhow!(
                "failed to extract repository facts for {}: cache lifecycle unavailable: {err:#}",
                root.display()
            )
        })?;

    let languages = swe_grep::language::expand_language_hint(cmd.language.as_deref());
    let extensions = swe_grep::language::extensions_for_languages(&languages, false)
        .map(|exts| exts.into_iter().map(str::to_string).collect());
    let options = ExtractFactsOptions::for_search(None, extensions);
    let (_stats, mut file_facts) =
        extract_repo_facts(&root, &cache_dir, &options).map_err(|err| {
            anyhow!(
                "failed to extract repository facts for {}: {err:#}",
                root.display()
            )
        })?;

    let target_file = resolve_target_file(&root, cmd.target_file.as_deref())?;
    let resolution = resolve_symbol(
        &file_facts,
        &symbol,
        target_file.as_deref(),
        cmd.target_line,
    );
    let ambiguous_callers = selection.include_callers
        && resolution
            .candidates
            .first()
            .is_some_and(|target| target_has_ambiguous_callers(&file_facts, target));
    if let Some(target) = resolution
        .candidates
        .first()
        .filter(|_| resolution.status == "resolved")
    {
        select_symbol_facts_in_place(&mut file_facts, target);
    }
    let mut response = if resolution.status == "resolved" {
        assemble_context_from_facts(&root, &symbol, &selection, &file_facts)?
    } else {
        ContextResponse::empty(symbol.clone())
    };
    response.resolution = resolution;
    for (name, requested) in [
        ("callers", selection.include_callers),
        ("callees", selection.include_callees),
        ("tests", selection.include_tests),
        ("types", selection.include_types),
        ("imports", selection.include_imports),
    ] {
        response.analysis.insert(
            name.to_string(),
            if !requested {
                "not_requested"
            } else if response.resolution.status != "resolved" || response.truncated {
                "incomplete"
            } else {
                "complete"
            }
            .to_string(),
        );
    }
    // Import context is a placeholder, not an exhaustive successful analysis.
    if selection.include_imports {
        response
            .analysis
            .insert("imports".into(), "incomplete".into());
    }
    if ambiguous_callers {
        response.callers.clear();
        response
            .analysis
            .insert("callers".into(), "incomplete".into());
        response.notes.push("Name-based analysis cannot attribute callers to duplicate declarations; ambiguous caller edges omitted.".into());
    }
    response.summary.callers = response.callers.len();
    if cmd.stream {
        stream_context(&response)?;
        return Ok(());
    }

    if cmd.grouped {
        let grouped = GroupedContextOutput::from_response(&response, cmd.budget);
        let json = serde_json::to_string_pretty(&grouped)?;
        println!("{json}");
        return Ok(());
    }

    if cmd.concise {
        let concise = ConciseContextOutput::from_response(&response);
        println!("{}", serde_json::to_string(&concise)?);
        return Ok(());
    }

    let json = serde_json::to_string_pretty(&response)?;
    println!("{json}");
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct ContextSelection {
    concise: bool,
    depth: usize,
    max_lines: usize,
    include_callers: bool,
    include_callees: bool,
    include_types: bool,
    include_imports: bool,
    include_tests: bool,
    include_body: bool,
    include_signatures: bool,
    include_explain: bool,
}

impl ContextSelection {
    fn from_command(cmd: &ContextCommand) -> Self {
        let mut selection = if let Some(preset) = cmd.preset {
            match preset {
                ContextPreset::Refactor => Self {
                    concise: cmd.concise,
                    depth: 2,
                    max_lines: 300,
                    include_callers: true,
                    include_callees: true,
                    include_types: true,
                    include_imports: false,
                    include_tests: false,
                    include_body: false,
                    include_signatures: true,
                    include_explain: false,
                },
                ContextPreset::Debug => Self {
                    concise: cmd.concise,
                    depth: 2,
                    max_lines: 200,
                    include_callers: true,
                    include_callees: false,
                    include_types: false,
                    include_imports: false,
                    include_tests: true,
                    include_body: true,
                    include_signatures: true,
                    include_explain: false,
                },
                ContextPreset::Understand => Self {
                    concise: cmd.concise,
                    depth: 2,
                    max_lines: 300,
                    include_callers: true,
                    include_callees: true,
                    include_types: true,
                    include_imports: true,
                    include_tests: true,
                    include_body: false,
                    include_signatures: true,
                    include_explain: false,
                },
            }
        } else {
            Self {
                concise: cmd.concise,
                depth: 1,
                max_lines: 300,
                include_callers: false,
                include_callees: false,
                include_types: false,
                include_imports: false,
                include_tests: false,
                include_body: false,
                include_signatures: true,
                include_explain: false,
            }
        };

        if let Some(depth) = cmd.depth {
            selection.depth = usize::max(1, depth);
        }
        if let Some(max_lines) = cmd.max_lines {
            selection.max_lines = usize::max(1, max_lines);
        }
        if let Some(value) = cmd.callers {
            selection.include_callers = value;
        }
        if let Some(value) = cmd.callees {
            selection.include_callees = value;
        }
        if let Some(value) = cmd.types {
            selection.include_types = value;
        }
        if let Some(value) = cmd.imports {
            selection.include_imports = value;
        }
        if let Some(value) = cmd.tests {
            selection.include_tests = value;
        }
        if let Some(value) = cmd.include_body {
            selection.include_body = value;
        }

        // Signatures needed for concise output (def field uses signature)
        selection.include_signatures = cmd.signatures || cmd.concise;
        selection.include_explain = cmd.explain && !cmd.concise;
        selection
    }
}

#[derive(Serialize)]
struct ConciseContextOutput {
    symbol: String,
    resolution: SymbolResolution,
    analysis: BTreeMap<String, String>,
    truncated: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    def: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    def_location: Option<String>,
    callers: Vec<String>,
    callees: Vec<String>,
    tests: Vec<String>,
}

#[derive(Serialize)]
struct GroupedContextOutput {
    symbol: String,
    resolution: SymbolResolution,
    analysis: BTreeMap<String, String>,
    grouped: GroupedContextBuckets,
    summary: GroupedContextSummary,
    truncated: bool,
}

#[derive(Serialize)]
struct GroupedContextBuckets {
    #[serde(skip_serializing_if = "Option::is_none")]
    definition: Option<GroupedContextDefinition>,
    callers_by_file: BTreeMap<String, Vec<usize>>,
    callees: Vec<String>,
    test_coverage: Vec<String>,
}

#[derive(Serialize)]
struct GroupedContextDefinition {
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
}

#[derive(Serialize)]
struct GroupedContextSummary {
    definition: usize,
    callers: usize,
    callees: usize,
    tests: usize,
}

impl GroupedContextOutput {
    fn from_response(response: &ContextResponse, budget: Option<usize>) -> Self {
        let definition = response.definition.as_ref().and_then(|definition| {
            let file = match (definition.file.as_deref(), definition.line) {
                (Some(file), Some(line)) => format!("{file}:{line}"),
                (Some(file), None) => file.to_string(),
                _ => return None,
            };
            Some(GroupedContextDefinition {
                file,
                signature: definition.signature.as_deref().map(normalize_line_snippet),
            })
        });

        let mut callers_by_file: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for caller in &response.callers {
            let Some(file) = caller.file.as_deref() else {
                continue;
            };
            let Some(line) = caller.line else { continue };
            callers_by_file
                .entry(file.to_string())
                .or_default()
                .push(line);
        }
        for lines in callers_by_file.values_mut() {
            lines.sort();
            lines.dedup();
        }

        let mut callees: Vec<String> = response
            .callees
            .iter()
            .map(|callee| callee.symbol.clone())
            .collect();
        callees.sort();
        callees.dedup();

        let mut test_coverage: Vec<String> = response
            .tests
            .iter()
            .map(|test| test.path.clone())
            .collect();
        test_coverage.sort();
        test_coverage.dedup();

        let summary = GroupedContextSummary {
            definition: usize::from(definition.is_some()),
            callers: callers_by_file.values().map(|lines| lines.len()).sum(),
            callees: callees.len(),
            tests: test_coverage.len(),
        };

        let mut grouped = GroupedContextBuckets {
            definition,
            callers_by_file,
            callees,
            test_coverage,
        };

        let mut truncated = response.truncated;
        if let Some(budget_tokens) = budget {
            truncated |= apply_grouped_context_budget(&mut grouped, budget_tokens);
        }
        let mut analysis = response.analysis.clone();
        if truncated {
            for status in analysis.values_mut() {
                if status == "complete" {
                    *status = "incomplete".into();
                }
            }
        }

        Self {
            symbol: response.symbol.clone(),
            resolution: response.resolution.clone(),
            analysis,
            grouped,
            summary,
            truncated,
        }
    }
}

fn apply_grouped_context_budget(grouped: &mut GroupedContextBuckets, budget_tokens: usize) -> bool {
    if budget_tokens == 0 {
        grouped.callers_by_file.clear();
        grouped.callees.clear();
        grouped.test_coverage.clear();
        return true;
    }

    let original = serde_json::to_vec(grouped).unwrap_or_default();
    grouped.test_coverage.truncate(20);
    grouped.callees.truncate(50);

    let mut per_file_limit = 50usize;
    for _ in 0..6 {
        let bytes = serde_json::to_vec(grouped).map(|v| v.len()).unwrap_or(0);
        let tokens = usize::max(1, bytes / 4);
        if tokens <= budget_tokens {
            return serde_json::to_vec(grouped).unwrap_or_default() != original;
        }

        if per_file_limit > 5 {
            per_file_limit = usize::max(5, per_file_limit / 2);
            for lines in grouped.callers_by_file.values_mut() {
                if lines.len() > per_file_limit {
                    lines.truncate(per_file_limit);
                }
            }
            continue;
        }

        if grouped.callers_by_file.len() > 10 {
            grouped.callers_by_file = grouped
                .callers_by_file
                .iter()
                .take(10)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            continue;
        }

        return true;
    }

    true
}

impl ConciseContextOutput {
    fn from_response(response: &ContextResponse) -> Self {
        let def_location =
            response
                .definition
                .as_ref()
                .and_then(|d| match (d.file.as_deref(), d.line) {
                    (Some(file), Some(line)) => Some(format!("{file}:{line}")),
                    (Some(file), None) => Some(file.to_string()),
                    _ => None,
                });
        let def = response
            .definition
            .as_ref()
            .and_then(|d| d.signature.clone())
            .or_else(|| def_location.clone());
        let callers = response
            .callers
            .iter()
            .filter_map(|c| match (c.file.as_deref(), c.line) {
                (Some(file), Some(line)) => Some(format!("{file}:{line}")),
                _ => None,
            })
            .collect();
        let mut callees: Vec<String> = response.callees.iter().map(|c| c.symbol.clone()).collect();
        callees.sort();
        callees.dedup();
        let mut tests: Vec<String> = response.tests.iter().map(|t| t.path.clone()).collect();
        tests.sort();
        tests.dedup();

        Self {
            symbol: response.symbol.clone(),
            resolution: response.resolution.clone(),
            analysis: response.analysis.clone(),
            truncated: response.truncated,
            notes: response.notes.clone(),
            def: def.as_deref().map(normalize_line_snippet),
            def_location: def_location.as_deref().map(normalize_line_snippet),
            callers,
            callees,
            tests,
        }
    }
}

fn resolve_context_root(path: Option<&PathBuf>) -> Result<PathBuf, AppError> {
    let base = path
        .cloned()
        .unwrap_or_else(|| PathBuf::from("."))
        .canonicalize()
        .unwrap_or_else(|_| path.cloned().unwrap_or_else(|| PathBuf::from(".")));
    let root = if base.is_file() {
        base.parent().unwrap_or(&base).to_path_buf()
    } else {
        base
    };
    Ok(root)
}

fn stream_context(response: &ContextResponse) -> Result<(), AppError> {
    emit_json_line(&serde_json::json!({"type":"resolution","data":response.resolution}))?;
    emit_json_line(&serde_json::json!({"type":"analysis","data":response.analysis}))?;
    emit_json_line(&serde_json::json!({"type":"definition","data":response.definition}))?;
    emit_json_line(&serde_json::json!({"type":"callers","data":response.callers}))?;
    emit_json_line(&serde_json::json!({"type":"callees","data":response.callees}))?;
    emit_json_line(&serde_json::json!({"type":"types","data":response.types}))?;
    emit_json_line(&serde_json::json!({"type":"imports","data":response.imports}))?;
    emit_json_line(&serde_json::json!({"type":"tests","data":response.tests}))?;
    emit_json_line(
        &serde_json::json!({"type":"summary","data":{"summary":response.summary,"truncated":response.truncated,"notes":response.notes}}),
    )?;
    Ok(())
}

fn assemble_context_from_facts(
    root: &Path,
    symbol: &str,
    selection: &ContextSelection,
    files: &[FileFacts],
) -> Result<ContextResponse, AppError> {
    let mut response = ContextResponse::empty(symbol.to_string());
    response.definition = assemble_definition_from_facts(root, symbol, selection, files);
    if selection.include_callers || selection.include_callees {
        let graph = if selection.include_signatures && !selection.concise {
            CallGraph::from_facts_enriched(root, files, true, true)
        } else {
            CallGraph::from_facts_without_signatures(files)
        };
        if selection.include_callers {
            let (callers, truncated) = assemble_callers_from_facts(&graph, symbol, selection);
            response.callers = callers;
            response.truncated |= truncated;
        }
        if selection.include_callees {
            let (callees, truncated) = assemble_callees_from_facts(&graph, symbol, selection);
            response.callees = callees;
            response.truncated |= truncated;
        }
    }
    if selection.include_types {
        response.types = assemble_types_from_facts(symbol, files);
    }
    if selection.include_imports {
        response.imports = ImportContext::default();
    }
    if selection.include_tests {
        response.tests = assemble_tests_from_facts(root, symbol, files);
        if response.tests.len() > 50 {
            response.tests.truncate(50);
            response.truncated = true;
        }
    }

    enforce_context_budget(&mut response, selection.max_lines);
    response.summary = ContextSummary {
        callers: response.callers.len(),
        callees: response.callees.len(),
        tests: response.tests.len(),
    };
    Ok(response)
}

fn assemble_definition_from_facts(
    root: &Path,
    symbol: &str,
    selection: &ContextSelection,
    files: &[FileFacts],
) -> Option<Definition> {
    let target = normalize_symbol(symbol);
    let mut best: Option<(i32, &FileFacts, &swe_grep::analysis::DefinitionFact)> = None;

    for file in files {
        for def in &file.facts.definitions {
            let score = if def.symbol == symbol {
                3
            } else if def.symbol.ends_with(&format!("::{symbol}")) {
                2
            } else if normalize_symbol(&def.symbol) == target {
                1
            } else {
                0
            };
            if score == 0 {
                continue;
            }
            match best {
                None => best = Some((score, file, def)),
                Some((best_score, _, _)) if score > best_score => best = Some((score, file, def)),
                _ => {}
            }
        }
    }

    let (_score, file, def) = best?;
    let abs_path = root.join(&file.file);
    let contents = std::fs::read_to_string(&abs_path).ok()?;
    let lines: Vec<&str> = contents.lines().collect();
    let idx = def.line.saturating_sub(1);
    let include_signature = selection.include_signatures || selection.include_explain;
    let signature = if include_signature {
        def.signature
            .as_ref()
            .map(|s| normalize_line_snippet(s))
            .or_else(|| lines.get(idx).map(|s| normalize_line_snippet(s)))
    } else {
        None
    };

    let docstring = if selection.include_explain {
        None
    } else {
        extract_leading_docstring(&lines, idx)
    };
    let body = if selection.include_body {
        extract_definition_body(&lines, def.line, def.span, selection.max_lines)
    } else {
        None
    };

    let mut definition = Definition {
        file: Some(file.file.clone()),
        line: Some(def.line),
        kind: if selection.include_explain {
            Some(def.kind.clone())
        } else {
            None
        },
        visibility: if selection.include_explain {
            def.visibility.clone()
        } else {
            None
        },
        signature,
        docstring,
        doc: if selection.include_explain {
            def.doc
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        } else {
            None
        },
        fields: Vec::new(),
        methods: Vec::new(),
        implements: Vec::new(),
        complexity: None,
        body,
    };

    if selection.include_explain {
        enrich_definition_explain(&mut definition, symbol, &target, files, def.span);
    }

    Some(definition)
}

fn enrich_definition_explain(
    definition: &mut Definition,
    symbol: &str,
    target: &str,
    files: &[FileFacts],
    def_span: Option<(usize, usize)>,
) {
    let mut fields_by_line: Vec<(usize, String)> = Vec::new();
    let mut methods_by_line: Vec<(usize, String, Option<(usize, usize)>)> = Vec::new();

    for file in files {
        for field in &file.facts.fields {
            if !symbol_matches(&field.owner, symbol, target) {
                continue;
            }
            fields_by_line.push((field.line, format_field_fact(field)));
        }
        for method in &file.facts.methods {
            if !symbol_matches(&method.owner, symbol, target) {
                continue;
            }
            let display = summarize_method_fact(method);
            methods_by_line.push((method.line, display, method.span));
        }
    }

    fields_by_line.sort_by_key(|(line, _)| *line);
    methods_by_line.sort_by_key(|(line, _, _)| *line);

    definition.fields = fields_by_line.into_iter().map(|(_, s)| s).collect();
    definition.methods = methods_by_line.iter().map(|(_, s, _)| s.clone()).collect();

    let mut implements = std::collections::BTreeSet::new();
    for file in files {
        for rel in &file.facts.relations {
            if !symbol_matches(&rel.from, symbol, target) {
                continue;
            }
            if rel.kind != "impl_trait" && rel.kind != "implements" {
                continue;
            }
            implements.insert(rel.to.clone());
        }
    }
    definition.implements = implements.into_iter().collect();

    let mut loc = def_span
        .map(|(s, e)| e.saturating_sub(s).saturating_add(1))
        .unwrap_or(0);
    for (_, _, span) in methods_by_line {
        if let Some((s, e)) = span {
            loc = loc.saturating_add(e.saturating_sub(s).saturating_add(1));
        }
    }
    definition.complexity = Some(SymbolComplexity {
        fields: definition.fields.len(),
        methods: definition.methods.len(),
        loc,
    });
}

fn symbol_matches(candidate: &str, symbol: &str, target: &str) -> bool {
    candidate == symbol
        || candidate.ends_with(&format!("::{symbol}"))
        || normalize_symbol(candidate) == target
}

fn format_field_fact(field: &swe_grep::analysis::FieldFact) -> String {
    if let Some(ty) = &field.type_repr {
        let ty = ty.trim();
        if !ty.is_empty() {
            return format!("{}: {}", field.name.trim(), ty);
        }
    }
    field.name.trim().to_string()
}

fn summarize_method_fact(method: &swe_grep::analysis::MethodFact) -> String {
    summarize_method_name_and_params(method.name.as_str(), method.signature.as_deref())
}

fn summarize_method_name_and_params(name: &str, signature: Option<&str>) -> String {
    let Some(sig) = signature else {
        return format!("{name}()");
    };
    let Some(name_pos) = sig.find(name) else {
        return format!("{name}()");
    };
    let after_name = &sig[name_pos + name.len()..];
    let Some(open_rel) = after_name.find('(') else {
        return format!("{name}()");
    };
    let after_open = &after_name[open_rel + 1..];
    let Some(close_rel) = after_open.find(')') else {
        return format!("{name}()");
    };
    let params_raw = &after_open[..close_rel];
    let params = params_raw
        .split(',')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(simplify_param_name)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>();
    if params.is_empty() {
        format!("{name}()")
    } else {
        format!("{name}({})", params.join(", "))
    }
}

fn simplify_param_name(param: &str) -> String {
    let mut p = param.trim();
    if p.is_empty() {
        return String::new();
    }
    if p.contains("self") && (p.starts_with('&') || p.starts_with("self") || p.contains(" self")) {
        return "self".to_string();
    }

    if let Some((before, _)) = p.split_once(':') {
        p = before.trim();
    }
    if let Some((before, _)) = p.split_once('=') {
        p = before.trim();
    }
    p = p.trim_start_matches("...");
    p = p.trim_end_matches('?');

    let p = p
        .trim_start_matches("mut ")
        .trim_start_matches("ref ")
        .trim();
    p.split_whitespace().last().unwrap_or("").trim().to_string()
}

fn extract_leading_docstring(lines: &[&str], def_line_idx: usize) -> Option<String> {
    if def_line_idx == 0 || lines.is_empty() {
        return None;
    }

    let mut collected: Vec<String> = Vec::new();
    let mut idx = def_line_idx.saturating_sub(1);
    let mut budget = 10usize;
    while budget > 0 {
        let trimmed = lines.get(idx).copied().unwrap_or_default().trim();
        if trimmed.is_empty() {
            break;
        }
        let is_comment = trimmed.starts_with("///")
            || trimmed.starts_with("//!")
            || trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*');
        if !is_comment {
            break;
        }
        collected.push(trimmed.to_string());
        budget -= 1;
        if idx == 0 {
            break;
        }
        idx -= 1;
    }
    if collected.is_empty() {
        return None;
    }
    collected.reverse();
    Some(collected.join("\n"))
}

fn extract_definition_body(
    lines: &[&str],
    start_line: usize,
    span: Option<(usize, usize)>,
    max_lines: usize,
) -> Option<String> {
    let max_lines = usize::max(1, max_lines);
    let (start, end) = span.unwrap_or((start_line, start_line.saturating_add(40)));
    if start == 0 || lines.is_empty() {
        return None;
    }
    let start_idx = start.saturating_sub(1);
    let end_idx = usize::min(lines.len().saturating_sub(1), end.saturating_sub(1));
    if start_idx > end_idx {
        return None;
    }
    let slice = &lines[start_idx..=end_idx];
    let mut body_lines: Vec<&str> = slice.iter().copied().collect();
    if body_lines.len() > max_lines {
        body_lines.truncate(max_lines);
    }
    Some(body_lines.join("\n"))
}

fn assemble_callers_from_facts(
    graph: &CallGraph,
    symbol: &str,
    selection: &ContextSelection,
) -> (Vec<Caller>, bool) {
    let view = graph.callers_view(
        symbol,
        selection.depth,
        selection.max_lines.saturating_mul(2),
    );
    let nodes = flatten_call_tree(&view.nodes)
        .into_iter()
        .map(|node| Caller {
            symbol: node.symbol,
            signature: if selection.include_signatures {
                node.signature
            } else {
                None
            },
            call_context: if selection.include_signatures {
                node.call_context
            } else {
                None
            },
            file: node.file,
            line: node.line,
        })
        .collect();
    (nodes, view.truncated)
}

fn assemble_callees_from_facts(
    graph: &CallGraph,
    symbol: &str,
    selection: &ContextSelection,
) -> (Vec<Callee>, bool) {
    let view = graph.callees_view(
        symbol,
        selection.depth,
        selection.max_lines.saturating_mul(2),
    );
    let nodes = flatten_call_tree(&view.nodes)
        .into_iter()
        .map(|node| Callee {
            symbol: node.symbol,
            signature: if selection.include_signatures {
                node.signature
            } else {
                None
            },
            call_context: if selection.include_signatures {
                node.call_context
            } else {
                None
            },
            file: node.file,
            line: node.line,
        })
        .collect();
    (nodes, view.truncated)
}

fn flatten_call_tree(nodes: &[CallTreeNode]) -> Vec<CallTreeNode> {
    let mut out: Vec<CallTreeNode> = Vec::new();
    let mut seen: HashSet<(String, Option<String>, Option<usize>)> = HashSet::new();

    fn visit(
        node: &CallTreeNode,
        out: &mut Vec<CallTreeNode>,
        seen: &mut HashSet<(String, Option<String>, Option<usize>)>,
    ) {
        let key = (node.symbol.clone(), node.file.clone(), node.line);
        if seen.insert(key) {
            out.push(node.clone());
        }
        for child in &node.children {
            visit(child, out, seen);
        }
    }

    for node in nodes {
        visit(node, &mut out, &mut seen);
    }

    out.sort_by(|a, b| (&a.file, a.line, &a.symbol).cmp(&(&b.file, b.line, &b.symbol)));
    out
}

fn assemble_types_from_facts(symbol: &str, files: &[FileFacts]) -> TypeContext {
    let target = normalize_symbol(symbol);
    let mut implements = HashSet::<String>::new();
    let mut extended_by = HashSet::<String>::new();
    let mut contains = HashSet::<String>::new();

    for file in files {
        for rel in &file.facts.relations {
            let from = normalize_symbol(&rel.from);
            let to = normalize_symbol(&rel.to);
            let kind = rel.kind.as_str();

            if from == target && kind == "implements" {
                implements.insert(rel.to.clone());
            }
            if to == target && (kind == "extends" || kind == "implements") {
                extended_by.insert(rel.from.clone());
            }
            if from == target && kind == "contains" {
                contains.insert(rel.to.clone());
            }
        }
    }

    let mut implements = implements.into_iter().collect::<Vec<_>>();
    let mut extended_by = extended_by.into_iter().collect::<Vec<_>>();
    let mut contains = contains.into_iter().collect::<Vec<_>>();
    implements.sort();
    extended_by.sort();
    contains.sort();
    TypeContext {
        implements,
        extended_by,
        contains,
    }
}

fn assemble_tests_from_facts(root: &Path, symbol: &str, files: &[FileFacts]) -> Vec<TestFile> {
    let target = normalize_symbol(symbol);
    let mut seen: HashSet<String> = HashSet::new();
    let mut results: Vec<TestFile> = Vec::new();

    for file in files {
        if !file_has_reference(file, &target) {
            continue;
        }
        let looks_like_test = looks_like_test_path(&file.file)
            || file_contains_test_markers(root, &file.file).unwrap_or(false);
        if !looks_like_test {
            continue;
        }
        if seen.insert(file.file.clone()) {
            results.push(TestFile {
                path: file.file.clone(),
                test_names: Vec::new(),
            });
        }
    }

    results.sort_by(|a, b| a.path.cmp(&b.path));
    results
}

fn file_has_reference(file: &FileFacts, target: &str) -> bool {
    file.facts
        .references
        .iter()
        .any(|r| normalize_symbol(&r.symbol) == target)
}

fn looks_like_test_path(path: &str) -> bool {
    let lowered = path.to_ascii_lowercase();
    lowered.contains("/tests/")
        || lowered.contains("\\tests\\")
        || lowered.ends_with("_test.rs")
        || lowered.ends_with("_test.ts")
        || lowered.ends_with("_test.tsx")
        || lowered.ends_with("_test.js")
        || lowered.ends_with("_test.jsx")
        || lowered.ends_with(".spec.ts")
        || lowered.ends_with(".spec.tsx")
        || lowered.ends_with(".spec.js")
        || lowered.ends_with(".spec.jsx")
        || lowered.contains("test")
}

fn file_contains_test_markers(root: &Path, rel_path: &str) -> Option<bool> {
    let abs_path = root.join(rel_path);
    let contents = std::fs::read_to_string(&abs_path).ok()?;
    let lowered = contents.to_ascii_lowercase();
    Some(
        lowered.contains("#[test]")
            || lowered.contains("cfg(test)")
            || lowered.contains("mod tests")
            || lowered.contains("xctestcase")
            || lowered.contains("describe(")
            || lowered.contains("it(")
            || lowered.contains("test("),
    )
}

fn enforce_context_budget(response: &mut ContextResponse, max_lines: usize) {
    let max_lines = usize::max(1, max_lines);
    let mut truncated = false;

    if let Some(def) = response.definition.as_mut() {
        if let Some(body) = def.body.as_mut() {
            let mut lines: Vec<&str> = body.lines().collect();
            let cap = usize::max(1, max_lines / 2);
            if lines.len() > cap {
                lines.truncate(cap);
                *body = lines.join("\n");
                truncated = true;
            }
        }
    }

    let cap_list = usize::max(1, max_lines / 3);
    if response.callers.len() > cap_list {
        response.callers.truncate(cap_list);
        truncated = true;
    }
    if response.callees.len() > cap_list {
        response.callees.truncate(cap_list);
        truncated = true;
    }
    if response.tests.len() > cap_list {
        response.tests.truncate(cap_list);
        truncated = true;
    }

    response.truncated |= truncated;
}

#[derive(Serialize)]
struct CallerRef {
    file: String,
    line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_info: Option<search::GitInfo>,
}

#[derive(Serialize)]
struct CalleeRef {
    symbol: String,
    file: String,
    line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_info: Option<search::GitInfo>,
}

#[derive(Serialize, Clone)]
struct ImportRef {
    file: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    symbols: Vec<String>,
    line: usize,
}

#[derive(Serialize)]
struct RefsOutput {
    symbol: String,
    resolution: SymbolResolution,
    analysis: BTreeMap<String, String>,
    callers: Vec<CallerRef>,
    callees: Vec<CalleeRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    imports: Vec<ImportRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    importers: Vec<ImportRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
}

async fn handle_refs(mut cmd: RefsCommand, _disable_telemetry: bool) -> Result<(), AppError> {
    // Silence tracing by default for refs (outputs structured JSON)
    telemetry::silence_tracing();

    let callers_requested = cmd.callers || (!cmd.callers && !cmd.callees);
    let callees_requested = cmd.callees;
    let imports_requested = cmd.imports;
    let importers_requested = cmd.importers;

    apply_positional_query_to_search_args(
        &mut cmd.search,
        cmd.positional.symbol_pos.as_deref(),
        cmd.positional.lang_or_path_pos.as_deref(),
    )?;

    if cmd.search.definitions_only || cmd.search.kind.is_some() {
        return Err(AppError::Usage(
            "refs reports usage edges; --definitions-only and --kind filter declaration sites. Use search or context for declarations.".to_string(),
        ));
    }

    let mut args = cmd.search.clone();
    if args.context_before == 0 && args.context_after == 0 {
        args.context_before = 2;
        args.context_after = 2;
    }
    if !args.body {
        args.body = true;
    }

    let (summary, scope) = search::execute_with_scope(args.clone()).await?;

    let mut caller_hits: Vec<CallerRef> = Vec::new();
    let mut callee_hits: Vec<CalleeRef> = Vec::new();
    let mut import_hits: Vec<ImportRef> = Vec::new();
    let mut importer_hits: Vec<ImportRef> = Vec::new();
    let mut notes = summary.notes.clone();
    let mut warnings = summary.warnings.clone();

    // Keep the full repository parse context for resolution; scope only emitted edges.
    let root = scope.root();
    let cache_dir = search::default_cache_dir_for_root(root);
    let _cache_lease = swe_grep::cache_gc::CacheLease::acquire(root, &cache_dir)?;

    let target_file = resolve_target_file(root, cmd.target_file.as_deref())?;
    if target_file
        .as_ref()
        .is_some_and(|file| !scope.allows(Path::new(file)))
    {
        return Err(AppError::Usage(
            "target file is excluded by the requested scope".into(),
        ));
    }
    let mut resolution = SymbolResolution {
        status: "not_analyzed".into(),
        candidates: Vec::new(),
    };
    let mut selected_graph = None;
    let mut ambiguous_callers = false;
    let mut target_unresolved = false;
    if callers_requested || callees_requested {
        match extract_repo_facts(
            root,
            &cache_dir,
            &ExtractFactsOptions::for_search(None, None),
        ) {
            Ok((_, mut files)) => {
                resolution = resolve_symbol(
                    &files,
                    &summary.symbol,
                    target_file.as_deref(),
                    cmd.target_line,
                );
                if target_file.is_some() {
                    if resolution.status == "resolved" {
                        let target = &resolution.candidates[0];
                        ambiguous_callers = target_has_ambiguous_callers(&files, target);
                        select_symbol_facts_in_place(&mut files, target);
                        selected_graph = Some(CallGraph::from_facts(&files));
                        if ambiguous_callers && callers_requested {
                            warnings.push("incomplete refs: name-based analysis cannot attribute callers to duplicate declarations; ambiguous caller edges omitted".into());
                        }
                    } else {
                        target_unresolved = true;
                        warnings.push("incomplete refs: target declaration was not uniquely resolved; select a declaration file and start line".into());
                    }
                } else if resolution.status == "ambiguous" {
                    warnings.push("incomplete refs: multiple declarations match; relationships are merged by name. Select target-file and target-line to disambiguate callees".into());
                }
            }
            Err(err) => {
                warnings.push(format!(
                    "incomplete refs: declaration resolution unavailable: {err}"
                ));
                target_unresolved = target_file.is_some();
            }
        }
    }

    let call_graph_requested = callers_requested || callees_requested;
    if call_graph_requested {
        let options = ExtractFactsOptions::for_search(None, None);
        let graph_cache = GraphCache::new(&root, &cache_dir);
        match graph_cache.load_or_update(&options) {
            Ok((stats, graphs, graph_warnings)) => {
                if !graph_warnings.is_empty() {
                    warnings.push("incomplete refs: call graph analysis reported warnings; some edges may be omitted".to_string());
                }
                warnings.extend(graph_warnings);

                if callers_requested && !target_unresolved && !ambiguous_callers {
                    let caller_edges = selected_graph
                        .as_ref()
                        .map(|graph| graph.caller_edges_for(&summary.symbol))
                        .unwrap_or_else(|| graphs.call_graph.caller_edges_for(&summary.symbol));
                    for edge in caller_edges {
                        if !scope.allows(Path::new(&edge.file)) {
                            continue;
                        }
                        let abs_path = root.join(&edge.file);
                        let snippet = std::fs::read_to_string(&abs_path).ok().and_then(|content| {
                            content
                                .lines()
                                .nth(edge.line.saturating_sub(1))
                                .map(|s| s.trim().to_string())
                        });

                        caller_hits.push(CallerRef {
                            file: edge.file.clone(),
                            line: edge.line,
                            snippet,
                            origin: Some("callgraph".to_string()),
                            git_info: None,
                        });
                    }
                }

                if callees_requested && !target_unresolved {
                    let callee_edges = selected_graph
                        .as_ref()
                        .map(|graph| graph.callee_edges_for(&summary.symbol))
                        .unwrap_or_else(|| graphs.call_graph.callee_edges_for(&summary.symbol));
                    for edge in callee_edges {
                        if !scope.allows(Path::new(&edge.file)) {
                            continue;
                        }
                        if !cmd.include_constructors
                            && edge.file.ends_with(".rs")
                            && is_rust_constructor_noise(&edge.callee)
                        {
                            continue;
                        }

                        let abs_path = root.join(&edge.file);
                        let snippet = std::fs::read_to_string(&abs_path).ok().and_then(|content| {
                            content
                                .lines()
                                .nth(edge.line.saturating_sub(1))
                                .map(|s| s.trim().to_string())
                        });

                        callee_hits.push(CalleeRef {
                            symbol: edge.callee.clone(),
                            file: edge.file.clone(),
                            line: edge.line,
                            snippet,
                            git_info: None,
                        });
                    }
                }

                notes.push(format!(
                    "call graph: {} callers, {} callees (updated {} / scanned {})",
                    caller_hits.len(),
                    callee_hits.len(),
                    stats.updated_files,
                    stats.scanned_files
                ));
            }
            Err(err) => {
                warnings.push(format!(
                    "incomplete refs: call graph unavailable; call edges omitted: {err}"
                ));
            }
        }
    }

    if imports_requested || importers_requested {
        let options = ExtractFactsOptions::for_search(None, None);
        match extract_repo_facts(&root, &cache_dir, &options) {
            Ok((stats, file_facts)) => {
                let graph = ImportGraph::from_facts(&file_facts);
                let query = cmd.search.symbol.trim();
                let target_files = resolve_import_graph_targets(&graph, query);
                if target_files.is_empty() {
                    notes.push("import graph: no matching file/export found for query".to_string());
                } else if target_files.len() > 1 {
                    notes.push(format!(
                        "import graph: query matched {} files; results merged",
                        target_files.len()
                    ));
                }

                if imports_requested {
                    import_hits =
                        collect_import_edges(&graph, &target_files, cmd.import_depth, &scope);
                }
                if importers_requested {
                    importer_hits =
                        collect_importer_edges(&graph, &target_files, cmd.import_depth, &scope);
                }

                notes.push(format!(
                    "import graph: {} files processed (cache hit rate: {:.2})",
                    stats.processed_files,
                    stats.hit_rate()
                ));
            }
            Err(err) => {
                warnings.push(format!("import graph unavailable: {err}"));
            }
        }
    }

    // Dedup and sort
    caller_hits.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    caller_hits.dedup_by(|a, b| a.file == b.file && a.line == b.line);

    callee_hits.sort_by(|a, b| (&a.file, a.line, &a.symbol).cmp(&(&b.file, b.line, &b.symbol)));
    callee_hits.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.symbol == b.symbol);

    // An empty successful graph query is authoritative. Never guess calls from
    // whole-file text: unrelated function bodies and declarations are not edges.
    let callees = callee_hits;

    let mut analysis = BTreeMap::new();
    for (name, requested) in [
        ("callers", callers_requested),
        ("callees", callees_requested),
        ("imports", imports_requested),
        ("importers", importers_requested),
    ] {
        analysis.insert(
            name.to_string(),
            if !requested {
                "not_requested"
            } else if !warnings.is_empty() {
                "incomplete"
            } else {
                "complete"
            }
            .to_string(),
        );
    }
    let output = RefsOutput {
        symbol: summary.symbol.clone(),
        resolution,
        analysis,
        callers: caller_hits,
        callees,
        imports: import_hits,
        importers: importer_hits,
        warnings,
        notes,
    };

    match cmd.format {
        RefsFormat::Json => {
            let rendered = serde_json::to_string_pretty(&output)?;
            println!("{rendered}");
        }
        RefsFormat::Tree => {
            let rendered = render_refs_tree(&output);
            println!("{rendered}");
        }
    }

    Ok(())
}

fn is_rust_constructor_noise(raw: &str) -> bool {
    let name = raw
        .rsplit("::")
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_matches(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'));
    matches!(
        name,
        "Ok" | "Err" | "Some" | "None" | "Result" | "Option" | "Box" | "Vec" | "String"
    )
}

fn resolve_import_graph_targets(graph: &ImportGraph, query: &str) -> Vec<String> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }

    if graph.is_known_file(query) {
        return vec![query.trim_start_matches("./").to_string()];
    }

    graph
        .find_export(query)
        .into_iter()
        .map(|s| s.to_string())
        .collect()
}

fn collect_import_edges(
    graph: &ImportGraph,
    targets: &[String],
    depth: usize,
    scope: &search::SearchScope,
) -> Vec<ImportRef> {
    let depth = depth.max(1);
    let mut out: Vec<ImportRef> = Vec::new();
    let mut visited_files: HashSet<String> = HashSet::new();
    let mut seen_edges: HashSet<(String, String, usize)> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    for file in targets {
        if visited_files.insert(file.clone()) {
            queue.push_back((file.clone(), 0));
        }
    }

    while let Some((current, d)) = queue.pop_front() {
        if d >= depth {
            continue;
        }
        for edge in graph.imports_of(&current) {
            if !scope.allows(Path::new(&edge.importer))
                || (graph.is_known_file(&edge.imported) && !scope.allows(Path::new(&edge.imported)))
            {
                continue;
            }
            let key = (edge.importer.clone(), edge.imported.clone(), edge.line);
            if !seen_edges.insert(key) {
                continue;
            }
            out.push(ImportRef {
                file: edge.imported.clone(),
                symbols: edge.symbols.clone(),
                line: edge.line,
            });
            if graph.is_known_file(&edge.imported) && visited_files.insert(edge.imported.clone()) {
                queue.push_back((edge.imported.clone(), d + 1));
            }
        }
    }

    out.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    out
}

fn collect_importer_edges(
    graph: &ImportGraph,
    targets: &[String],
    depth: usize,
    scope: &search::SearchScope,
) -> Vec<ImportRef> {
    let depth = depth.max(1);
    let mut out: Vec<ImportRef> = Vec::new();
    let mut visited_files: HashSet<String> = HashSet::new();
    let mut seen_edges: HashSet<(String, String, usize)> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    for file in targets {
        if visited_files.insert(file.clone()) {
            queue.push_back((file.clone(), 0));
        }
    }

    while let Some((current, d)) = queue.pop_front() {
        if d >= depth {
            continue;
        }
        for edge in graph.importers_of(&current) {
            if !scope.allows(Path::new(&edge.importer))
                || (graph.is_known_file(&edge.imported) && !scope.allows(Path::new(&edge.imported)))
            {
                continue;
            }
            let key = (edge.importer.clone(), edge.imported.clone(), edge.line);
            if !seen_edges.insert(key) {
                continue;
            }
            out.push(ImportRef {
                file: edge.importer.clone(),
                symbols: edge.symbols.clone(),
                line: edge.line,
            });
            if graph.is_known_file(&edge.importer) && visited_files.insert(edge.importer.clone()) {
                queue.push_back((edge.importer.clone(), d + 1));
            }
        }
    }

    out.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    out
}

async fn handle_remove(cmd: RemoveCommand, disable_telemetry: bool) -> Result<(), AppError> {
    if !disable_telemetry {
        telemetry::init()?;
    }
    let mut cmd = cmd;
    apply_positional_query_to_search_args(
        &mut cmd.search,
        cmd.positional.symbol_pos.as_deref(),
        cmd.positional.lang_or_path_pos.as_deref(),
    )?;
    let summary = search::execute(cmd.search.clone()).await?;
    let plan = build_removal_plan(&summary);

    if cmd.apply && !cmd.dry_run {
        apply_removal_plan(&plan)?;
    }

    let rendered = render_removal_plan(&plan, cmd.apply && !cmd.dry_run);
    println!("{rendered}");
    Ok(())
}

fn collect_symbols(cmd: &SearchCommand) -> Result<Vec<String>, AppError> {
    let mut collected: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let push_symbol = |target: &mut Vec<String>, seen: &mut HashSet<String>, value: &str| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return;
        }
        if seen.insert(trimmed.to_string()) {
            target.push(trimmed.to_string());
        }
    };

    push_symbol(&mut collected, &mut seen, &cmd.search.symbol);

    if collected.is_empty() {
        if let Some(concept) = &cmd.search.concept {
            push_symbol(&mut collected, &mut seen, concept);
        }
    }

    if let Some(list) = &cmd.symbols {
        for symbol in list.split(',') {
            push_symbol(&mut collected, &mut seen, symbol);
        }
    }

    if let Some(path) = &cmd.symbols_file {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read symbols file {}", path.display()))?;
        for line in contents.lines() {
            push_symbol(&mut collected, &mut seen, line);
        }
    }

    if cmd.stdin_symbols {
        let mut buffer = String::new();
        io::stdin()
            .read_to_string(&mut buffer)
            .with_context(|| "failed to read symbols from stdin")?;
        for line in buffer.lines() {
            push_symbol(&mut collected, &mut seen, line);
        }
    }

    if collected.is_empty() {
        return Err(AppError::Usage(
            "at least one symbol is required (--symbol/--symbols/--symbols-file/--stdin-symbols)"
                .to_string(),
        ));
    }

    Ok(collected)
}

const AGENT_DEFAULT_TRIM_BODY: usize = 50;

fn apply_agent_defaults(
    args: &mut SearchArgs,
    trim_body: &mut Option<usize>,
    full_body: bool,
    body_head: Option<usize>,
    body_tail: Option<usize>,
) {
    if args.context_before == 0 && args.context_after == 0 {
        args.context_before = 2;
        args.context_after = 2;
    }
    if !args.body {
        args.body = true;
    }
    if full_body {
        *trim_body = None;
        args.no_truncate = true;
        args.body_limit = None;
        return;
    }
    if body_head.is_some() || body_tail.is_some() {
        *trim_body = None;
        return;
    }
    if trim_body.is_none() {
        *trim_body = Some(AGENT_DEFAULT_TRIM_BODY);
    }
}

fn resolve_root(args: &SearchArgs) -> Result<PathBuf, AppError> {
    let base = if let Some(path) = &args.path {
        path.clone()
    } else {
        std::env::current_dir().context("failed to resolve current directory")?
    };
    let canonical = base.canonicalize().unwrap_or(base);
    Ok(canonical)
}

fn emit_json_line<T: serde::Serialize>(value: &T) -> Result<(), AppError> {
    let line = serde_json::to_string(value)?;
    let mut stdout = io::stdout().lock();
    stdout.write_all(line.as_bytes())?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn build_open_commands_from_top_hits(
    hits: &[search::TopHit],
    template: Option<&str>,
) -> Vec<String> {
    build_open_commands(
        hits.iter().map(|hit| (hit.path.as_str(), hit.line)),
        template,
    )
}

fn build_open_commands_from_agent_hits(
    hits: &[swe_grep::agent::AgentHit],
    template: Option<&str>,
) -> Vec<String> {
    build_open_commands(
        hits.iter().map(|hit| (hit.file.as_str(), hit.line)),
        template,
    )
}

fn build_open_commands<'a, I>(hits: I, template: Option<&str>) -> Vec<String>
where
    I: IntoIterator<Item = (&'a str, usize)>,
{
    let template = open_command_template(template);
    let mut seen: HashSet<String> = HashSet::new();
    hits.into_iter()
        .filter_map(|(path, line)| {
            if path.is_empty() {
                return None;
            }
            let command = format_open_command(&template, path, line);
            if seen.insert(command.clone()) {
                Some(command)
            } else {
                None
            }
        })
        .collect()
}

fn open_command_template(template: Option<&str>) -> String {
    if let Some(custom) = template {
        return custom.to_string();
    }
    if let Ok(editor) = std::env::var("EDITOR") {
        let lowered = editor.to_ascii_lowercase();
        if lowered.contains("code") {
            return format!("{editor} -g {{path}}:{{line}}");
        }
        if lowered.contains("vim") || lowered.contains("nvim") {
            return format!("{editor} +{{line}} {{path}}");
        }
        return format!("{editor} {{path}}");
    }
    "code -g {path}:{line}".to_string()
}

fn format_open_command(template: &str, path: &str, line: usize) -> String {
    template
        .replace("{path}", path)
        .replace("{line}", &line.to_string())
}

fn render_usage_output(
    summary: &search::SearchSummary,
    format: UsageFormat,
) -> Result<String, AppError> {
    let settings = AgentSettings {
        include_definitions: true,
        trim_body: Some(8),
        body_head: None,
        body_tail: None,
        concise: true,
    };
    let agent_result = AgentResult::from_summary(&summary.symbol, summary, &settings);

    let rendered = match format {
        UsageFormat::Tree => render_usage_tree(summary, &agent_result),
        UsageFormat::Compact => render_compact(&agent_result.hits),
        UsageFormat::Json => serde_json::to_string_pretty(summary)?,
    };
    Ok(rendered)
}

fn render_usage_tree(summary: &search::SearchSummary, result: &AgentResult) -> String {
    let mut lines = Vec::new();
    lines.push(summary.symbol.clone());
    if let Some(first) = summary.top_hits.first() {
        lines.push(format!("({})", first.path));
    }

    if !result.hits.is_empty() {
        let mut grouped: BTreeMap<String, Vec<&swe_grep::agent::AgentHit>> = BTreeMap::new();
        for hit in &result.hits {
            grouped.entry(hit.file.clone()).or_default().push(hit);
        }
        let mut usage_lines = Vec::new();
        for (file, hits) in grouped {
            let refs = hits
                .iter()
                .take(3)
                .map(|hit| format!("{}:{}", file, hit.line))
                .collect::<Vec<_>>()
                .join(", ");
            usage_lines.push(refs);
        }
        if !usage_lines.is_empty() {
            lines.push(format!("|-- used by: {}", usage_lines.join("; ")));
        }
    }

    let types = collect_type_hints(summary);
    if !types.is_empty() {
        lines.push(format!("|-- types: {}", types.join(", ")));
    }

    let imports = collect_imports(summary);
    if !imports.is_empty() {
        lines.push(format!("\\-- imports: {}", imports.join(", ")));
    } else if !summary.suggested_symbols.is_empty() {
        lines.push(format!(
            "\\-- related symbols: {}",
            summary.suggested_symbols.join(", ")
        ));
    }

    lines.join("\n")
}

fn collect_type_hints(summary: &search::SearchSummary) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for hit in &summary.top_hits {
        for hint in &hit.hints {
            if matches!(
                hint.kind.as_str(),
                "declaration" | "module" | "type" | "impl"
            ) && seen.insert(hint.label.clone())
            {
                labels.push(hint.label.clone());
            }
        }
    }
    labels.truncate(5);
    labels
}

fn collect_imports(summary: &search::SearchSummary) -> Vec<String> {
    let mut imports: HashSet<String> = HashSet::new();
    for hit in &summary.top_hits {
        for snippet in [
            hit.snippet.as_deref(),
            hit.expanded_snippet.as_deref(),
            hit.raw_snippet.as_deref(),
        ]
        .iter()
        .flatten()
        {
            for line in snippet.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("import ")
                    || trimmed.starts_with("use ")
                    || trimmed.starts_with("from ")
                {
                    imports.insert(trimmed.to_string());
                }
            }
        }
    }
    let mut collected: Vec<String> = imports.into_iter().collect();
    collected.truncate(5);
    collected
}

fn normalize_line_snippet(value: &str) -> String {
    let collapsed = value
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.len() > 160 {
        format!("{}...", &collapsed[..157])
    } else {
        collapsed
    }
}

fn render_refs_tree(output: &RefsOutput) -> String {
    let mut lines = Vec::new();
    lines.push(output.symbol.clone());

    lines.push("|-- callers:".to_string());
    if output.callers.is_empty() {
        lines.push("|   <none>".to_string());
    } else {
        for caller in &output.callers {
            let preview = caller
                .snippet
                .as_deref()
                .unwrap_or("<no snippet>")
                .to_string();
            lines.push(format!("|   {}:{} {}", caller.file, caller.line, preview));
        }
    }

    lines.push("\\-- callees:".to_string());
    if output.callees.is_empty() {
        lines.push("    <none>".to_string());
    } else {
        for callee in &output.callees {
            let preview = callee
                .snippet
                .as_deref()
                .unwrap_or("<no snippet>")
                .to_string();
            lines.push(format!(
                "    {} -> {}:{} {}",
                output.symbol, callee.file, callee.line, preview
            ));
        }
    }

    if !output.imports.is_empty() {
        lines.push(String::new());
        lines.push("imports:".to_string());
        for imp in &output.imports {
            let symbols = if imp.symbols.is_empty() {
                "*".to_string()
            } else {
                imp.symbols.join(", ")
            };
            lines.push(format!("- {}:{} [{symbols}]", imp.file, imp.line));
        }
    }

    if !output.importers.is_empty() {
        lines.push(String::new());
        lines.push("importers:".to_string());
        for imp in &output.importers {
            let symbols = if imp.symbols.is_empty() {
                "*".to_string()
            } else {
                imp.symbols.join(", ")
            };
            lines.push(format!("- {}:{} [{symbols}]", imp.file, imp.line));
        }
    }

    if !output.warnings.is_empty() {
        lines.push(String::new());
        lines.push("warnings:".to_string());
        for warning in &output.warnings {
            lines.push(format!("- {warning}"));
        }
    }

    if !output.notes.is_empty() {
        lines.push(String::new());
        lines.push("notes:".to_string());
        for note in &output.notes {
            lines.push(format!("- {note}"));
        }
    }

    lines.join("\n")
}

struct RemovalPlan {
    symbol: String,
    references: BTreeMap<String, usize>,
    delete_candidates: Vec<String>,
    suggestions: Vec<String>,
}

fn build_removal_plan(summary: &search::SearchSummary) -> RemovalPlan {
    let mut references: BTreeMap<String, usize> = BTreeMap::new();
    for hit in &summary.top_hits {
        *references.entry(hit.path.clone()).or_insert(0) += 1;
    }

    let mut delete_candidates: Vec<String> = Vec::new();
    for path in references.keys() {
        let stem_matches = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|stem| stem.eq_ignore_ascii_case(&summary.symbol))
            .unwrap_or(false);
        if stem_matches {
            delete_candidates.push(path.clone());
        }
    }

    let suggestions = if summary.top_hits.is_empty() {
        summary.suggested_symbols.clone()
    } else {
        Vec::new()
    };

    RemovalPlan {
        symbol: summary.symbol.clone(),
        references,
        delete_candidates,
        suggestions,
    }
}

fn apply_removal_plan(plan: &RemovalPlan) -> Result<(), AppError> {
    for path in &plan.delete_candidates {
        let path_obj = PathBuf::from(path);
        if path_obj.is_file() {
            fs::remove_file(&path_obj)
                .with_context(|| format!("failed to delete {}", path_obj.display()))?;
        }
    }
    Ok(())
}

fn render_removal_plan(plan: &RemovalPlan, applied: bool) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Removal plan for {}", plan.symbol));
    if plan.references.is_empty() {
        lines.push("Files to modify: <none found>".to_string());
    } else {
        lines.push("Files to modify:".to_string());
        for (path, count) in &plan.references {
            lines.push(format!("  - {} ({} references)", path, count));
        }
    }

    if plan.delete_candidates.is_empty() {
        lines.push("Files to delete: <none suggested>".to_string());
    } else {
        lines.push("Files to delete:".to_string());
        for path in &plan.delete_candidates {
            lines.push(format!("  - {}", path));
        }
    }

    if !plan.suggestions.is_empty() {
        lines.push(format!("Related symbols: {}", plan.suggestions.join(", ")));
    }

    if applied {
        lines.push("Applied deletions for the listed files.".to_string());
    }

    lines.join("\n")
}

fn clear_cache_for_command(cli: &Cli) -> Result<(), AppError> {
    let search_root = |args: &SearchArgs,
                       positional: &swe_grep::cli::PositionalQueryArgs|
     -> Result<PathBuf, AppError> {
        let mut args = args.clone();
        apply_positional_query_to_search_args(
            &mut args,
            positional.symbol_pos.as_deref(),
            positional.lang_or_path_pos.as_deref(),
        )?;
        resolve_root(&args)
    };
    let root = match &cli.command {
        Some(Commands::Search(cmd)) => search_root(&cmd.search, &cmd.positional)?,
        Some(Commands::Usages(cmd)) => search_root(&cmd.search, &cmd.positional)?,
        Some(Commands::Refs(cmd)) => search_root(&cmd.search, &cmd.positional)?,
        Some(Commands::Context(cmd)) => {
            let base = if let Some(path) = cmd.path.as_ref().or(cmd.path_pos.as_ref()) {
                path.clone()
            } else {
                std::env::current_dir().context("failed to resolve current directory")?
            };
            base.canonicalize().unwrap_or(base)
        }
        Some(Commands::Remove(cmd)) => search_root(&cmd.search, &cmd.positional)?,
        Some(Commands::Feedback(_)) => {
            std::env::current_dir().context("failed to resolve current directory")?
        }
        Some(Commands::Cache(_)) => {
            return Err(AppError::Usage(
                "Use cache gc --apply to collect managed caches".into(),
            ));
        }
        Some(Commands::Serve(args)) => {
            if let Some(path) = &args.path {
                path.clone()
            } else {
                std::env::current_dir().context("failed to resolve current directory")?
            }
        }
        Some(Commands::Bench(args)) => {
            if let Some(path) = &args.cache_dir {
                path.clone()
            } else {
                std::env::current_dir().context("failed to resolve current directory")?
            }
        }
        None => {
            // Simple mode - use current directory
            std::env::current_dir().context("failed to resolve current directory")?
        }
    };
    let root = root.canonicalize().unwrap_or(root);
    let root = if root.is_file() {
        root.parent().unwrap_or(&root).to_path_buf()
    } else {
        root
    };
    clear_cache_dirs(&root)?;
    Ok(())
}

fn clear_cache_dirs(root: &Path) -> Result<(), AppError> {
    let report = swe_grep::cache_gc::purge(root)?;
    if report.collector_busy {
        return Err(AppError::Usage(
            "Cache collector is busy; retry --clear-cache after it finishes".into(),
        ));
    }
    // Active requests keep their shared lease; never invalidate data under them.
    if report.skipped_active > 0 {
        return Err(AppError::Usage(
            "Cache is currently in use; retry --clear-cache after active requests finish".into(),
        ));
    }
    if report.scan_limited {
        return Err(AppError::Usage(
            "Cache cleanup reached its bounded scan limit; repeat cache gc --apply before retrying"
                .into(),
        ));
    }
    Ok(())
}

fn telemetry_default_disabled() -> bool {
    if let Ok(value) = std::env::var("SWE_GREP_TELEMETRY") {
        let normalized = value.trim().to_ascii_lowercase();
        if matches!(
            normalized.as_str(),
            "0" | "false" | "off" | "no" | "disable" | "disabled"
        ) {
            return true;
        }
        if matches!(
            normalized.as_str(),
            "1" | "true" | "on" | "yes" | "enable" | "enabled"
        ) {
            return false;
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let path = PathBuf::from(home).join(".swe-grep.toml");
        if path.is_file() {
            if let Ok(contents) = fs::read_to_string(&path) {
                if let Ok(value) = contents.parse::<toml::Value>() {
                    if let Some(disable) = value.get("disable_telemetry").and_then(|v| v.as_bool())
                    {
                        return disable;
                    }
                }
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use swe_grep::cli::{PositionalQueryArgs, ToolArgs};

    fn search_args(symbol: &str) -> SearchArgs {
        SearchArgs {
            exact_symbol: false,
            symbol: symbol.to_string(),
            text: false,
            concept: None,
            concept_expand: false,
            regex: false,
            literal_regex: false,
            wildcard: false,
            attribute: false,
            string: false,
            verbose_ast: false,
            include_strings: false,
            code_only: false,
            ignore_case: false,
            fuzzy_threshold: 0.7,
            definitions_only: false,
            usages_only: false,
            mode: None,
            kind: None,
            path: None,
            scope_file: None,
            path_glob: Vec::new(),
            exclude_patterns: Vec::new(),
            exclude_tests: false,
            language: None,
            context: None,
            timeout_secs: 3,
            max_matches: 20,
            top_hits: 20,
            max_columns: 200,
            concurrency: 8,
            tools: ToolArgs {
                enable_index: false,
                enable_rga: false,
                use_fd: true,
                use_ast_grep: true,
            },
            context_before: 0,
            context_after: 0,
            body: false,
            body_limit: None,
            no_truncate: false,
            git_modified: false,
            git_since: None,
            git_info: false,
            tracked_only: false,
            index_dir: None,
            cache_dir: None,
            log_dir: None,
        }
    }

    #[test]
    fn dedupes_batch_symbols() {
        let cmd = SearchCommand {
            search: search_args("Alpha"),
            positional: PositionalQueryArgs::default(),
            json_only: false,
            agent: true,
            format: OutputFormat::Json,
            concise: false,
            grouped: false,
            budget: None,
            json_pretty: false,
            symbols: Some("Alpha,Beta,Alpha,Gamma".to_string()),
            symbols_file: None,
            stdin_symbols: false,
            stream: false,
            progress: false,
            quiet: false,
            trim_body: None,
            body_head: None,
            body_tail: None,
            full_body: false,
            definitions: false,
            open: false,
            open_command: None,
            assert_absent: false,
            files_only: false,
        };

        let symbols = collect_symbols(&cmd).expect("expected symbols to be collected");
        assert_eq!(
            symbols,
            vec!["Alpha".to_string(), "Beta".to_string(), "Gamma".to_string()]
        );
    }

    #[test]
    fn parses_simple_symbols_batch_by_comma() {
        let parsed = parse_simple_symbols("Foo, Bar, Foo", true);
        assert_eq!(parsed, vec!["Foo".to_string(), "Bar".to_string()]);
    }

    #[test]
    fn does_not_split_simple_symbols_when_disabled() {
        let parsed = parse_simple_symbols("foo,bar", false);
        assert_eq!(parsed, vec!["foo,bar".to_string()]);
    }

    #[test]
    fn suggests_flags_across_subcommands() {
        let suggestions = suggest_flags("--caller", 5);
        assert!(
            suggestions
                .iter()
                .any(|s| s.flag == "--callers" && s.command_path == vec!["refs".to_string()]),
            "expected --callers (refs) in {suggestions:?}"
        );
    }
}
