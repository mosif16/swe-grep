use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::Serialize;

use crate::search::{SearchSummary, TopHit};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct AgentSettings {
    pub include_definitions: bool,
    pub trim_body: Option<usize>,
    pub concise: bool,
}

#[derive(Serialize)]
pub struct AgentQuery {
    pub symbols: Vec<String>,
    pub path: String,
}

#[derive(Serialize)]
pub struct AgentSummary {
    pub count: usize,
    pub latency_ms: u64,
}

#[derive(Clone, Serialize)]
pub struct AgentHit {
    pub symbol: String,
    pub file: String,
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub def: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub body_truncated: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub body_retrieved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
}

#[derive(Serialize)]
pub struct AgentOutput {
    pub version: u32,
    pub cli_version: &'static str,
    pub query: AgentQuery,
    pub hits: Vec<AgentHit>,
    pub summary: AgentSummary,
    pub errors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_symbols: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_symbols: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<String>,
    pub truncated: bool,
}

#[derive(Serialize)]
pub struct AgentStreamEntry {
    pub version: u32,
    pub cli_version: &'static str,
    pub symbol: String,
    pub hits: Vec<AgentHit>,
    pub summary: AgentSummary,
    pub errors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_symbols: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_symbols: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<String>,
    pub truncated: bool,
    pub done: bool,
}

#[derive(Default)]
pub struct AgentResult {
    pub symbol: String,
    pub hits: Vec<AgentHit>,
    pub latency_ms: u64,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub truncated: bool,
    pub related_symbols: Vec<String>,
    pub suggested_symbols: Vec<String>,
    pub related_paths: Vec<String>,
    pub notes: Vec<String>,
    pub concept: Option<String>,
}

impl AgentResult {
    pub fn from_summary(symbol: &str, summary: &SearchSummary, settings: &AgentSettings) -> Self {
        let hits = summary
            .top_hits
            .iter()
            .map(|hit| map_hit(symbol, hit, settings))
            .collect();

        Self {
            symbol: symbol.to_string(),
            hits,
            latency_ms: summary.stage_stats.cycle_latency_ms,
            errors: Vec::new(),
            warnings: summary.warnings.clone(),
            truncated: summary.truncated,
            related_symbols: summary.related_symbols.clone(),
            suggested_symbols: summary.suggested_symbols.clone(),
            related_paths: summary
                .related_paths
                .iter()
                .map(|p| p.to_string())
                .collect(),
            notes: summary.notes.clone(),
            concept: summary.concept.clone(),
        }
    }
}

pub fn aggregate_output(root: &Path, symbols: &[String], results: &[AgentResult]) -> AgentOutput {
    let mut hits: Vec<AgentHit> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut latency_ms: u64 = 0;
    let mut truncated = false;
    let mut related_symbols: HashSet<String> = HashSet::new();
    let mut suggested_symbols: HashSet<String> = HashSet::new();
    let mut related_paths: HashSet<String> = HashSet::new();
    let mut notes: HashSet<String> = HashSet::new();
    let mut concept: Option<String> = None;

    for result in results {
        hits.extend(result.hits.clone());
        latency_ms = latency_ms.saturating_add(result.latency_ms);
        truncated |= result.truncated;
        errors.extend(
            result
                .errors
                .iter()
                .map(|err| format!("{}: {}", result.symbol, err)),
        );
        warnings.extend(
            result
                .warnings
                .iter()
                .map(|warn| format!("{}: {}", result.symbol, warn)),
        );
        related_symbols.extend(result.related_symbols.clone());
        suggested_symbols.extend(result.suggested_symbols.clone());
        related_paths.extend(result.related_paths.clone());
        notes.extend(result.notes.clone());
        if concept.is_none() {
            concept = result.concept.clone();
        }
    }

    let mut related_symbols: Vec<String> = related_symbols.into_iter().collect();
    let mut suggested_symbols: Vec<String> = suggested_symbols.into_iter().collect();
    let mut related_paths: Vec<String> = related_paths.into_iter().collect();
    let mut notes: Vec<String> = notes.into_iter().collect();
    related_symbols.sort();
    suggested_symbols.sort();
    related_paths.sort();
    notes.sort();

    AgentOutput {
        version: 1,
        cli_version: CLI_VERSION,
        query: AgentQuery {
            symbols: symbols.to_vec(),
            path: root.to_string_lossy().to_string(),
        },
        hits,
        summary: AgentSummary {
            count: results.iter().map(|r| r.hits.len()).sum(),
            latency_ms,
        },
        errors,
        warnings,
        related_symbols,
        suggested_symbols,
        related_paths,
        notes,
        concept,
        truncated,
    }
}

