use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use clap::Parser;
use regex::Regex;
use serde::Serialize;

use swe_grep::agent::{
    AgentResult, AgentSettings, aggregate_output, render_compact, render_edit, render_minimal,
    render_table, render_tree_grouped, stream_entry, summarize_hits,
};
use swe_grep::bench;
use swe_grep::cli::{
    Cli, Commands, OutputFormat, RefsCommand, RefsFormat, RemoveCommand, SearchArgs, SearchCommand,
    UsageCommand, UsageFormat,
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

async fn run(cli: Cli) -> Result<(), AppError> {
    let disable_telemetry = cli.disable_telemetry || telemetry_default_disabled();
    if cli.clear_cache {
        clear_cache_for_command(&cli)?;
    }
    match cli.command {
        Commands::Search(cmd) => handle_search(cmd, disable_telemetry).await,
        Commands::Usages(cmd) => handle_usages(cmd, disable_telemetry).await,
        Commands::Refs(cmd) => handle_refs(cmd, disable_telemetry).await,
        Commands::Remove(cmd) => handle_remove(cmd, disable_telemetry).await,
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

async fn handle_search(cmd: SearchCommand, disable_telemetry: bool) -> Result<(), AppError> {
    let symbols = collect_symbols(&cmd)?;

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
        if assert_absent && found_hits {
            return Err(AppError::AssertionFailed(
                "symbol found while --assert-absent is enabled".to_string(),
            ));
        }
        return Ok(());
    }

    let root = root_hint.unwrap_or(resolve_root(&args)?);
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

#[derive(Serialize)]
struct RefsOutput {
    symbol: String,
    callers: Vec<CallerRef>,
    callees: Vec<CalleeRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
}

async fn handle_refs(cmd: RefsCommand, disable_telemetry: bool) -> Result<(), AppError> {
    if !disable_telemetry {
        telemetry::init()?;
    }

    let callers_requested = cmd.callers || (!cmd.callers && !cmd.callees);
    let callees_requested = cmd.callees;

    let mut args = cmd.search.clone();
    if args.context_before == 0 && args.context_after == 0 {
        args.context_before = 2;
        args.context_after = 2;
    }
    if !args.body {
        args.body = true;
    }

    let summary = search::execute(args).await?;

    let mut caller_hits: Vec<CallerRef> = Vec::new();
    let mut definition_hits: Vec<&search::TopHit> = Vec::new();

    for hit in &summary.top_hits {
        let is_definition = is_definition_hit(hit);
        if is_definition {
            definition_hits.push(hit);
        }
        if callers_requested && (!is_definition || summary.top_hits.len() == 1) {
            caller_hits.push(CallerRef {
                file: hit.path.clone(),
                line: hit.line,
                snippet: pick_snippet(hit),
                origin: Some(hit.origin_label.clone()),
                git_info: hit.git_info.clone(),
            });
        }
    }

    if callers_requested && caller_hits.is_empty() {
        if let Some(first) = summary.top_hits.first() {
            caller_hits.push(CallerRef {
                file: first.path.clone(),
                line: first.line,
                snippet: pick_snippet(first),
                origin: Some(first.origin_label.clone()),
                git_info: first.git_info.clone(),
            });
        }
    }

    let callees = if callees_requested {
        extract_callees(&definition_hits, &summary.symbol)
    } else {
        Vec::new()
    };

    let output = RefsOutput {
        symbol: summary.symbol.clone(),
        callers: caller_hits,
        callees,
        warnings: summary.warnings.clone(),
        notes: summary.notes.clone(),
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

async fn handle_remove(cmd: RemoveCommand, disable_telemetry: bool) -> Result<(), AppError> {
    if !disable_telemetry {
        telemetry::init()?;
    }
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

fn is_definition_hit(hit: &search::TopHit) -> bool {
    if hit
        .hints
        .iter()
        .any(|hint| matches!(hint.kind.as_str(), "declaration" | "impl" | "type"))
    {
        return true;
    }

    let Some(text) = hit
        .snippet
        .as_deref()
        .or(hit.raw_snippet.as_deref())
        .map(|s| s.to_ascii_lowercase())
    else {
        return false;
    };

    let trimmed = text.trim_start();
    trimmed.starts_with("fn ")
        || trimmed.starts_with("func ")
        || trimmed.starts_with("pub fn")
        || trimmed.starts_with("struct ")
        || trimmed.starts_with("class ")
        || trimmed.starts_with("enum ")
        || trimmed.starts_with("trait ")
        || trimmed.starts_with("impl ")
}

fn pick_snippet(hit: &search::TopHit) -> Option<String> {
    hit.snippet
        .as_deref()
        .or(hit.expanded_snippet.as_deref())
        .or(hit.raw_snippet.as_deref())
        .map(normalize_line_snippet)
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

fn extract_callees(definitions: &[&search::TopHit], symbol: &str) -> Vec<CalleeRef> {
    let call_regex = Regex::new(r"(?m)\b([A-Za-z_][A-Za-z0-9_]*)\s*\(")
        .unwrap_or_else(|_| Regex::new("$^").unwrap());
    let stoplist: HashSet<&'static str> = [
        "if",
        "for",
        "while",
        "loop",
        "match",
        "switch",
        "return",
        "await",
        "self",
        "super",
        "this",
        "macro_rules",
        "let",
        "var",
        "func",
        "function",
        "class",
        "struct",
        "enum",
        "impl",
        "where",
    ]
    .into_iter()
    .collect();

    let mut seen: HashSet<(String, usize, String)> = HashSet::new();
    let mut results: Vec<CalleeRef> = Vec::new();
    let lowered_symbol = symbol.to_ascii_lowercase();

    for hit in definitions {
        let Some(body) = hit.body.as_deref() else {
            continue;
        };
        let start = hit.line.saturating_sub(50);
        let end = hit.line.saturating_add(120);

        for (idx, line) in body.lines().enumerate() {
            let line_no = idx + 1;
            if line_no < start || line_no > end {
                continue;
            }
            for caps in call_regex.captures_iter(line) {
                let name = caps
                    .get(1)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();
                if name.is_empty() {
                    continue;
                }
                let lowered = name.to_ascii_lowercase();
                if lowered == lowered_symbol || stoplist.contains(lowered.as_str()) {
                    continue;
                }
                let key = (lowered.clone(), line_no, hit.path.clone());
                if seen.insert(key) {
                    results.push(CalleeRef {
                        symbol: name,
                        file: hit.path.clone(),
                        line: line_no,
                        snippet: Some(normalize_line_snippet(line)),
                        git_info: hit.git_info.clone(),
                    });
                    if results.len() >= 50 {
                        return results;
                    }
                }
            }
        }
    }

    results
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
    let root = match &cli.command {
        Commands::Search(cmd) => resolve_root(&cmd.search)?,
        Commands::Usages(cmd) => resolve_root(&cmd.search)?,
        Commands::Refs(cmd) => resolve_root(&cmd.search)?,
        Commands::Remove(cmd) => resolve_root(&cmd.search)?,
        Commands::Serve(args) => {
            if let Some(path) = &args.path {
                path.clone()
            } else {
                std::env::current_dir().context("failed to resolve current directory")?
            }
        }
        Commands::Bench(args) => {
            if let Some(path) = &args.cache_dir {
                path.clone()
            } else {
                std::env::current_dir().context("failed to resolve current directory")?
            }
        }
    };
    clear_cache_dirs(&root)?;
    Ok(())
}

fn clear_cache_dirs(root: &Path) -> Result<(), AppError> {
    let cache_dir = root.join(".swe-grep-cache");
    let index_dir = root.join(".swe-grep-index");

    for dir in [cache_dir, index_dir] {
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .with_context(|| format!("failed to clear cache directory {}", dir.display()))?;
        }
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
    use swe_grep::cli::ToolArgs;

    fn search_args(symbol: &str) -> SearchArgs {
        SearchArgs {
            symbol: symbol.to_string(),
            concept: None,
            concept_expand: false,
            regex: false,
            wildcard: false,
            string: false,
            include_strings: false,
            code_only: false,
            ignore_case: false,
            definitions_only: false,
            usages_only: false,
            kind: None,
            path: None,
            path_glob: Vec::new(),
            exclude_patterns: Vec::new(),
            exclude_tests: false,
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
            git_modified: false,
            git_since: None,
            git_info: false,
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
            body_head: None,
            body_tail: None,
            full_body: false,
            definitions: false,
            open: false,
            open_command: None,
            assert_absent: false,
        };

        let symbols = collect_symbols(&cmd).expect("expected symbols to be collected");
        assert_eq!(
            symbols,
            vec!["Alpha".to_string(), "Beta".to_string(), "Gamma".to_string()]
        );
    }
}
