use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde::Serialize;

use crate::search::{
    ConceptExpansionSummary, FuzzyMatch, GitInfo, HitSemanticKind, SearchSummary,
    SemanticClassification, TopHit, UsageContextKind, classify_semantic,
};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct AgentSettings {
    pub include_definitions: bool,
    pub trim_body: Option<usize>,
    pub body_head: Option<usize>,
    pub body_tail: Option<usize>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_info: Option<GitInfo>,
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
    pub fuzzy_matches: Vec<FuzzyMatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub did_you_mean: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fd_candidates: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept_expansion: Option<ConceptExpansionSummary>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ast_fallback: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_commands: Vec<String>,
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
    pub fuzzy_matches: Vec<FuzzyMatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub did_you_mean: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fd_candidates: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept_expansion: Option<ConceptExpansionSummary>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ast_fallback: bool,
    pub truncated: bool,
    pub done: bool,
}

#[derive(Clone, Serialize)]
pub struct GroupedLocation {
    pub file: String,
    pub signature: String,
}

#[derive(Clone, Serialize)]
pub struct GroupedImplementation {
    pub file: String,
    #[serde(rename = "trait")]
    pub trait_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct GroupedUsageSite {
    pub line: usize,
    pub context: String,
}

#[derive(Clone, Serialize)]
pub struct GroupedSymbolBuckets {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<GroupedLocation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implementations: Vec<GroupedImplementation>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub usages_by_file: BTreeMap<String, Vec<GroupedUsageSite>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_coverage: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct GroupedSymbolSummary {
    pub definition: usize,
    pub impls: usize,
    pub usages: usize,
    pub tests: usize,
}

#[derive(Clone, Serialize)]
pub struct GroupedAgentEntry {
    pub symbol: String,
    pub grouped: GroupedSymbolBuckets,
    pub summary: GroupedSymbolSummary,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

#[derive(Serialize)]
pub struct GroupedAgentOutput {
    pub version: u32,
    pub cli_version: &'static str,
    pub query: AgentQuery,
    pub results: Vec<GroupedAgentEntry>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fd_candidates: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept_expansion: Option<ConceptExpansionSummary>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ast_fallback: bool,
    pub truncated: bool,
}

#[derive(Serialize)]
pub struct GroupedAgentStreamEntry {
    pub version: u32,
    pub cli_version: &'static str,
    pub symbol: String,
    pub grouped: GroupedSymbolBuckets,
    pub summary: GroupedSymbolSummary,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fd_candidates: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept_expansion: Option<ConceptExpansionSummary>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ast_fallback: bool,
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
    pub fuzzy_matches: Vec<FuzzyMatch>,
    pub did_you_mean: Option<String>,
    pub related_paths: Vec<String>,
    pub notes: Vec<String>,
    pub fd_candidates: Vec<String>,
    pub concept: Option<String>,
    pub concept_expansion: Option<ConceptExpansionSummary>,
    pub ast_fallback: bool,
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
            fuzzy_matches: summary.fuzzy_matches.clone(),
            did_you_mean: summary.did_you_mean.clone(),
            related_paths: summary
                .related_paths
                .iter()
                .map(|p| p.to_string())
                .collect(),
            notes: summary.notes.clone(),
            fd_candidates: summary
                .fd_candidates
                .iter()
                .filter_map(|p| p.to_str().map(|s| s.to_string()))
                .take(20)
                .collect(),
            concept: summary.concept.clone(),
            concept_expansion: summary.concept_expansion.clone(),
            ast_fallback: summary.ast_fallback,
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
    let mut fd_candidates: HashSet<String> = HashSet::new();
    let mut notes: HashSet<String> = HashSet::new();
    let mut fuzzy_matches: Vec<FuzzyMatch> = Vec::new();
    let mut did_you_mean: Option<String> = None;
    let mut concept: Option<String> = None;
    let mut concept_expansion: Option<ConceptExpansionSummary> = None;
    let mut ast_fallback = false;

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
        fd_candidates.extend(result.fd_candidates.clone());
        notes.extend(result.notes.clone());
        fuzzy_matches.extend(result.fuzzy_matches.clone());
        ast_fallback |= result.ast_fallback;
        if concept.is_none() {
            concept = result.concept.clone();
        }
        if concept_expansion.is_none() {
            concept_expansion = result.concept_expansion.clone();
        }
        if results.len() == 1 && did_you_mean.is_none() {
            did_you_mean = result.did_you_mean.clone();
        }
    }