pub fn stream_entry(result: &AgentResult, done: bool) -> AgentStreamEntry {
    AgentStreamEntry {
        version: 1,
        cli_version: CLI_VERSION,
        symbol: result.symbol.clone(),
        hits: result.hits.clone(),
        summary: AgentSummary {
            count: result.hits.len(),
            latency_ms: result.latency_ms,
        },
        errors: result.errors.clone(),
        warnings: result.warnings.clone(),
        related_symbols: result.related_symbols.clone(),
        suggested_symbols: result.suggested_symbols.clone(),
        related_paths: result.related_paths.clone(),
        notes: result.notes.clone(),
        concept: result.concept.clone(),
        truncated: result.truncated,
        done,
    }
}

pub fn render_table(results: &[AgentHit]) -> String {
    const SYMBOL_COL_WIDTH: usize = 16;
    const LOCATION_COL_WIDTH: usize = 44;
    const PREVIEW_COL_WIDTH: usize = 60;

    let mut buffer = String::new();
    let mut rows = results.to_vec();
    rows.sort_by(|a, b| {
        a.symbol
            .cmp(&b.symbol)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });

    buffer.push_str(&format!(
        "{:<symbol_width$} {:<location_width$} {}\n",
        "SYMBOL",
        "LOCATION",
        "DEF/SNIPPET",
        symbol_width = SYMBOL_COL_WIDTH,
        location_width = LOCATION_COL_WIDTH
    ));

    for hit in rows {
        let location_raw = format!("{}:{}", hit.file, hit.line);
        let location = truncate_middle(&location_raw, LOCATION_COL_WIDTH);
        let preview_raw = hit
            .def
            .clone()
            .or_else(|| hit.snippet.clone())
            .unwrap_or_else(|| "<no snippet>".to_string());
        let preview = truncate_tail(&collapse_whitespace(&preview_raw), PREVIEW_COL_WIDTH);
        let symbol = truncate_tail(&hit.symbol, SYMBOL_COL_WIDTH);
        buffer.push_str(&format!(
            "{:<symbol_width$} {:<location_width$} {}\n",
            symbol,
            location,
            preview,
            symbol_width = SYMBOL_COL_WIDTH,
            location_width = LOCATION_COL_WIDTH
        ));
    }

    buffer
}

fn map_hit(symbol: &str, hit: &TopHit, settings: &AgentSettings) -> AgentHit {
    let snippet = if settings.concise {
        hit.snippet.clone()
    } else {
        hit.snippet.clone().or_else(|| hit.raw_snippet.clone())
    };
    let (body, body_truncated, body_retrieved) = if settings.concise {
        (None, false, false)
    } else {
        let (body, truncated) = trim_body(hit.body.as_deref(), settings.trim_body);
        (body, truncated, hit.body_retrieved)
    };

    AgentHit {
        symbol: symbol.to_string(),
        file: hit.path.clone(),
        line: hit.line,
        def: settings
            .include_definitions
            .then(|| extract_definition(hit, symbol)),
        snippet,
        context: if settings.concise {
            None
        } else {
            hit.expanded_snippet.clone()
        },
        body,
        body_truncated,
        body_retrieved,
        origin: if settings.concise {
            None
        } else {
            Some(hit.origin_label.clone())
        },
        score: if settings.concise {
            None
        } else {
            Some(hit.score)
        },
    }
}

