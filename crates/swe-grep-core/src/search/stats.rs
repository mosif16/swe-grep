use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant as StdInstant;

use serde::Serialize;
use tokio::time::Instant;

use crate::language::detect_language_from_path;
use crate::search::formatting::ContextHint;
use crate::search::hits::SearchHit;
use crate::tools::ast_grep::AstGrepMatch;

#[derive(Default)]
pub(crate) struct SearchMetrics {
    pub(crate) precision: f32,
    pub(crate) density: f32,
    pub(crate) cluster_score: f32,
    pub(crate) reward: f32,
}

pub(crate) fn compute_metrics(
    hits: &[SearchHit],
    ast_set: &HashSet<(PathBuf, usize)>,
    fd_candidates: usize,
) -> SearchMetrics {
    if hits.is_empty() {
        return SearchMetrics::default();
    }

    let precision = ast_set.len() as f32 / hits.len() as f32;

    let unique_files: HashSet<_> = hits.iter().map(|hit| hit.path.clone()).collect();
    let density_raw = hits.len() as f32 / unique_files.len() as f32;
    let density = density_raw / (density_raw + 1.0);

    let (min_line, max_line) = hits.iter().fold((usize::MAX, 0usize), |acc, hit| {
        (acc.0.min(hit.line), acc.1.max(hit.line))
    });
    let line_span = if max_line >= min_line {
        max_line - min_line
    } else {
        0
    };
    let cluster_norm = line_span as f32 / (hits.len() as f32 + 1.0);
    let cluster_score = 1.0 / (1.0 + cluster_norm);

    let fd_bonus = if fd_candidates > 0 {
        (hits.len().min(fd_candidates) as f32) / fd_candidates as f32
    } else {
        0.0
    };

    let reward = 0.5 * precision + 0.3 * density + 0.15 * cluster_score + 0.05 * fd_bonus;

    SearchMetrics {
        precision,
        density,
        cluster_score,
        reward,
    }
}

pub(crate) fn aggregate_language_counts<'a, I>(paths: I) -> BTreeMap<String, usize>
where
    I: IntoIterator<Item = &'a Path>,
{
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for path in paths {
        let key = detect_language_from_path(path)
            .map(|lang| lang.to_string())
            .unwrap_or_else(|| "other".to_string());
        *counts.entry(key).or_default() += 1;
    }
    counts
}

pub(crate) fn elapsed_std_ms(start: StdInstant) -> u64 {
    let nanos = start.elapsed().as_nanos();
    ((nanos + 999_999) / 1_000_000) as u64
}

pub(crate) fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