    let mut related_symbols: Vec<String> = related_symbols.into_iter().collect();
    let mut suggested_symbols: Vec<String> = suggested_symbols.into_iter().collect();
    let mut related_paths: Vec<String> = related_paths.into_iter().collect();
    let mut fd_candidates: Vec<String> = fd_candidates.into_iter().collect();
    let mut notes: Vec<String> = notes.into_iter().collect();
    related_symbols.sort();
    suggested_symbols.sort();
    related_paths.sort();
    fd_candidates.sort();
    notes.sort();
    if fd_candidates.len() > 20 {
        fd_candidates.truncate(20);
    }

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
        fuzzy_matches: dedupe_fuzzy_matches(fuzzy_matches),
        did_you_mean,
        related_paths,
        notes,
        fd_candidates,
        concept,
        concept_expansion,
        ast_fallback,
        open_commands: Vec::new(),
        truncated,
    }
}

pub fn aggregate_grouped_output(
    root: &Path,
    symbols: &[String],
    results: &[AgentResult],
    budget_tokens: Option<usize>,
) -> GroupedAgentOutput {
    let mut entries: Vec<GroupedAgentEntry> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut latency_ms: u64 = 0;
    let mut truncated = false;
    let mut related_symbols: HashSet<String> = HashSet::new();
    let mut suggested_symbols: HashSet<String> = HashSet::new();
    let mut related_paths: HashSet<String> = HashSet::new();
    let mut fd_candidates: HashSet<String> = HashSet::new();
    let mut notes: HashSet<String> = HashSet::new();
    let mut concept: Option<String> = None;
    let mut concept_expansion: Option<ConceptExpansionSummary> = None;
    let mut ast_fallback = false;

    for result in results {
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
        fd_candidates.extend(result.fd_candidates.clone());
        notes.extend(result.notes.clone());
        ast_fallback |= result.ast_fallback;
        if concept.is_none() {
            concept = result.concept.clone();
        }
        if concept_expansion.is_none() {
            concept_expansion = result.concept_expansion.clone();
        }

        let (mut entry, entry_truncated) =
            group_agent_hits(&result.symbol, &result.hits, budget_tokens);
        entry.truncated |= entry_truncated;
        truncated |= entry.truncated;
        entries.push(entry);
    }

    let mut related_symbols: Vec<String> = related_symbols.into_iter().collect();
    let mut suggested_symbols: Vec<String> = suggested_symbols.into_iter().collect();
    let mut related_paths: Vec<String> = related_paths.into_iter().collect();
    let mut fd_candidates: Vec<String> = fd_candidates.into_iter().collect();
    let mut notes: Vec<String> = notes.into_iter().collect();
    related_symbols.sort();
    suggested_symbols.sort();
    related_paths.sort();
    fd_candidates.sort();
    notes.sort();
    if fd_candidates.len() > 20 {
        fd_candidates.truncate(20);
    }

    GroupedAgentOutput {
        version: 1,
        cli_version: CLI_VERSION,
        query: AgentQuery {
            symbols: symbols.to_vec(),
            path: root.to_string_lossy().to_string(),
        },
        results: entries,
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
        fd_candidates,
        concept,
        concept_expansion,
        ast_fallback,
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
        fuzzy_matches: result.fuzzy_matches.clone(),
        did_you_mean: result.did_you_mean.clone(),
        related_paths: result.related_paths.clone(),
        notes: result.notes.clone(),
        fd_candidates: result.fd_candidates.clone(),
        concept: result.concept.clone(),
        concept_expansion: result.concept_expansion.clone(),
        ast_fallback: result.ast_fallback,
        truncated: result.truncated,
        done,
    }
}

fn dedupe_fuzzy_matches(matches: Vec<FuzzyMatch>) -> Vec<FuzzyMatch> {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut deduped = Vec::new();
    for m in matches {
        if seen.insert((m.symbol.clone(), m.file.clone())) {
            deduped.push(m);
        }
    }
    deduped.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.symbol.cmp(&b.symbol))
            .then_with(|| a.file.cmp(&b.file))
    });
    if deduped.len() > 20 {
        deduped.truncate(20);
    }
    deduped
}