fn trim_body(body: Option<&str>, trim: Option<usize>) -> (Option<String>, bool) {
    let Some(body) = body else {
        return (None, false);
    };
    let Some(limit) = trim else {
        return (Some(body.to_string()), false);
    };
    if limit == 0 {
        return (Some(String::new()), true);
    }

    let mut lines: Vec<&str> = body.lines().collect();
    if lines.len() <= limit {
        return (Some(body.to_string()), false);
    }
    lines.truncate(limit);
    (Some(lines.join("\n")), true)
}

fn extract_definition(hit: &TopHit, symbol: &str) -> String {
    if let Some(label) = hit
        .hints
        .iter()
        .find(|hint| hint.kind == "declaration")
        .map(|hint| hint.label.clone())
    {
        return label;
    }

    if let Some(label) = hit
        .hints
        .iter()
        .find(|hint| hint.kind == "impl")
        .map(|hint| hint.label.clone())
    {
        return label;
    }

    if let Some(snippet) = hit.snippet.as_deref().or(hit.raw_snippet.as_deref()) {
        if let Some(line) = first_content_line(snippet) {
            return line;
        }
    }

    symbol.to_string()
}

fn first_content_line(snippet: &str) -> Option<String> {
    snippet.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn is_false(value: &bool) -> bool {
    !*value
}

pub fn render_compact(results: &[AgentHit]) -> String {
    let mut rows = results.to_vec();
    rows.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.line.cmp(&b.line)));
    rows.into_iter()
        .map(|hit| {
            let preview = hit
                .def
                .clone()
                .or(hit.snippet.clone())
                .unwrap_or_else(|| "<no snippet>".to_string());
            format!(
                "{} {}:{} {}",
                hit.symbol,
                hit.file,
                hit.line,
                collapse_whitespace(&preview)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render_tree_grouped(results: &[AgentHit]) -> String {
    let mut grouped: BTreeMap<String, Vec<&AgentHit>> = BTreeMap::new();
    for hit in results {
        grouped.entry(hit.file.clone()).or_default().push(hit);
    }
    let mut buffer = String::new();
    for (file, hits) in grouped {
        buffer.push_str(&file);
        buffer.push('\n');
        for (idx, hit) in hits.iter().enumerate() {
            let connector = if idx + 1 == hits.len() { "\\--" } else { "|--" };
            let preview = hit
                .def
                .clone()
                .or(hit.snippet.clone())
                .unwrap_or_else(|| "<no snippet>".to_string());
            let collapsed = collapse_whitespace(&preview);
            buffer.push_str(&format!(
                "{connector} {}:{} {collapsed}\n",
                hit.symbol, hit.line
            ));
        }
    }
    buffer
}

pub fn render_edit(results: &[AgentHit]) -> String {
    let mut rows = results.to_vec();
    rows.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.line.cmp(&b.line)));
    rows.into_iter()
        .map(|hit| {
            let preview = hit
                .snippet
                .clone()
                .or(hit.def.clone())
                .unwrap_or_else(|| "<no snippet>".to_string());
            let collapsed = collapse_whitespace(&preview);
            format!("{}:{}: {}", hit.file, hit.line, collapsed)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_tail(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.to_string();
    }
    if max_len <= 3 {
        return ".".repeat(max_len);
    }
    format!("{}...", &value[..max_len.saturating_sub(3)])
}

fn truncate_middle(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.to_string();
    }
    if max_len <= 3 {
        return ".".repeat(max_len);
    }

    let keep = max_len.saturating_sub(3);
    let front = keep / 2;
    let back = keep.saturating_sub(front);
    format!(
        "{}...{}",
        &value[..front],
        &value[value.len().saturating_sub(back)..]
    )
}

fn collapse_whitespace(value: &str) -> String {
    value
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
