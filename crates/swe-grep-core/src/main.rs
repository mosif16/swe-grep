use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use anyhow::{Context, anyhow};
use clap::Parser;

use swe_grep::agent::{aggregate_output, render_table, stream_entry, AgentResult, AgentSettings};
use swe_grep::bench;
use swe_grep::cli::{Cli, Commands, OutputFormat, SearchArgs, SearchCommand};
use swe_grep::search;
use swe_grep::service;
use swe_grep::telemetry;

#[derive(Debug)]
enum AppError {
    Usage(String),
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
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let _ = err.print();
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
        Err(AppError::Runtime(err)) => {
            eprintln!("{err}");
            2
        }
    };

    std::process::exit(exit_code);
}

async fn run(cli: Cli) -> Result<(), AppError> {
    match cli.command {
        Commands::Search(cmd) => handle_search(cmd, cli.disable_telemetry).await,
        Commands::Bench(args) => {
            if !cli.disable_telemetry {
                telemetry::init()?;
            }
            bench::run(args).await?;
            Ok(())
        }
        Commands::Serve(args) => {
            if !cli.disable_telemetry {
                telemetry::init()?;
            }
            service::serve(args).await?;
            Ok(())
        }
    }
}

async fn handle_search(cmd: SearchCommand, disable_telemetry: bool) -> Result<(), AppError> {
    let symbols = collect_symbols(&cmd)?;

    if !cmd.agent && symbols.len() > 1 {
        return Err(AppError::Usage(
            "multiple symbols require --agent mode".to_string(),
        ));
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
    if cmd.json_pretty && matches!(cmd.format, OutputFormat::Table) {
        return Err(AppError::Usage(
            "--json-pretty requires --format json".to_string(),
        ));
    }
    if cmd.trim_body.is_some() && !cmd.agent {
        return Err(AppError::Usage(
            "--trim-body is only available in --agent mode".to_string(),
        ));
    }
    if cmd.stream && !cmd.agent {
        return Err(AppError::Usage(
            "--stream is only available in --agent mode".to_string(),
        ));
    }
    if cmd.stream && matches!(cmd.format, OutputFormat::Table) {
        return Err(AppError::Usage(
            "--format table cannot be combined with --stream".to_string(),
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
            cmd.definitions,
            cmd.trim_body,
            cmd.concise,
            cmd.json_pretty,
            cmd.quiet,
            cmd.progress,
        )
        .await
    } else {
        run_default_search(cmd.search, symbols).await
    }
}

async fn run_default_search(mut args: SearchArgs, symbols: Vec<String>) -> Result<(), AppError> {
    let symbol = symbols
        .first()
        .cloned()
        .ok_or_else(|| AppError::Usage("symbol is required".to_string()))?;
    args.symbol = symbol;

    let summary = search::execute(args).await?;
    let json = serde_json::to_string_pretty(&summary)?;
    println!("{json}");

    Ok(())
}

async fn run_agent_search(
    mut args: SearchArgs,
    symbols: Vec<String>,
    format: OutputFormat,
    stream: bool,
    definitions: bool,
    trim_body: Option<usize>,
    concise: bool,
    json_pretty: bool,
    quiet: bool,
    progress: bool,
) -> Result<(), AppError> {
    let mut trim_body = trim_body;
    apply_agent_defaults(&mut args, &mut trim_body);
    let settings = AgentSettings {
        include_definitions: definitions,
        trim_body,
        concise,
    };

    let mut results = Vec::new();
    let mut root_hint: Option<PathBuf> = None;
    let mut had_error = false;
    let total = symbols.len();

    for (idx, symbol) in symbols.iter().enumerate() {
        let mut per_symbol_args = args.clone();
        per_symbol_args.symbol = symbol.clone();

        if progress && !quiet {
            eprintln!("[{}/{}] Searching {}...", idx + 1, total, symbol);
        }

        match search::execute(per_symbol_args).await {
            Ok(summary) => {
                root_hint.get_or_insert(summary.root.clone());
                let result = AgentResult::from_summary(symbol, &summary, &settings);
                if stream {
                    emit_json_line(&stream_entry(&result, idx + 1 == symbols.len()))?;
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
                    emit_json_line(&stream_entry(&result, idx + 1 == symbols.len()))?;
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
        return Ok(());
    }

    let root = root_hint.unwrap_or(resolve_root(&args)?);
    let output = aggregate_output(&root, &symbols, &results);

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
    }

    if had_error {
        return Err(AppError::Runtime(anyhow!("one or more searches failed")));
    }

    Ok(())
}

fn collect_symbols(cmd: &SearchCommand) -> Result<Vec<String>, AppError> {
    let mut collected: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let mut push_symbol = |value: &str| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return;
        }
        if seen.insert(trimmed.to_string()) {
            collected.push(trimmed.to_string());
        }
    };

    push_symbol(&cmd.search.symbol);

    if let Some(list) = &cmd.symbols {
        for symbol in list.split(',') {
            push_symbol(symbol);
        }
    }

    if let Some(path) = &cmd.symbols_file {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read symbols file {}", path.display()))?;
        for line in contents.lines() {
            push_symbol(line);
        }
    }

    if cmd.stdin_symbols {
        let mut buffer = String::new();
        io::stdin()
            .read_to_string(&mut buffer)
            .with_context(|| "failed to read symbols from stdin")?;
        for line in buffer.lines() {
            push_symbol(line);
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

fn apply_agent_defaults(args: &mut SearchArgs, trim_body: &mut Option<usize>) {
    if args.context_before == 0 && args.context_after == 0 {
        args.context_before = 2;
        args.context_after = 2;
    }
    if !args.body {
        args.body = true;
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

#[cfg(test)]
mod tests {
    use super::*;
    use swe_grep::cli::ToolArgs;

    fn search_args(symbol: &str) -> SearchArgs {
        SearchArgs {
            symbol: symbol.to_string(),
            path: None,
            language: None,
            timeout_secs: 3,
            max_matches: 20,
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
            index_dir: None,
            cache_dir: None,
            log_dir: None,
        }
    }

    #[test]
    fn dedupes_batch_symbols() {
        let cmd = SearchCommand {
            search: search_args("Alpha"),
            agent: true,
            format: OutputFormat::Json,
            concise: false,
            json_pretty: false,
            symbols: Some("Alpha,Beta,Alpha,Gamma".to_string()),
            symbols_file: None,
            stdin_symbols: false,
            stream: false,
            progress: false,
            quiet: false,
            trim_body: None,
            definitions: false,
        };

        let symbols = collect_symbols(&cmd).expect("expected symbols to be collected");
        assert_eq!(
            symbols,
            vec!["Alpha".to_string(), "Beta".to_string(), "Gamma".to_string()]
        );
    }
}