pub fn grouped_stream_entry(
    result: &AgentResult,
    done: bool,
    budget_tokens: Option<usize>,
) -> GroupedAgentStreamEntry {
    let (entry, _truncated) = group_agent_hits(&result.symbol, &result.hits, budget_tokens);
    GroupedAgentStreamEntry {
        version: 1,
        cli_version: CLI_VERSION,
        symbol: result.symbol.clone(),
        grouped: entry.grouped,
        summary: entry.summary,
        errors: result.errors.clone(),
        warnings: result.warnings.clone(),
        related_symbols: result.related_symbols.clone(),
        suggested_symbols: result.suggested_symbols.clone(),
        related_paths: result.related_paths.clone(),
        notes: result.notes.clone(),
        fd_candidates: result.fd_candidates.clone(),
        concept: result.concept.clone(),
        concept_expansion: result.concept_expansion.clone(),
        ast_fallback: result.ast_fallback,
        truncated: result.truncated || entry.truncated,
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
        let (body, truncated) = format_body(
            hit.body.as_deref(),
            settings.trim_body,
            settings.body_head,
            settings.body_tail,
        );
        (body, truncated || hit.body_truncated, hit.body_retrieved)
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
        git_info: hit.git_info.clone(),
    }
}

fn format_body(
    body: Option<&str>,
    trim: Option<usize>,
    head: Option<usize>,
    tail: Option<usize>,
) -> (Option<String>, bool) {
    let Some(body) = body else {
        return (None, false);
    };

    let lines: Vec<&str> = body.lines().collect();
    let head_count = head.unwrap_or(0);
    let tail_count = tail.unwrap_or(0);

    if head.is_some() || tail.is_some() {
        let keep_head = head_count.min(lines.len());
        let keep_tail = tail_count.min(lines.len().saturating_sub(keep_head));
        let total_keep = keep_head + keep_tail;
        if total_keep == 0 {
            return (Some(String::new()), !lines.is_empty());
        }
        if total_keep >= lines.len() {
            return (Some(body.to_string()), false);
        }

        let mut segments: Vec<String> = Vec::new();
        if keep_head > 0 {
            segments.push(lines[..keep_head].join("\n"));
        }
        segments.push("...".to_string());
        if keep_tail > 0 {
            segments.push(lines[lines.len().saturating_sub(keep_tail)..].join("\n"));
        }
        return (Some(segments.join("\n")), true);
    }

    let Some(limit) = trim else {
        return (Some(body.to_string()), false);
    };
    if limit == 0 {
        return (Some(String::new()), true);
    }

    if lines.len() <= limit {
        return (Some(body.to_string()), false);
    }

    let mut shortened = lines;
    shortened.truncate(limit);
    (Some(shortened.join("\n")), true)
}