pub(crate) fn round_two(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

#[derive(Default, Clone, Serialize)]
pub struct StartupStats {
    pub init_ms: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub fd_ms: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub rg_ms: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub ast_ms: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub rga_ms: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub cache_ms: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub state_ms: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub index_ms: u64,
}

#[derive(Default, Serialize)]
pub struct StageStats {
    pub discover_candidates: usize,
    pub discover_ms: u64,
    pub probe_hits: usize,
    pub probe_ms: u64,
    pub escalate_hits: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub escalate_ms: u64,
    pub index_candidates: usize,
    pub index_probe_hits: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub index_ms: u64,
    pub rga_hits: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub rga_ms: u64,
    pub ast_matches: usize,
    pub disambiguate_ms: u64,
    pub verify_ms: u64,
    pub cycle_latency_ms: u64,
    pub precision: f32,
    pub density: f32,
    pub clustering: f32,
    pub reward: f32,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub language_metrics: BTreeMap<String, LanguageMetrics>,
}

#[derive(Default, Serialize)]
pub struct LanguageMetrics {
    #[serde(skip_serializing_if = "is_usize_zero")]
    pub discover_candidates: usize,
    #[serde(skip_serializing_if = "is_usize_zero")]
    pub probe_hits: usize,
    #[serde(skip_serializing_if = "is_usize_zero")]
    pub escalate_hits: usize,
    #[serde(skip_serializing_if = "is_usize_zero")]
    pub disambiguate_hits: usize,
    #[serde(skip_serializing_if = "is_usize_zero")]
    pub verify_hits: usize,
    #[serde(skip_serializing_if = "LanguageLatencyStats::is_empty")]
    pub latency: LanguageLatencyStats,
}

#[derive(Default, Serialize)]
pub struct LanguageLatencyStats {
    #[serde(skip_serializing_if = "is_zero")]
    pub discover_ms: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub probe_ms: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub escalate_ms: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub disambiguate_ms: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub verify_ms: u64,
}

impl StageStats {
    pub(crate) fn record_discover_languages(&mut self, candidates: &[PathBuf], latency_ms: u64) {
        if candidates.is_empty() {
            return;
        }
        let counts = aggregate_language_counts(candidates.iter().map(|p| p.as_path()));
        if counts.is_empty() {
            return;
        }
        let mut shares = distribute_latency(latency_ms, counts.len()).into_iter();
        for (lang, count) in counts {
            let share = shares.next().unwrap_or(0);
            let metrics = self.language_metrics.entry(lang).or_default();
            metrics.discover_candidates += count;
            metrics.latency.discover_ms = metrics.latency.discover_ms.saturating_add(share);
        }
    }

    pub(crate) fn record_probe_languages(&mut self, hits: &[SearchHit], latency_ms: u64) {
        if hits.is_empty() {
            return;
        }
        let counts = aggregate_language_counts(hits.iter().map(|hit| hit.path.as_path()));
        if counts.is_empty() {
            return;
        }
        let mut shares = distribute_latency(latency_ms, counts.len()).into_iter();
        for (lang, count) in counts {
            let share = shares.next().unwrap_or(0);
            let metrics = self.language_metrics.entry(lang).or_default();
            metrics.probe_hits += count;
            metrics.latency.probe_ms = metrics.latency.probe_ms.saturating_add(share);
        }
    }

    pub(crate) fn record_escalate_languages(&mut self, hits: &[SearchHit], latency_ms: u64) {
        if hits.is_empty() {
            return;
        }
        let counts = aggregate_language_counts(hits.iter().map(|hit| hit.path.as_path()));
        if counts.is_empty() {
            return;
        }
        let mut shares = distribute_latency(latency_ms, counts.len()).into_iter();
        for (lang, count) in counts {
            let share = shares.next().unwrap_or(0);
            let metrics = self.language_metrics.entry(lang).or_default();
            metrics.escalate_hits += count;
            metrics.latency.escalate_ms = metrics.latency.escalate_ms.saturating_add(share);
        }
    }

    pub(crate) fn record_disambiguate_languages(
        &mut self,
        matches: &[AstGrepMatch],
        latency_ms: u64,
    ) {
        if matches.is_empty() {
            return;
        }
        let counts = aggregate_language_counts(matches.iter().map(|m| m.path.as_path()));
        if counts.is_empty() {
            return;
        }
        let mut shares = distribute_latency(latency_ms, counts.len()).into_iter();
        for (lang, count) in counts {
            let share = shares.next().unwrap_or(0);
            let metrics = self.language_metrics.entry(lang).or_default();
            metrics.disambiguate_hits += count;
            metrics.latency.disambiguate_ms = metrics.latency.disambiguate_ms.saturating_add(share);
        }
    }

    pub(crate) fn record_verify_languages(
        &mut self,
        counts: &BTreeMap<String, usize>,
        latency_ms: u64,
    ) {
        if counts.is_empty() {
            return;
        }
        let mut shares = distribute_latency(latency_ms, counts.len()).into_iter();
        for (lang, count) in counts {
            let share = shares.next().unwrap_or(0);
            let metrics = self.language_metrics.entry(lang.clone()).or_default();
            metrics.verify_hits += *count;
            metrics.latency.verify_ms = metrics.latency.verify_ms.saturating_add(share);
        }
    }
}

#[derive(Serialize)]
pub struct SearchSummary {
    pub cycle: u32,
    pub root: PathBuf,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concept: Option<String>,
    pub queries: Vec<String>,
    pub top_hits: Vec<TopHit>,
    pub deduped: usize,
    pub max_matches: usize,
    pub next_actions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_symbols: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_symbols: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fd_candidates: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ast_hits: Vec<(PathBuf, usize)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_stats: Option<StartupStats>,
    pub stage_stats: StageStats,
    pub reward: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

#[derive(Clone, Serialize)]
pub struct GitInfo {
    pub last_commit: String,
    pub last_modified: String,
}

#[derive(Clone, Serialize)]
pub struct TopHit {
    pub path: String,
    pub line: usize,
    pub score: f32,
    pub origin: String,
    pub origin_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet_length: Option<usize>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub raw_snippet_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expanded_snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_end: Option<usize>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto_expanded_context: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub body_retrieved: bool,
    #[serde(default, skip_serializing_if = "hints_is_empty")]
    pub hints: Vec<ContextHint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_info: Option<GitInfo>,
}

pub(crate) fn distribute_latency(latency_ms: u64, buckets: usize) -> Vec<u64> {
    if buckets == 0 {
        return Vec::new();
    }
    let buckets_u64 = buckets as u64;
    let base = latency_ms / buckets_u64;
    let remainder = latency_ms % buckets_u64;
    let mut shares = vec![base; buckets];
    for share in shares.iter_mut().take(remainder as usize) {
        *share = share.saturating_add(1);
    }
    shares
}

impl LanguageLatencyStats {
    pub(crate) fn is_empty(&self) -> bool {
        self.discover_ms == 0
            && self.probe_ms == 0
            && self.escalate_ms == 0
            && self.disambiguate_ms == 0
            && self.verify_ms == 0
    }
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn is_usize_zero(value: &usize) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn hints_is_empty(value: &Vec<ContextHint>) -> bool {
    value.is_empty()
}
