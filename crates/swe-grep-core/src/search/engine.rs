use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant as StdInstant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::{self, json};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::time::Instant;

use crate::file_cache::FileCache;
use crate::tools::ast_grep::{AstGrepMatch, AstGrepTool, AstPatternError};
use crate::tools::fd::FdTool;
use crate::tools::rg::RipgrepTool;
use crate::tools::rga::RgaTool;
#[cfg(feature = "indexing")]
use swe_grep_indexer::{IndexConfig, TantivyIndex};

use super::config::{
    DEFAULT_INLINE_CONTEXT, SearchConfig, TRUNCATED_INLINE_CONTEXT, extensions_for_languages,
    languages_include, normalize_path, passes_extension_filter,
};
use super::formatting::{
    ContextHint, format_snippet, gather_expanded_snippet, rust_context_hints, swift_context_hints,
};
use super::fuzzy::{rank_suggestions, tokenize};
use super::hits::{BodyPayload, HitOrigin, ProbeKind, SearchCache, SearchHit};
use super::query::{PatternMode, QueryRewriter};
use super::state::PersistentState;
use super::stats::{
    GitInfo, SearchMetrics, SearchSummary, StageStats, StartupStats, TopHit,
    aggregate_language_counts, compute_metrics, elapsed_ms, elapsed_std_ms, round_two,
};
use crate::cli::SymbolKind;
use crate::language::detect_language_from_path;

pub(crate) struct SearchEngine {
    pub(crate) config: SearchConfig,
    fd_tool: Option<FdTool>,
    rg_tool: RipgrepTool,
    rga_tool: Option<RgaTool>,
    ast_tool: Option<AstGrepTool>,
    fd_unavailable: bool,
    ast_unavailable: bool,
    #[cfg(feature = "indexing")]
    index: Option<TantivyIndex>,
    dedup_cache: SearchCache,
    state: PersistentState,
    reward_total: f32,
    startup_stats: StartupStats,
    file_cache: FileCache,
    language_cache: HashMap<PathBuf, &'static str>,
    body_cache: HashMap<PathBuf, BodyPayload>,
    warnings: Vec<String>,
}

impl SearchEngine {
    pub(crate) fn new(config: SearchConfig) -> Result<Self> {
        let init_start = StdInstant::now();
        let mut startup_stats = StartupStats::default();

        let fd_tool = None;

        let rg_start = StdInstant::now();
        let rg_tool = RipgrepTool::new(
            config.timeout,
            config.max_matches,
            config.context_before,
            config.context_after,
            config.max_columns,
            config.concurrency,
            config.ignore_case,
        );
        startup_stats.rg_ms = elapsed_std_ms(rg_start);

        let ast_tool = None;

        if config.use_index {
            let start = StdInstant::now();
            fs::create_dir_all(&config.index_dir).with_context(|| {
                format!(
                    "failed to create index directory {}",
                    config.index_dir.display()
                )
            })?;
            startup_stats.index_ms = elapsed_std_ms(start);
        }

        let state_start = StdInstant::now();
        let state = PersistentState::load(&config.root, &config.cache_dir)?;
        let state_elapsed = elapsed_std_ms(state_start);
        startup_stats.state_ms = state_elapsed;
        startup_stats.cache_ms = state_elapsed;

        let rga_tool = None;

        startup_stats.init_ms = elapsed_std_ms(init_start);
        crate::telemetry::record_stage_latency("init", startup_stats.init_ms);
        crate::telemetry::record_stage_latency("init_rg", startup_stats.rg_ms);
        crate::telemetry::record_stage_latency("init_cache", startup_stats.cache_ms);
        crate::telemetry::record_stage_latency("init_state", startup_stats.state_ms);
        crate::telemetry::record_stage_latency("init_index", startup_stats.index_ms);

        Ok(Self {
            config,
            fd_tool,
            rg_tool,
            rga_tool,
            ast_tool,
            fd_unavailable: false,
            ast_unavailable: false,
            #[cfg(feature = "indexing")]
            index: None,
            dedup_cache: SearchCache::default(),
            state,
            reward_total: 0.0,
            startup_stats,
            file_cache: FileCache::new(128),
            language_cache: HashMap::new(),
            body_cache: HashMap::new(),
            warnings: Vec::new(),
        })
    }