fn group_agent_hits(
    symbol: &str,
    hits: &[AgentHit],
    budget_tokens: Option<usize>,
) -> (GroupedAgentEntry, bool) {
    let mut definition: Option<GroupedLocation> = None;
    let mut impls: Vec<GroupedImplementation> = Vec::new();
    let mut tests: HashSet<String> = HashSet::new();

    let mut usages_by_file: HashMap<String, Vec<GroupedUsageSite>> = HashMap::new();
    let mut usage_priority: HashMap<(String, usize), i32> = HashMap::new();

    for hit in hits {
        let snippet_line = hit
            .snippet
            .as_deref()
            .and_then(first_content_line)
            .unwrap_or_else(|| symbol.to_string());
        let classification = classify_semantic(Path::new(&hit.file), &snippet_line, symbol);

        match classification.kind {
            HitSemanticKind::Test => {
                tests.insert(hit.file.clone());
            }
            HitSemanticKind::Definition => {
                if definition.is_none() {
                    let signature = hit
                        .def
                        .clone()
                        .or_else(|| hit.snippet.clone().and_then(|s| first_content_line(&s)))
                        .unwrap_or_else(|| snippet_line.clone());
                    definition = Some(GroupedLocation {
                        file: format!("{}:{}", hit.file, hit.line),
                        signature,
                    });
                }
            }
            HitSemanticKind::Implementation => {
                let trait_name = classification
                    .trait_name
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "inherent".to_string());
                let methods = hit
                    .body
                    .as_deref()
                    .map(|body| extract_methods_near(body, hit.line, &hit.file))
                    .unwrap_or_default();
                impls.push(GroupedImplementation {
                    file: format!("{}:{}", hit.file, hit.line),
                    trait_name,
                    methods,
                });
            }
            HitSemanticKind::Import | HitSemanticKind::Usage => {
                let context = usage_context_label(&classification);
                let entry = GroupedUsageSite {
                    line: hit.line,
                    context,
                };
                let key = (hit.file.clone(), hit.line);
                let priority = usage_context_priority(&classification);
                let should_replace = match usage_priority.get(&key) {
                    Some(existing) => priority > *existing,
                    None => true,
                };
                if should_replace {
                    usage_priority.insert(key, priority);
                    let sites = usages_by_file.entry(hit.file.clone()).or_default();
                    if let Some(existing) = sites.iter_mut().find(|site| site.line == hit.line) {
                        *existing = entry;
                    } else {
                        sites.push(entry);
                    }
                }
            }
        }
    }

    impls.sort_by(|a, b| a.file.cmp(&b.file));

    let usages_by_file: BTreeMap<String, Vec<GroupedUsageSite>> = usages_by_file
        .into_iter()
        .map(|(file, mut sites)| {
            sites.sort_by(|a, b| a.line.cmp(&b.line));
            (file, sites)
        })
        .collect();

    let mut test_coverage: Vec<String> = tests.into_iter().collect();
    test_coverage.sort();

    let summary = GroupedSymbolSummary {
        definition: usize::from(definition.is_some()),
        impls: impls.len(),
        usages: usages_by_file.values().map(|v| v.len()).sum(),
        tests: test_coverage.len(),
    };

    let mut grouped = GroupedSymbolBuckets {
        definition,
        implementations: impls,
        usages_by_file,
        test_coverage,
    };

    let mut truncated = false;
    if let Some(budget_tokens) = budget_tokens {
        truncated = apply_grouped_budget(&mut grouped, budget_tokens);
    }

    (
        GroupedAgentEntry {
            symbol: symbol.to_string(),
            grouped,
            summary,
            truncated,
        },
        truncated,
    )
}

fn usage_context_label(classification: &SemanticClassification) -> String {
    if classification.kind == HitSemanticKind::Import {
        return "import".to_string();
    }
    match classification
        .usage_context
        .unwrap_or(UsageContextKind::Other)
    {
        UsageContextKind::Construction => "construction".to_string(),
        UsageContextKind::MethodCall => "method_call".to_string(),
        UsageContextKind::TypeReference => "type_reference".to_string(),
        UsageContextKind::Other => "usage".to_string(),
    }
}

fn usage_context_priority(classification: &SemanticClassification) -> i32 {
    if classification.kind == HitSemanticKind::Import {
        return 100;
    }
    match classification
        .usage_context
        .unwrap_or(UsageContextKind::Other)
    {
        UsageContextKind::Construction => 80,
        UsageContextKind::MethodCall => 60,
        UsageContextKind::TypeReference => 40,
        UsageContextKind::Other => 20,
    }
}

fn apply_grouped_budget(grouped: &mut GroupedSymbolBuckets, budget_tokens: usize) -> bool {
    if budget_tokens == 0 {
        grouped.usages_by_file.clear();
        grouped.implementations.clear();
        grouped.test_coverage.clear();
        return true;
    }

    let mut truncated = false;
    grouped.test_coverage.truncate(20);

    let mut per_file_limit = 50usize;
    let mut max_files: Option<usize> = None;
    let mut max_impls: Option<usize> = None;

    for _ in 0..8 {
        let tokens = estimate_grouped_tokens(grouped);
        if tokens <= budget_tokens {
            break;
        }

        truncated = true;

        if per_file_limit > 5 {
            per_file_limit = usize::max(5, per_file_limit / 2);
            for sites in grouped.usages_by_file.values_mut() {
                if sites.len() > per_file_limit {
                    sites.truncate(per_file_limit);
                }
            }
            continue;
        }

        if max_files.is_none() {
            max_files = Some(30);
        }
        if let Some(limit) = max_files {
            if grouped.usages_by_file.len() > limit {
                grouped.usages_by_file = keep_top_usage_files(&grouped.usages_by_file, limit);
                max_files = Some(usize::max(10, limit / 2));
                continue;
            }
        }

        if max_impls.is_none() {
            max_impls = Some(10);
        }
        if let Some(limit) = max_impls {
            if grouped.implementations.len() > limit {
                grouped.implementations.truncate(limit);
                max_impls = Some(usize::max(3, limit / 2));
                continue;
            }
        }

        break;
    }

    truncated
}

fn keep_top_usage_files(
    map: &BTreeMap<String, Vec<GroupedUsageSite>>,
    limit: usize,
) -> BTreeMap<String, Vec<GroupedUsageSite>> {
    let mut files: Vec<(String, Vec<GroupedUsageSite>)> =
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    files.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
    files.truncate(limit);
    files.into_iter().collect()
}

fn estimate_grouped_tokens(grouped: &GroupedSymbolBuckets) -> usize {
    let bytes = match serde_json::to_vec(grouped) {
        Ok(bytes) => bytes.len(),
        Err(_) => 0,
    };
    usize::max(1, bytes / 4)
}

fn extract_methods_near(body: &str, anchor_line: usize, path: &str) -> Vec<String> {
    if !path.ends_with(".rs") {
        return Vec::new();
    }

    let lines: Vec<&str> = body.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let anchor = anchor_line.saturating_sub(1);
    let start = usize::min(lines.len(), anchor.saturating_sub(30));
    let end = usize::min(lines.len(), anchor.saturating_add(200));
    let mut methods: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for line in &lines[start..end] {
        let mut trimmed = line.trim_start();
        if trimmed.starts_with("pub ") {
            trimmed = trimmed.trim_start_matches("pub").trim_start();
        }
        if trimmed.starts_with("async ") {
            trimmed = trimmed.trim_start_matches("async").trim_start();
        }
        if trimmed.starts_with("unsafe ") {
            trimmed = trimmed.trim_start_matches("unsafe").trim_start();
        }
        if !trimmed.starts_with("fn ") {
            continue;
        }
        let after_fn = trimmed.trim_start_matches("fn").trim_start();
        let name = after_fn
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect::<String>();
        if name.is_empty() || !seen.insert(name.clone()) {
            continue;
        }
        methods.push(name);
        if methods.len() >= 20 {
            break;
        }
    }

    methods
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

#[derive(Serialize)]
pub struct FileSummary {
    pub path: String,
    pub hits: usize,
}

#[derive(Serialize)]
pub struct SummaryView {
    pub files: Vec<FileSummary>,
    pub total: usize,
}

pub fn summarize_hits(results: &[AgentHit]) -> SummaryView {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for hit in results {
        *counts.entry(hit.file.clone()).or_default() += 1;
    }
    let mut files: Vec<FileSummary> = counts
        .into_iter()
        .map(|(path, hits)| FileSummary { path, hits })
        .collect();
    files.sort_by(|a, b| b.hits.cmp(&a.hits).then_with(|| a.path.cmp(&b.path)));

    SummaryView {
        files,
        total: results.len(),
    }
}

pub fn render_minimal(results: &[AgentHit]) -> String {
    let mut rows = results.to_vec();
    rows.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.line.cmp(&b.line)));
    rows.into_iter()
        .map(|hit| {
            let snippet = hit
                .snippet
                .clone()
                .or(hit.def.clone())
                .unwrap_or_else(|| "<no snippet>".to_string());
            format!(
                "{}:{}:{}",
                hit.file,
                hit.line,
                collapse_whitespace(&snippet)
            )
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