    pub(crate) async fn run_cycle(&mut self) -> Result<SearchSummary> {
        let mut stage_stats = StageStats::default();
        self.warnings.clear();

        tracing::info!(symbol = %self.config.symbol, "search_cycle_start");

        let rewrites = self.build_queries();
        if let Some(summary) = self.try_fast_path(&rewrites).await? {
            return Ok(summary);
        }

        let discover_start = Instant::now();
        let discover_candidates = self.discover().await;
        stage_stats.discover_ms = elapsed_ms(discover_start);
        stage_stats.discover_candidates = discover_candidates.len();
        stage_stats.record_discover_languages(&discover_candidates, stage_stats.discover_ms);
        let discover_set: HashSet<PathBuf> = discover_candidates.iter().cloned().collect();

        let probe_start = Instant::now();
        let (mut hits, scoped_hits_count) = self
            .probe(&rewrites, &discover_candidates, ProbeKind::Scoped)
            .await;
        stage_stats.probe_ms = elapsed_ms(probe_start);
        stage_stats.probe_hits = scoped_hits_count;
        stage_stats.record_probe_languages(&hits, stage_stats.probe_ms);

        if hits.is_empty() {
            let escalate_start = Instant::now();
            let (global_hits, global_hits_count) =
                self.probe(&rewrites, &[], ProbeKind::Global).await;
            stage_stats.escalate_ms = elapsed_ms(escalate_start);
            stage_stats.escalate_hits = global_hits_count;
            stage_stats.record_escalate_languages(&global_hits, stage_stats.escalate_ms);
            hits = global_hits;
        }

        #[cfg(feature = "indexing")]
        if hits.is_empty() && self.config.use_index {
            let index_stage_start = Instant::now();
            let symbol = self.config.symbol.clone();
            let max_matches = self.config.max_matches;
            match self.ensure_index().await {
                Ok(index) => {
                    crate::telemetry::record_tool_invocation("index");
                    match index.search(&symbol, max_matches).await {
                        Ok(candidates) => {
                            stage_stats.index_candidates = candidates.len();
                            crate::telemetry::record_tool_results("index", candidates.len());
                            if !candidates.is_empty() {
                                let (indexed_hits, indexed_count) =
                                    self.probe(&rewrites, &candidates, ProbeKind::Indexed).await;
                                stage_stats.index_probe_hits = indexed_count;
                                hits.extend(indexed_hits);
                            }
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "tantivy search failed");
                            self.push_warning(format!("index search failed: {err}"));
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "failed to initialize index");
                    self.push_warning(format!("index initialization failed: {err}"));
                }
            }
            stage_stats.index_ms = elapsed_ms(index_stage_start);
        }

        if hits.is_empty() {
            let root_clone = self.config.root.clone();
            let symbol_clone = self.config.symbol.clone();
            if let Some(rga_tool) = self.ensure_rga_tool() {
                let rga_start = Instant::now();
                crate::telemetry::record_tool_invocation("rga");
                match rga_tool.search(&root_clone, symbol_clone.as_str()).await {
                    Ok(matches) => {
                        stage_stats.rga_hits = matches.len();
                        crate::telemetry::record_tool_results("rga", matches.len());
                        for m in matches {
                            hits.push(SearchHit::from_rga(&self.config.root, m));
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "rga search failed");
                        self.push_warning(format!("rga search failed: {err}"));
                    }
                }
                stage_stats.rga_ms = elapsed_ms(rga_start);
            }
        }

        let disambiguate_start = Instant::now();
        let ast_scope: Vec<PathBuf> = hits
            .iter()
            .map(|hit| hit.path.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let ast_matches = self.disambiguate(&ast_scope).await;
        stage_stats.disambiguate_ms = elapsed_ms(disambiguate_start);
        stage_stats.ast_matches = ast_matches.len();
        stage_stats.record_disambiguate_languages(&ast_matches, stage_stats.disambiguate_ms);

        let verify_start = Instant::now();
        let verification = self
            .verify(hits, ast_matches, discover_set, discover_candidates.clone())
            .await?;
        stage_stats.verify_ms = elapsed_ms(verify_start);
        stage_stats.record_verify_languages(&verification.language_counts, stage_stats.verify_ms);

        stage_stats.precision = round_two(verification.metrics.precision);
        stage_stats.density = round_two(verification.metrics.density);
        stage_stats.clustering = round_two(verification.metrics.cluster_score);
        stage_stats.reward = round_two(verification.metrics.reward);
        stage_stats.cycle_latency_ms = stage_stats.discover_ms
            + stage_stats.probe_ms
            + stage_stats.escalate_ms
            + stage_stats.index_ms
            + stage_stats.rga_ms
            + stage_stats.disambiguate_ms
            + stage_stats.verify_ms;

        self.reward_total += verification.metrics.reward;

        if let Err(err) = self.state.save() {
            tracing::warn!(error = %err, "failed to persist cache state");
        }

        crate::telemetry::record_stage_latency("discover", stage_stats.discover_ms);
        crate::telemetry::record_stage_latency("probe", stage_stats.probe_ms);
        crate::telemetry::record_stage_latency("escalate", stage_stats.escalate_ms);
        crate::telemetry::record_stage_latency("index", stage_stats.index_ms);
        crate::telemetry::record_stage_latency("rga", stage_stats.rga_ms);
        crate::telemetry::record_stage_latency("ast", stage_stats.disambiguate_ms);
        crate::telemetry::record_stage_latency("verify", stage_stats.verify_ms);

        let (related_symbols, related_paths, suggested_symbols, notes) = self
            .suggest_alternatives(&discover_candidates, &verification, &rewrites)
            .await;

        let summary = SearchSummary {
            cycle: 1,
            root: self.config.root.clone(),
            symbol: self.config.symbol.clone(),
            concept: self.config.concept.clone(),
            queries: rewrites,
            top_hits: verification.top_hits,
            deduped: verification.dedup_count,
            max_matches: self.config.max_matches,
            next_actions: verification.next_actions,
            related_symbols,
            suggested_symbols,
            related_paths,
            notes,
            fd_candidates: verification.fd_candidates,
            ast_hits: verification.ast_hits,
            startup_stats: Some(self.startup_stats.clone()),
            stage_stats,
            reward: round_two(self.reward_total),
            warnings: self.warnings.clone(),
            truncated: verification.dedup_count >= self.config.max_matches,
        };

        crate::telemetry::record_reward(verification.metrics.reward);
        crate::telemetry::record_cycle_latency(summary.stage_stats.cycle_latency_ms);
        crate::telemetry::record_stage_latency("cycle", summary.stage_stats.cycle_latency_ms);

        let top_hit_path = summary.top_hits.first().map(|hit| hit.path.clone());
        tracing::info!(
            symbol = %summary.symbol,
            latency_ms = summary.stage_stats.cycle_latency_ms,
            reward = summary.reward,
            deduped = summary.deduped,
            top_hit = ?top_hit_path,
            "search_cycle_complete"
        );

        self.log_summary(&summary).await?;

        Ok(summary)
    }

    fn build_queries(&self) -> Vec<String> {
        if let Some(concept) = &self.config.concept {
            return QueryRewriter::for_concept(
                concept,
                &self.config.language_tokens,
                self.config.ignore_case,
            )
            .build();
        }
        QueryRewriter::for_symbol(
            &self.config.symbol,
            &self.config.language_tokens,
            self.config.pattern_mode,
            self.config.ignore_case,
        )
        .build()
    }

    async fn try_fast_path(&mut self, rewrites: &[String]) -> Result<Option<SearchSummary>> {
        if !self.is_literal_symbol() {
            return Ok(None);
        }

        crate::telemetry::record_tool_invocation("rg");
        let probe_start = Instant::now();
        let matches = match self
            .rg_tool
            .search_union(&self.config.root, rewrites, &[])
            .await
        {
            Ok(matches) => matches,
            Err(err) => {
                self.push_warning(format!("fast-path ripgrep failed: {err}"));
                tracing::warn!(error = %err, "fast-path ripgrep failed");
                return Ok(None);
            }
        };
        crate::telemetry::record_tool_results("rg", matches.len());

        if matches.is_empty() {
            return Ok(None);
        }

        let probe_ms = elapsed_ms(probe_start);
        let mut hits: Vec<SearchHit> = matches
            .into_iter()
            .map(|m| {
                SearchHit::from_ripgrep(
                    &self.config.root,
                    m,
                    ProbeKind::Global,
                    self.config.max_columns,
                )
            })
            .filter(|hit| self.matches_path_globs(&hit.path))
            .collect();
        let total_hits = hits.len();
        let probe_hits_snapshot = hits.clone();

        let verify_start = Instant::now();
        let verification = self
            .verify(
                std::mem::take(&mut hits),
                Vec::new(),
                HashSet::new(),
                Vec::new(),
            )
            .await?;
        let verify_ms = elapsed_ms(verify_start);

        let mut stage_stats = StageStats::default();
        stage_stats.probe_ms = probe_ms;
        stage_stats.probe_hits = total_hits;
        stage_stats.record_probe_languages(&probe_hits_snapshot, stage_stats.probe_ms);
        stage_stats.verify_ms = verify_ms;
        stage_stats.cycle_latency_ms = probe_ms + verify_ms;
        stage_stats.precision = round_two(verification.metrics.precision);
        stage_stats.density = round_two(verification.metrics.density);
        stage_stats.clustering = round_two(verification.metrics.cluster_score);
        stage_stats.reward = round_two(verification.metrics.reward);
        stage_stats.record_verify_languages(&verification.language_counts, stage_stats.verify_ms);

        self.reward_total += verification.metrics.reward;

        if let Err(err) = self.state.save() {
            tracing::warn!(error = %err, "failed to persist cache state");
        }

        crate::telemetry::record_stage_latency("probe", stage_stats.probe_ms);
        crate::telemetry::record_stage_latency("verify", stage_stats.verify_ms);

        let summary = SearchSummary {
            cycle: 1,
            root: self.config.root.clone(),
            symbol: self.config.symbol.clone(),
            concept: self.config.concept.clone(),
            queries: rewrites.to_vec(),
            top_hits: verification.top_hits,
            deduped: verification.dedup_count,
            max_matches: self.config.max_matches,
            next_actions: verification.next_actions,
            related_symbols: Vec::new(),
            suggested_symbols: Vec::new(),
            related_paths: Vec::new(),
            notes: Vec::new(),
            fd_candidates: Vec::new(),
            ast_hits: Vec::new(),
            startup_stats: Some(self.startup_stats.clone()),
            stage_stats,
            reward: round_two(self.reward_total),
            warnings: self.warnings.clone(),
            truncated: verification.dedup_count >= self.config.max_matches,
        };

        crate::telemetry::record_reward(verification.metrics.reward);
        crate::telemetry::record_cycle_latency(summary.stage_stats.cycle_latency_ms);
        crate::telemetry::record_stage_latency("cycle", summary.stage_stats.cycle_latency_ms);

        let top_hit_path = summary.top_hits.first().map(|hit| hit.path.clone());
        tracing::info!(
            symbol = %summary.symbol,
            latency_ms = summary.stage_stats.cycle_latency_ms,
            reward = summary.reward,
            deduped = summary.deduped,
            top_hit = ?top_hit_path,
            "search_cycle_complete"
        );

        self.log_summary(&summary).await?;

        Ok(Some(summary))
    }

    fn fetch_body(&mut self, path: &Path) -> BodyPayload {
        if let Some(cached) = self.body_cache.get(path) {
            return cached.clone();
        }

        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.config.root.join(path)
        };

        let payload = match fs::metadata(&absolute) {
            Ok(metadata) => {
                if metadata.len() as usize > self.config.max_body_bytes {
                    BodyPayload {
                        body: None,
                        retrieved: false,
                    }
                } else {
                    match self
                        .file_cache
                        .read_file(&absolute)
                        .map(|f| f.contents().to_string())
                    {
                        Some(contents) => BodyPayload {
                            body: Some(contents),
                            retrieved: true,
                        },
                        None => BodyPayload {
                            body: None,
                            retrieved: false,
                        },
                    }
                }
            }
            Err(_) => BodyPayload {
                body: None,
                retrieved: false,
            },
        };

        self.body_cache.insert(path.to_path_buf(), payload.clone());
        payload
    }

    fn compute_git_info(&self, path: &Path) -> Option<GitInfo> {
        if !self.config.git_info {
            return None;
        }

        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.config.root.join(path)
        };
        let relative = absolute
            .strip_prefix(&self.config.root)
            .unwrap_or(&absolute)
            .to_path_buf();

        let output = Command::new("git")
            .arg("-C")
            .arg(&self.config.root)
            .arg("log")
            .arg("-n")
            .arg("1")
            .arg("--format=%h\t%cs")
            .arg("--")
            .arg(&relative)
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut parts = stdout.lines().next().unwrap_or("").split('\t');
        let commit = parts.next().unwrap_or("").trim();
        let date = parts.next().unwrap_or("").trim();
        if commit.is_empty() || date.is_empty() {
            return None;
        }

        Some(GitInfo {
            last_commit: commit.to_string(),
            last_modified: date.to_string(),
        })
    }

    fn should_attach_body(&self, path: &Path) -> bool {
        if self.config.body {
            return true;
        }
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
        {
            Some(ref ext) if ext == "rs" || ext == "swift" => true,
            _ => false,
        }
    }

    fn compute_context_hints(&self, path: &Path, line: usize) -> Vec<ContextHint> {
        if line == 0 {
            return Vec::new();
        }

        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.config.root.join(path)
        };

        let Some(contents) = self
            .file_cache
            .read_file(&absolute)
            .map(|f| f.contents().to_string())
        else {
            return Vec::new();
        };

        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());

        match extension.as_deref() {
            Some("rs") => rust_context_hints(&contents, line),
            Some("swift") => swift_context_hints(&contents, line),
            _ => Vec::new(),
        }
    }

    fn is_definition_hit(&self, hit: &SearchHit, hints: &[ContextHint]) -> bool {
        if hints.iter().any(|hint| {
            matches!(
                hint.kind.as_str(),
                "declaration" | "impl" | "type" | "extension" | "module"
            )
        }) {
            return true;
        }

        let snippet_lower = hit.snippet.to_ascii_lowercase();
        let symbol_lower = self.config.symbol.to_ascii_lowercase();
        if snippet_lower.is_empty() || symbol_lower.is_empty() {
            return false;
        }

        let definition_markers = [
            format!("fn {symbol_lower}"),
            format!("struct {symbol_lower}"),
            format!("enum {symbol_lower}"),
            format!("trait {symbol_lower}"),
            format!("impl {symbol_lower}"),
            format!("type {symbol_lower}"),
            format!("const {symbol_lower}"),
            format!("static {symbol_lower}"),
            format!("class {symbol_lower}"),
            format!("interface {symbol_lower}"),
            format!("function {symbol_lower}"),
            format!("protocol {symbol_lower}"),
            format!("extension {symbol_lower}"),
            format!("actor {symbol_lower}"),
            format!("mod {symbol_lower}"),
            format!("let {symbol_lower}"),
            format!("var {symbol_lower}"),
            format!("case {symbol_lower}"),
        ];

        definition_markers
            .iter()
            .any(|pattern| snippet_lower.contains(pattern))
    }

    fn ensure_fd_tool(&mut self) -> Option<&FdTool> {
        if !self.config.use_fd || self.fd_unavailable {
            return None;
        }
        if self.fd_tool.is_none() {
            let start = StdInstant::now();
            let tool = FdTool::new(self.config.timeout, 200);
            let elapsed = elapsed_std_ms(start);
            if self.startup_stats.fd_ms == 0 {
                self.startup_stats.fd_ms = elapsed;
                crate::telemetry::record_stage_latency("init_fd", elapsed);
            }
            self.fd_tool = Some(tool);
        }
        self.fd_tool.as_ref()
    }

    fn ensure_ast_tool(&mut self) -> Option<&AstGrepTool> {
        if !self.config.use_ast || self.ast_unavailable {
            return None;
        }
        if self.ast_tool.is_none() {
            let start = StdInstant::now();
            let tool = AstGrepTool::new(self.config.timeout, self.config.max_matches);
            let elapsed = elapsed_std_ms(start);
            if self.startup_stats.ast_ms == 0 {
                self.startup_stats.ast_ms = elapsed;
                crate::telemetry::record_stage_latency("init_ast", elapsed);
            }
            self.ast_tool = Some(tool);
        }
        self.ast_tool.as_ref()
    }

    fn ensure_rga_tool(&mut self) -> Option<&RgaTool> {
        if !self.config.use_rga {
            return None;
        }
        if self.rga_tool.is_none() {
            let start = StdInstant::now();
            let tool = RgaTool::new(self.config.timeout, self.config.max_matches);
            let elapsed = elapsed_std_ms(start);
            if self.startup_stats.rga_ms == 0 {
                self.startup_stats.rga_ms = elapsed;
                crate::telemetry::record_stage_latency("init_rga", elapsed);
            }
            self.rga_tool = Some(tool);
        }
        self.rga_tool.as_ref()
    }

    fn push_warning(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    fn is_not_found_error(err: &anyhow::Error) -> bool {
        err.chain().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .map(|io| io.kind() == ErrorKind::NotFound)
                .unwrap_or(false)
        })
    }

    async fn suggest_alternatives(
        &self,
        discover_candidates: &[PathBuf],
        verification: &VerificationOutcome,
        rewrites: &[String],
    ) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
        if !verification.top_hits.is_empty() {
            return (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        }

        let target = if let Some(concept) = &self.config.concept {
            concept
        } else {
            &self.config.symbol
        };
        let target_lower = target.to_ascii_lowercase();

        let mut candidates: Vec<String> = Vec::new();
        candidates.extend(self.state.known_symbols());
        candidates.extend(collect_candidate_symbols(discover_candidates));
        candidates.extend(collect_candidate_symbols(&verification.fd_candidates));
        let ast_paths: Vec<PathBuf> = verification
            .ast_hits
            .iter()
            .map(|(p, _)| p.clone())
            .collect();
        candidates.extend(collect_candidate_symbols(&ast_paths));

        if let Some(prefix) = self.fuzzy_prefix(target) {
            let mut fuzzy = self.fuzzy_candidates(&prefix).await;
            candidates.append(&mut fuzzy);
        }

        let mut related_paths: Vec<String> = Vec::new();
        let mut related_symbols: Vec<String> = Vec::new();
        for path in discover_candidates {
            if let Some(text) = path.to_str() {
                let lower = text.to_ascii_lowercase();
                if lower.contains(&target_lower) || contains_synonym(&lower, &target_lower) {
                    related_paths.push(text.to_string());
                }
            }
        }
        let mut seen_paths: HashSet<String> = HashSet::new();
        related_paths.retain(|p| seen_paths.insert(p.clone()));

        let ranked = rank_suggestions(target, candidates, 5);
        for candidate in &ranked {
            let lower = candidate.to_ascii_lowercase();
            if lower.contains(&target_lower) || contains_synonym(&lower, &target_lower) {
                related_symbols.push(candidate.clone());
            }
        }

        let mut notes = Vec::new();
        if self.config.concept.is_some() {
            notes.push("semantic/concept search enabled".to_string());
        }
        if !matches!(self.config.pattern_mode, PatternMode::Literal) {
            notes.push("regex/wildcard mode disables AST disambiguation".to_string());
        }
        if self.config.include_strings {
            notes.push("search includes localization resources".to_string());
        }
        if rewrites.is_empty() {
            notes.push("no queries generated for provided input".to_string());
        }

        (related_symbols, related_paths, ranked, notes)
    }

    fn fuzzy_prefix(&self, target: &str) -> Option<String> {
        let cleaned: String = target
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let prefix: String = cleaned.chars().take(3).collect();
        if prefix.len() >= 2 {
            Some(prefix)
        } else {
            None
        }
    }

    async fn fuzzy_candidates(&self, prefix: &str) -> Vec<String> {
        let queries = vec![prefix.to_string()];
        match self
            .rg_tool
            .search_union(&self.config.root, &queries, &[])
            .await
        {
            Ok(matches) => {
                let mut tokens = Vec::new();
                for m in matches {
                    tokens.extend(tokenize(&m.lines));
                    if let Some(stem) = m.path.file_stem().and_then(|s| s.to_str()) {
                        tokens.push(stem.to_string());
                    }
                    if let Some(parent) = m
                        .path
                        .parent()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                    {
                        tokens.push(parent.to_string());
                    }
                }
                tokens
            }
            Err(err) => {
                tracing::debug!(error = %err, "fuzzy ripgrep probe failed");
                Vec::new()
            }
        }
    }

    fn matches_kind(&self, kind: SymbolKind, hit: &SearchHit, hints: &[ContextHint]) -> bool {
        let lang = detect_language_from_path(&hit.path).unwrap_or("other");
        let mut labels: Vec<String> = hints.iter().map(|hint| hint.label.clone()).collect();
        labels.push(hit.snippet.clone());
        labels
            .iter()
            .any(|label| label_matches_kind(kind, label, lang))
    }

    fn format_origin_label(&mut self, origin: &HitOrigin, path: &Path) -> String {
        let tool = origin.as_str();
        if let Some(lang) = self.language_cache.get(path) {
            return format!("{tool} [{lang}]");
        }
        if let Some(lang) = detect_language_from_path(path) {
            self.language_cache.insert(path.to_path_buf(), lang);
            format!("{tool} [{lang}]")
        } else {
            tool.to_string()
        }
    }

    fn is_literal_symbol(&self) -> bool {
        if self.config.concept.is_some() {
            return false;
        }
        if !matches!(self.config.pattern_mode, PatternMode::Literal) {
            return false;
        }
        let s = self.config.symbol.trim();
        !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    fn should_run_ast(&self) -> bool {
        if !self.config.use_ast {
            return false;
        }
        if self.ast_unavailable {
            return false;
        }
        if self.config.concept.is_some() {
            return false;
        }
        if !matches!(self.config.pattern_mode, PatternMode::Literal) {
            return false;
        }
        let s = self.config.symbol.trim();
        if s.is_empty() {
            return false;
        }
        s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$'))
    }

    fn matches_path_globs(&self, path: &Path) -> bool {
        if let Some(globs) = &self.config.path_globs {
            let rel = path.to_string_lossy().replace('\\', "/");
            let abs_path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                self.config.root.join(path)
            };
            let abs = abs_path.to_string_lossy().replace('\\', "/");
            globs.is_match(rel.as_str()) || globs.is_match(abs.as_str())
        } else {
            true
        }
    }

    fn context_padding(&self, hit: &SearchHit) -> (usize, usize, bool) {
        let mut before = self.config.context_before;
        let mut after = self.config.context_after;
        let mut auto_expanded = false;

        if before == 0 && after == 0 {
            before = DEFAULT_INLINE_CONTEXT;
            after = DEFAULT_INLINE_CONTEXT;
            auto_expanded = true;
        }

        if hit.raw_snippet_truncated {
            let adjusted_before = before.max(TRUNCATED_INLINE_CONTEXT);
            let adjusted_after = after.max(TRUNCATED_INLINE_CONTEXT);
            if adjusted_before != before || adjusted_after != after {
                auto_expanded = true;
                before = adjusted_before;
                after = adjusted_after;
            }
        }

        (before, after, auto_expanded)
    }

    async fn log_summary(&self, summary: &SearchSummary) -> Result<()> {
        let Some(dir) = &self.config.log_dir else {
            return Ok(());
        };

        tokio::fs::create_dir_all(dir)
            .await
            .with_context(|| format!("failed to create log directory {}", dir.display()))?;

        let log_path = dir.join("search.log.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await
            .with_context(|| format!("failed to open log file {}", log_path.display()))?;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        let entry = json!({
            "timestamp": timestamp,
            "root": self.config.root,
            "symbol": self.config.symbol,
            "use_index": self.config.use_index,
            "use_rga": self.config.use_rga,
            "use_fd": self.config.use_fd,
            "use_ast_grep": self.config.use_ast,
            "status": "ok",
            "latency_ms": summary.stage_stats.cycle_latency_ms,
            "summary": summary,
        });

        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        file.write_all(&line).await?;

        Ok(())
    }

    async fn discover(&mut self) -> Vec<PathBuf> {
        let root = self.config.root.clone();
        let symbol = self.config.symbol.clone();
        let extension_filters =
            extensions_for_languages(&self.config.language_tokens, self.config.include_strings);
        let extensions = extension_filters.as_deref();
        let mut candidates: Vec<PathBuf> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        let fd_results = if let Some(fd_tool) = self.ensure_fd_tool() {
            crate::telemetry::record_tool_invocation("fd");
            match fd_tool.run(&root, symbol.as_str()).await {
                Ok(results) => results,
                Err(err) => {
                    if Self::is_not_found_error(&err) {
                        self.fd_unavailable = true;
                        tracing::debug!(error = %err, "fd not found on PATH; disabling fd hints");
                    } else {
                        self.push_warning(format!("fd invocation failed: {err}"));
                        tracing::warn!(error = %err, "fd invocation failed");
                    }
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        crate::telemetry::record_tool_results("fd", fd_results.len());

        for path in fd_results {
            if let Ok(normalized) = normalize_path(&root, &path) {
                if passes_extension_filter(&normalized, extensions)
                    && self.matches_path_globs(&normalized)
                    && seen.insert(normalized.clone())
                {
                    candidates.push(normalized);
                }
            }
        }

        let symbol_hints = self.state.hints_for_symbol(&self.config.symbol);
        crate::telemetry::record_cache_hits("symbol_hints", symbol_hints.len());
        for hint in symbol_hints {
            if passes_extension_filter(&hint, extensions)
                && self.matches_path_globs(&hint)
                && seen.insert(hint.clone())
            {
                candidates.push(hint);
            }
        }

        let directory_hints = self.state.top_directories(3);
        crate::telemetry::record_cache_hits("directory_hints", directory_hints.len());
        for dir in directory_hints {
            let dir_path = root.join(&dir);
            if !dir_path.is_dir() {
                continue;
            }
            match fs::read_dir(&dir_path) {
                Ok(entries) => {
                    for entry in entries.flatten().take(5) {
                        let path = entry.path();
                        if path.is_file() {
                            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                if name.starts_with('.') {
                                    continue;
                                }
                            }
                            if let Ok(normalized) = normalize_path(&root, &path) {
                                if passes_extension_filter(&normalized, extensions)
                                    && self.matches_path_globs(&normalized)
                                    && seen.insert(normalized.clone())
                                {
                                    candidates.push(normalized);
                                }
                            }
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        path = %dir_path.display(),
                        error = %err,
                        "failed to read cached directory"
                    );
                }
            }
        }

        if languages_include(&self.config.language_tokens, "swift") {
            let mut swift_hints: Vec<PathBuf> = Vec::new();
            let package_manifest = root.join("Package.swift");
            if package_manifest.is_file() {
                swift_hints.push(package_manifest);
            }
            if let Ok(entries) = fs::read_dir(&root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.eq_ignore_ascii_case("sources") && path.is_dir() {
                            if let Ok(children) = fs::read_dir(&path) {
                                for child in children.flatten().take(20) {
                                    let child_path = child.path();
                                    if child_path.is_file() {
                                        swift_hints.push(child_path);
                                    } else if child_path.is_dir() {
                                        if let Ok(grandchildren) = fs::read_dir(&child_path) {
                                            for file in grandchildren.flatten().take(10) {
                                                let file_path = file.path();
                                                if file_path.is_file() {
                                                    swift_hints.push(file_path);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            for hint in swift_hints {
                if let Ok(normalized) = normalize_path(&root, &hint) {
                    if passes_extension_filter(&normalized, extensions)
                        && self.matches_path_globs(&normalized)
                        && seen.insert(normalized.clone())
                    {
                        candidates.push(normalized);
                    }
                }
            }
        }

        candidates
    }

    async fn probe(
        &mut self,
        rewrites: &[String],
        scope: &[PathBuf],
        kind: ProbeKind,
    ) -> (Vec<SearchHit>, usize) {
        if rewrites.is_empty() {
            return (Vec::new(), 0);
        }

        crate::telemetry::record_tool_invocation("rg");
        match self
            .rg_tool
            .search_union(&self.config.root, rewrites, scope)
            .await
        {
            Ok(matches) => {
                crate::telemetry::record_tool_results("rg", matches.len());
                let hits = matches
                    .into_iter()
                    .map(|m| {
                        SearchHit::from_ripgrep(
                            &self.config.root,
                            m,
                            kind.clone(),
                            self.config.max_columns,
                        )
                    })
                    .filter(|hit| self.matches_path_globs(&hit.path))
                    .collect::<Vec<_>>();
                let hit_count = hits.len();
                (hits, hit_count)
            }
            Err(err) => {
                self.push_warning(format!("ripgrep invocation failed: {err}"));
                tracing::warn!(error = %err, "ripgrep invocation failed");
                (Vec::new(), 0)
            }
        }
    }

    async fn disambiguate(&mut self, scope: &[PathBuf]) -> Vec<AstGrepMatch> {
        if !self.should_run_ast() {
            return Vec::new();
        }

        let root = self.config.root.clone();
        let symbol = self.config.symbol.clone();
        let language_tokens = self.config.language_tokens.clone();
        let Some(ast_tool) = self.ensure_ast_tool() else {
            return Vec::new();
        };

        crate::telemetry::record_tool_invocation("ast-grep");

        ast_tool
            .search_identifier(&root, symbol.as_str(), &language_tokens, scope)
            .await
            .map(|matches| {
                crate::telemetry::record_tool_results("ast-grep", matches.len());
                matches
            })
            .unwrap_or_else(|err| {
                if Self::is_not_found_error(&err) {
                    self.ast_unavailable = true;
                    tracing::debug!(
                        error = %err,
                        "ast-grep not found on PATH; disabling AST disambiguation"
                    );
                } else if let Some(pattern_err) = err.downcast_ref::<AstPatternError>() {
                    tracing::debug!(
                        symbol = %symbol,
                        pattern = %pattern_err.pattern(),
                        "ast-grep pattern invalid; falling back to literal search"
                    );
                    self.push_warning(format!(
                        "ast-grep pattern invalid for `{symbol}`: {}",
                        pattern_err.message()
                    ));
                } else {
                    tracing::warn!(error = %err, "ast-grep invocation failed");
                    self.push_warning(format!("ast-grep invocation failed: {err}"));
                }
                Vec::new()
            })
    }

    #[cfg(feature = "indexing")]
    async fn ensure_index(&mut self) -> Result<&TantivyIndex> {
        if self.index.is_none() {
            let extensions =
                extensions_for_languages(&self.config.language_tokens, self.config.include_strings)
                    .map(|exts| exts.into_iter().map(|s| s.to_string()).collect::<Vec<_>>());
            let index_config = IndexConfig {
                root: self.config.root.clone(),
                index_dir: self.config.index_dir.clone(),
                extensions,
            };
            let built = TantivyIndex::open_or_build(index_config).await?;
            self.index = Some(built);
        }
        Ok(self.index.as_ref().expect("index initialized"))
    }

    async fn verify(
        &mut self,
        hits: Vec<SearchHit>,
        ast_matches: Vec<AstGrepMatch>,
        fd_set: HashSet<PathBuf>,
        fd_candidates: Vec<PathBuf>,
    ) -> Result<VerificationOutcome> {
        let ast_set: HashSet<(PathBuf, usize)> = ast_matches
            .iter()
            .filter_map(|m| {
                normalize_path(&self.config.root, &m.path)
                    .ok()
                    .map(|path| (path, m.line.saturating_add(1)))
            })
            .collect();

        let mut dedup: HashMap<(PathBuf, usize), SearchHit> = HashMap::new();
        for mut hit in hits {
            let key = (hit.path.clone(), hit.line);
            if fd_set.contains(&hit.path) {
                hit.score += 0.2;
            }
            match &hit.origin {
                HitOrigin::Ripgrep(ProbeKind::Global) => hit.score -= 0.05,
                #[cfg(feature = "indexing")]
                HitOrigin::Ripgrep(ProbeKind::Indexed) => hit.score += 0.1,
                HitOrigin::Rga => hit.score -= 0.1,
                _ => {}
            }
            if ast_set.contains(&key) {
                hit.score += 0.5;
                hit.origin = HitOrigin::AstGrep;
            }

            dedup
                .entry(key)
                .and_modify(|existing| {
                    if hit.score > existing.score {
                        *existing = hit.clone();
                    }
                })
                .or_insert(hit);
        }

        let mut dedup_hits: Vec<SearchHit> = dedup.into_values().collect();
        dedup_hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.line.cmp(&b.line))
        });

        self.dedup_cache.retain_new(&mut dedup_hits);

        let mut hint_cache: HashMap<(PathBuf, usize), Vec<ContextHint>> = HashMap::new();
        if self.config.definitions_only || self.config.usages_only {
            dedup_hits.retain(|hit| {
                let hints = self.compute_context_hints(&hit.path, hit.line);
                hint_cache.insert((hit.path.clone(), hit.line), hints.clone());
                let is_definition = self.is_definition_hit(hit, &hints);
                if self.config.definitions_only {
                    is_definition
                } else {
                    !is_definition
                }
            });
        }

        if let Some(kind) = self.config.kind {
            dedup_hits.retain(|hit| {
                let hints = hint_cache
                    .get(&(hit.path.clone(), hit.line))
                    .cloned()
                    .unwrap_or_else(|| self.compute_context_hints(&hit.path, hit.line));
                hint_cache.insert((hit.path.clone(), hit.line), hints.clone());
                self.matches_kind(kind, hit, &hints)
            });
        }

        self.state.observe(&self.config.symbol, &dedup_hits);

        let top_hits: Vec<TopHit> = dedup_hits
            .iter()
            .take(5)
            .map(|hit| {
                let formatted_snippet = format_snippet(
                    &self.config.root,
                    &self.file_cache,
                    &hit.path,
                    hit.line,
                    &hit.snippet,
                );
                let (context_before, context_after, auto_expanded_context) =
                    self.context_padding(hit);
                let context_window = gather_expanded_snippet(
                    &self.config.root,
                    &self.file_cache,
                    &hit.path,
                    hit.line,
                    context_before,
                    context_after,
                );
                let (expanded_snippet, context_start, context_end, auto_context_flag) =
                    match context_window {
                        Some((snippet, start, end)) => {
                            (Some(snippet), Some(start), Some(end), auto_expanded_context)
                        }
                        None => (None, None, None, false),
                    };

                let include_body = self.should_attach_body(&hit.path);
                let (body, body_retrieved) = if include_body {
                    let payload = self.fetch_body(&hit.path);
                    (payload.body, payload.retrieved)
                } else {
                    (None, false)
                };

                let hints = hint_cache
                    .get(&(hit.path.clone(), hit.line))
                    .cloned()
                    .unwrap_or_else(|| self.compute_context_hints(&hit.path, hit.line));
                let git_info = self.compute_git_info(&hit.path);

                TopHit {
                    path: hit.path.to_string_lossy().to_string(),
                    line: hit.line,
                    score: round_two(hit.score),
                    origin: hit.origin.as_str().to_string(),
                    origin_label: self.format_origin_label(&hit.origin, &hit.path),
                    snippet: formatted_snippet,
                    raw_snippet: hit.raw_snippet.clone(),
                    snippet_length: Some(hit.snippet_length),
                    raw_snippet_truncated: hit.raw_snippet_truncated,
                    expanded_snippet,
                    context_start,
                    context_end,
                    auto_expanded_context: auto_context_flag,
                    body,
                    body_retrieved,
                    hints,
                    git_info,
                }
            })
            .collect();

        let next_actions: Vec<String> = top_hits
            .iter()
            .map(|hit| format!("inspect {}:{}", hit.path, hit.line))
            .collect();

        let metrics = compute_metrics(&dedup_hits, &ast_set, fd_set.len());

        let language_counts =
            aggregate_language_counts(dedup_hits.iter().map(|hit| hit.path.as_path()));

        Ok(VerificationOutcome {
            top_hits,
            next_actions,
            dedup_count: dedup_hits.len(),
            fd_candidates,
            ast_hits: ast_matches
                .into_iter()
                .filter_map(|a| {
                    normalize_path(&self.config.root, &a.path)
                        .ok()
                        .map(|path| (path, a.line + 1))
                })
                .collect(),
            metrics,
            language_counts,
        })
    }
}

struct VerificationOutcome {
    top_hits: Vec<TopHit>,
    next_actions: Vec<String>,
    dedup_count: usize,
    fd_candidates: Vec<PathBuf>,
    ast_hits: Vec<(PathBuf, usize)>,
    metrics: SearchMetrics,
    language_counts: BTreeMap<String, usize>,
}

fn collect_candidate_symbols(paths: &[PathBuf]) -> Vec<String> {
    let mut results: Vec<String> = Vec::new();
    for path in paths {
        if let Some(stem) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
        {
            results.push(stem.to_string());
            results.extend(split_tokens(stem));
        }
        if let Some(dir) = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
        {
            results.push(dir.to_string());
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    results
        .into_iter()
        .filter(|item| seen.insert(item.to_ascii_lowercase()))
        .collect()
}

fn contains_synonym(candidate_lower: &str, target_lower: &str) -> bool {
    let mut tokens = split_tokens(target_lower);
    tokens.push(target_lower.to_string());
    for token in tokens {
        for synonym in synonyms_for(&token) {
            if candidate_lower.contains(synonym) {
                return true;
            }
        }
    }
    false
}

fn label_matches_kind(kind: SymbolKind, label: &str, language: &str) -> bool {
    let lower = label.to_ascii_lowercase();
    match kind {
        SymbolKind::Func => {
            lower.starts_with("fn ")
                || lower.contains(" fn ")
                || lower.starts_with("func ")
                || lower.contains(" func ")
                || lower.contains(" function ")
                || lower.starts_with("function ")
                || lower.starts_with("init")
                || lower.contains("->")
                || lower.contains("=>")
        }
        SymbolKind::Struct => lower.contains("struct "),
        SymbolKind::Class => lower.contains("class "),
        SymbolKind::Enum => lower.contains("enum "),
        SymbolKind::Trait => lower.contains("trait "),
        SymbolKind::Protocol => lower.contains("protocol "),
        SymbolKind::Interface => lower.contains("interface "),
        SymbolKind::Const => {
            lower.starts_with("const ")
                || lower.contains(" const ")
                || lower.starts_with("static ")
                || lower.contains(" static ")
                || (language.eq_ignore_ascii_case("swift")
                    && (lower.starts_with("let ") || lower.starts_with("var ")))
        }
        SymbolKind::Actor => lower.contains("actor "),
    }
}

fn split_tokens(raw: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() && !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            tokens.push(current.clone());
            current.clear();
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn synonyms_for(token: &str) -> Vec<&'static str> {
    match token.to_ascii_lowercase().as_str() {
        "speak" | "speech" | "voice" => vec!["speech", "voice", "say", "aloud", "talk"],
        "text" => vec!["string", "copy"],
        "delete" | "remove" => vec!["cleanup", "prune"],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::label_matches_kind;
    use crate::cli::SymbolKind;

    #[test]
    fn detects_function_kind() {
        assert!(label_matches_kind(
            SymbolKind::Func,
            "fn run_cycle(args: SearchArgs)",
            "rust"
        ));
    }

    #[test]
    fn detects_type_kinds() {
        assert!(label_matches_kind(
            SymbolKind::Struct,
            "struct SearchSummary {",
            "rust"
        ));
        assert!(label_matches_kind(
            SymbolKind::Class,
            "class NetworkClient {",
            "swift"
        ));
    }
}
