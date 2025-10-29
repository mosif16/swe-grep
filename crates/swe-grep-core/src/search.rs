use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant as StdInstant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{self, json};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::time::Instant;

use crate::cli::SearchArgs;
use crate::tools::ast_grep::{AstGrepMatch, AstGrepTool};
use crate::tools::fd::FdTool;
use crate::tools::rg::{RipgrepMatch, RipgrepTool};
use crate::tools::rga::{RgaMatch, RgaTool};
#[cfg(feature = "indexing")]
use swe_grep_indexer::{IndexConfig, TantivyIndex};

/// Execute a single SWE-grep cycle using the phase-3 workflow.
pub async fn execute(args: SearchArgs) -> Result<SearchSummary> {
    let config = SearchConfig::try_from_args(args)?;
    let mut engine = SearchEngine::new(config)?;
    engine.run_cycle().await
}

struct SearchConfig {
    root: PathBuf,
    symbol: String,
    #[allow(dead_code)]
    language: Option<String>,
    language_tokens: Vec<String>,
    timeout: Duration,
    max_matches: usize,
    #[allow(dead_code)]
    concurrency: usize,
    use_index: bool,
    index_dir: PathBuf,
    use_rga: bool,
    use_fd: bool,
    use_ast: bool,
    cache_dir: PathBuf,
    log_dir: Option<PathBuf>,
}

impl SearchConfig {
    fn try_from_args(args: SearchArgs) -> Result<Self> {
        let root = args
            .path
            .unwrap_or(std::env::current_dir().context("failed to resolve current directory")?);
        let root = root.canonicalize().with_context(|| {
            format!(
                "failed to canonicalize repository root path: {}",
                root.display()
            )
        })?;

        let concurrency = usize::max(1, args.concurrency);
        let timeout = Duration::from_secs(args.timeout_secs);
        let index_dir = args
            .index_dir
            .clone()
            .unwrap_or_else(|| root.join(".swe-grep-index"));
        let cache_dir = args
            .cache_dir
            .clone()
            .unwrap_or_else(|| root.join(".swe-grep-cache"));
        let log_dir = args.log_dir.map(|dir| {
            if dir.is_absolute() {
                dir
            } else {
                root.join(dir)
            }
        });

        let use_fd = args.use_fd;
        let use_ast = args.use_ast_grep;

        let mut use_index = args.enable_index;
        if use_index && !cfg!(feature = "indexing") {
            eprintln!("warn: indexing support not compiled; ignoring --enable-index");
            use_index = false;
        }

        let language = args
            .language
            .map(|lang| lang.trim().to_string())
            .filter(|s| !s.is_empty());
        let language_tokens = expand_language_hint(language.as_deref());

        Ok(Self {
            root,
            symbol: args.symbol,
            language,
            language_tokens,
            timeout,
            max_matches: usize::max(1, args.max_matches),
            concurrency,
            use_index,
            index_dir,
            use_rga: args.enable_rga,
            use_fd,
            use_ast,
            cache_dir,
            log_dir,
        })
    }
}

struct SearchEngine {
    config: SearchConfig,
    fd_tool: Option<FdTool>,
    rg_tool: RipgrepTool,
    rga_tool: Option<RgaTool>,
    ast_tool: Option<AstGrepTool>,
    #[cfg(feature = "indexing")]
    index: Option<TantivyIndex>,
    dedup_cache: SearchCache,
    state: PersistentState,
    reward_total: f32,
    startup_stats: StartupStats,
    language_cache: HashMap<PathBuf, &'static str>,
}

impl SearchEngine {
    fn new(config: SearchConfig) -> Result<Self> {
        let init_start = StdInstant::now();
        let mut startup_stats = StartupStats::default();

        let fd_tool = None;

        let rg_start = StdInstant::now();
        let rg_tool = RipgrepTool::new(config.timeout, config.max_matches);
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

        let cache_start = StdInstant::now();
        fs::create_dir_all(&config.cache_dir).with_context(|| {
            format!(
                "failed to create cache directory {}",
                config.cache_dir.display()
            )
        })?;
        startup_stats.cache_ms = elapsed_std_ms(cache_start);

        let state_start = StdInstant::now();
        let state = PersistentState::load(&config.root, &config.cache_dir)?;
        startup_stats.state_ms = elapsed_std_ms(state_start);

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
            #[cfg(feature = "indexing")]
            index: None,
            dedup_cache: SearchCache::default(),
            state,
            reward_total: 0.0,
            startup_stats,
            language_cache: HashMap::new(),
        })
    }

    fn ensure_fd_tool(&mut self) -> Option<&FdTool> {
        if !self.config.use_fd {
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
        if !self.config.use_ast {
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

    async fn run_cycle(&mut self) -> Result<SearchSummary> {
        let mut stage_stats = StageStats::default();

        tracing::info!(symbol = %self.config.symbol, "search_cycle_start");

        let rewrites =
            QueryRewriter::for_symbol(&self.config.symbol, &self.config.language_tokens).build();
        if let Some(summary) = self.try_fast_path(&rewrites).await? {
            return Ok(summary);
        }

        // --- Discover ---
        let discover_start = Instant::now();
        let discover_candidates = self.discover().await;
        stage_stats.discover_ms = elapsed_ms(discover_start);
        stage_stats.discover_candidates = discover_candidates.len();
        stage_stats.record_discover_languages(&discover_candidates, stage_stats.discover_ms);
        let discover_set: HashSet<PathBuf> = discover_candidates.iter().cloned().collect();

        // --- Probe (Scoped) ---
        let probe_start = Instant::now();
        let (mut hits, scoped_hits_count) = self
            .probe(&rewrites, &discover_candidates, ProbeKind::Scoped)
            .await;
        stage_stats.probe_ms = elapsed_ms(probe_start);
        stage_stats.probe_hits = scoped_hits_count;
        stage_stats.record_probe_languages(&hits, stage_stats.probe_ms);

        // --- Escalate to global if needed ---
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
                            eprintln!("warn: tantivy search failed: {err}");
                        }
                    }
                }
                Err(err) => {
                    eprintln!("warn: failed to initialize index: {err}");
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
                        eprintln!("warn: rga search failed: {err}");
                    }
                }
                stage_stats.rga_ms = elapsed_ms(rga_start);
            }
        }

        // --- Disambiguate ---
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

        // --- Verify & Summarize ---
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
            eprintln!("warn: failed to persist cache state: {err}");
        }

        crate::telemetry::record_stage_latency("discover", stage_stats.discover_ms);
        crate::telemetry::record_stage_latency("probe", stage_stats.probe_ms);
        crate::telemetry::record_stage_latency("escalate", stage_stats.escalate_ms);
        crate::telemetry::record_stage_latency("index", stage_stats.index_ms);
        crate::telemetry::record_stage_latency("rga", stage_stats.rga_ms);
        crate::telemetry::record_stage_latency("ast", stage_stats.disambiguate_ms);
        crate::telemetry::record_stage_latency("verify", stage_stats.verify_ms);

        let summary = SearchSummary {
            cycle: 1,
            symbol: self.config.symbol.clone(),
            queries: rewrites,
            top_hits: verification.top_hits,
            deduped: verification.dedup_count,
            next_actions: verification.next_actions,
            fd_candidates: verification.fd_candidates,
            ast_hits: verification.ast_hits,
            startup_stats: Some(self.startup_stats.clone()),
            stage_stats,
            reward: round_two(self.reward_total),
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
                eprintln!("warn: fast-path ripgrep failed: {err}");
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
            .map(|m| SearchHit::from_ripgrep(&self.config.root, m, ProbeKind::Global))
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
            eprintln!("warn: failed to persist cache state: {err}");
        }

        crate::telemetry::record_stage_latency("probe", stage_stats.probe_ms);
        crate::telemetry::record_stage_latency("verify", stage_stats.verify_ms);

        let summary = SearchSummary {
            cycle: 1,
            symbol: self.config.symbol.clone(),
            queries: rewrites.to_vec(),
            top_hits: verification.top_hits,
            deduped: verification.dedup_count,
            next_actions: verification.next_actions,
            fd_candidates: Vec::new(),
            ast_hits: Vec::new(),
            startup_stats: Some(self.startup_stats.clone()),
            stage_stats,
            reward: round_two(self.reward_total),
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

    fn is_literal_symbol(&self) -> bool {
        let s = self.config.symbol.trim();
        !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
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
        let extension_filters = extensions_for_languages(&self.config.language_tokens);
        let extensions = extension_filters.as_deref();
        let mut candidates: Vec<PathBuf> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        let fd_results = if let Some(fd_tool) = self.ensure_fd_tool() {
            crate::telemetry::record_tool_invocation("fd");
            fd_tool
                .run(&root, symbol.as_str())
                .await
                .unwrap_or_else(|err| {
                    eprintln!("warn: fd invocation failed: {err}");
                    Vec::new()
                })
        } else {
            Vec::new()
        };
        crate::telemetry::record_tool_results("fd", fd_results.len());

        for path in fd_results {
            if let Ok(normalized) = normalize_path(&root, &path) {
                if passes_extension_filter(&normalized, extensions)
                    && seen.insert(normalized.clone())
                {
                    candidates.push(normalized);
                }
            }
        }

        let symbol_hints = self.state.hints_for_symbol(&self.config.symbol);
        crate::telemetry::record_cache_hits("symbol_hints", symbol_hints.len());
        for hint in symbol_hints {
            if passes_extension_filter(&hint, extensions) && seen.insert(hint.clone()) {
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
                                    && seen.insert(normalized.clone())
                                {
                                    candidates.push(normalized);
                                }
                            }
                        }
                    }
                }
                Err(err) => {
                    eprintln!(
                        "warn: failed to read cached directory {}: {err}",
                        dir_path.display()
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
        &self,
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
                    .map(|m| SearchHit::from_ripgrep(&self.config.root, m, kind.clone()))
                    .collect::<Vec<_>>();
                let hit_count = hits.len();
                (hits, hit_count)
            }
            Err(err) => {
                eprintln!("warn: ripgrep invocation failed: {err}");
                (Vec::new(), 0)
            }
        }
    }

    async fn disambiguate(&mut self, scope: &[PathBuf]) -> Vec<AstGrepMatch> {
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
                eprintln!("warn: ast-grep invocation failed: {err}");
                Vec::new()
            })
    }

    #[cfg(feature = "indexing")]
    async fn ensure_index(&mut self) -> Result<&TantivyIndex> {
        if self.index.is_none() {
            let extensions = extensions_for_languages(&self.config.language_tokens)
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
        });

        self.dedup_cache.retain_new(&mut dedup_hits);

        self.state.observe(&self.config.symbol, &dedup_hits);

        let top_hits: Vec<TopHit> = dedup_hits
            .iter()
            .take(5)
            .map(|hit| TopHit {
                path: hit.path.to_string_lossy().to_string(),
                line: hit.line,
                score: round_two(hit.score),
                origin: hit.origin.as_str().to_string(),
                origin_label: self.format_origin_label(&hit.origin, &hit.path),
                snippet: format_snippet(&self.config.root, &hit.path, hit.line, &hit.snippet),
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

#[derive(Clone, Debug)]
struct SearchHit {
    path: PathBuf,
    line: usize,
    snippet: String,
    score: f32,
    origin: HitOrigin,
}

impl SearchHit {
    fn from_ripgrep(root: &Path, rg_match: RipgrepMatch, kind: ProbeKind) -> Self {
        let absolute = if rg_match.path.is_absolute() {
            rg_match.path.clone()
        } else {
            root.join(&rg_match.path)
        };
        let normalized = normalize_path(root, &absolute).unwrap_or(absolute);
        let line = rg_match.line_number;
        Self {
            path: normalized,
            line,
            snippet: rg_match.lines,
            score: 1.0,
            origin: HitOrigin::Ripgrep(kind),
        }
    }

    fn from_rga(root: &Path, rga_match: RgaMatch) -> Self {
        let absolute = if rga_match.path.is_absolute() {
            rga_match.path.clone()
        } else {
            root.join(&rga_match.path)
        };
        let normalized = normalize_path(root, &absolute).unwrap_or(absolute);
        let line = rga_match.line_number;
        Self {
            path: normalized,
            line,
            snippet: rga_match.lines,
            score: 0.9,
            origin: HitOrigin::Rga,
        }
    }
}

#[derive(Clone, Debug)]
enum HitOrigin {
    Ripgrep(ProbeKind),
    AstGrep,
    Rga,
}

impl HitOrigin {
    fn as_str(&self) -> &str {
        match self {
            HitOrigin::Ripgrep(ProbeKind::Scoped) => "rg-scoped",
            HitOrigin::Ripgrep(ProbeKind::Global) => "rg-global",
            #[cfg(feature = "indexing")]
            HitOrigin::Ripgrep(ProbeKind::Indexed) => "rg-indexed",
            HitOrigin::AstGrep => "ast-grep",
            HitOrigin::Rga => "rga",
        }
    }
}

#[derive(Clone, Debug)]
enum ProbeKind {
    Scoped,
    Global,
    #[cfg(feature = "indexing")]
    Indexed,
}

#[derive(Serialize, Deserialize, Default)]
struct PersistentStateData {
    symbol_hits: HashMap<String, Vec<String>>,
    directory_scores: HashMap<String, u32>,
}

struct PersistentState {
    root: PathBuf,
    file_path: PathBuf,
    data: PersistentStateData,
    dirty: bool,
}

impl PersistentState {
    fn load(root: &Path, cache_dir: &Path) -> Result<Self> {
        let file_path = cache_dir.join("state.json");
        let data = if file_path.exists() {
            match fs::read_to_string(&file_path) {
                Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
                Err(err) => {
                    eprintln!(
                        "warn: failed to read persistent state {}; {err}",
                        file_path.display()
                    );
                    PersistentStateData::default()
                }
            }
        } else {
            PersistentStateData::default()
        };
        Ok(Self {
            root: root.to_path_buf(),
            file_path,
            data,
            dirty: false,
        })
    }

    fn hints_for_symbol(&self, symbol: &str) -> Vec<PathBuf> {
        self.data
            .symbol_hits
            .get(symbol)
            .into_iter()
            .flat_map(|paths| paths.iter())
            .filter_map(|text| {
                let relative = PathBuf::from(text);
                let absolute = self.root.join(&relative);
                if absolute.exists()
                    && absolute
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|name| !name.starts_with('.'))
                        .unwrap_or(true)
                {
                    Some(relative)
                } else {
                    None
                }
            })
            .collect()
    }

    fn top_directories(&self, limit: usize) -> Vec<PathBuf> {
        let mut dirs: Vec<_> = self.data.directory_scores.iter().collect();
        dirs.sort_by(|a, b| b.1.cmp(a.1));
        dirs.into_iter()
            .take(limit)
            .filter_map(|(dir, _)| {
                let path = PathBuf::from(dir);
                let absolute = self.root.join(&path);
                if absolute.is_dir() { Some(path) } else { None }
            })
            .collect()
    }

    fn observe(&mut self, symbol: &str, hits: &[SearchHit]) {
        if hits.is_empty() {
            return;
        }
        let entry = self.data.symbol_hits.entry(symbol.to_string()).or_default();

        for hit in hits.iter().take(10) {
            let text = hit.path.to_string_lossy().to_string();
            if !entry.contains(&text) {
                entry.push(text);
            }
        }
        if entry.len() > 10 {
            entry.drain(10..);
        }

        for hit in hits.iter().take(20) {
            if let Some(parent) = hit.path.parent() {
                if let Some(dir) = parent.to_str() {
                    if dir.is_empty() {
                        continue;
                    }
                    let counter = self
                        .data
                        .directory_scores
                        .entry(dir.to_string())
                        .or_insert(0);
                    *counter = counter.saturating_add(1);
                }
            }
        }

        self.dirty = true;
    }

    fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let tmp_path = self.file_path.with_extension("json.tmp");
        let file = fs::File::create(&tmp_path)
            .with_context(|| format!("failed to create {}", tmp_path.display()))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &self.data)
            .context("failed to serialize persistent state")?;
        writer.flush().ok();
        fs::rename(&tmp_path, &self.file_path).with_context(|| {
            format!(
                "failed to move persistent state into place {}",
                self.file_path.display()
            )
        })?;
        self.dirty = false;
        Ok(())
    }
}

#[derive(Default)]
struct SearchCache {
    seen: HashSet<(String, usize)>,
}

impl SearchCache {
    fn retain_new(&mut self, hits: &mut Vec<SearchHit>) {
        hits.retain(|hit| {
            let key = (hit.path.to_string_lossy().to_string(), hit.line);
            if self.seen.contains(&key) {
                false
            } else {
                self.seen.insert(key);
                true
            }
        });
    }
}

#[derive(Debug)]
struct QueryRewriter {
    symbol: String,
    languages: Vec<String>,
}

impl QueryRewriter {
    fn for_symbol(symbol: &str, languages: &[String]) -> Self {
        Self {
            symbol: symbol.to_string(),
            languages: languages.iter().cloned().collect(),
        }
    }

    fn build(&self) -> Vec<String> {
        let s = self.symbol.trim();
        if s.is_empty() {
            return Vec::new();
        }
        let type_hint = self.derive_type_hint();

        let mut queries = vec![
            Self::escape_literal(s),
            Self::escape_literal(&format!("{s} {type_hint}")),
            Self::escape_literal(&format!("{s} error")),
            Self::escape_literal(&format!("{type_hint}.{s}")),
        ];

        for lang in &self.languages {
            match lang.as_str() {
                "typescript" | "ts" | "tsx" => {
                    queries.extend(self.build_typescript_variants(s));
                }
                "swift" => {
                    queries.extend(self.build_swift_variants(s));
                }
                "rust" => {
                    queries.extend(self.build_rust_variants(s));
                }
                _ => {}
            }
        }

        dedup_queries(queries)
    }

    fn derive_type_hint(&self) -> String {
        let s = self.symbol.trim();
        if s.is_empty() {
            return "value".to_string();
        }
        if let Some(part) = s.rsplit([':', '_', '.']).next() {
            if s.contains('_') {
                return capitalize(part);
            }
        }
        if let Some(index) = s
            .char_indices()
            .filter(|(_, c)| c.is_uppercase())
            .map(|(i, _)| i)
            .last()
        {
            return s[index..].to_string();
        }

        capitalize(s)
    }

    fn build_typescript_variants(&self, symbol: &str) -> Vec<String> {
        let mut variants = Vec::new();
        if symbol.is_empty() {
            return variants;
        }

        let is_hook = symbol.starts_with("use") && symbol.len() > 3;
        let is_component = symbol
            .chars()
            .next()
            .map(|ch| ch.is_uppercase())
            .unwrap_or(false);

        variants.push(Self::escape_literal(&format!("{symbol}<")));
        variants.push(Self::escape_literal(&format!("{symbol} <")));
        variants.push(Self::escape_literal(&format!("<{symbol}")));
        variants.push(Self::escape_literal(&format!("</{symbol}")));
        variants.push(Self::escape_literal(&format!("{symbol} extends")));
        variants.push(Self::escape_literal(&format!("type {symbol}")));
        variants.push(Self::escape_literal(&format!("interface {symbol}")));
        variants.push(Self::escape_literal(&format!("const {symbol}")));
        variants.push(Self::escape_literal(&format!("export const {symbol}")));
        variants.push(Self::escape_literal(&format!("function {symbol}")));
        variants.push(Self::escape_literal(&format!("export function {symbol}")));
        variants.push(Self::escape_literal(&format!("{symbol}(")));
        variants.push(Self::escape_literal(&format!("{symbol} satisfies")));
        variants.push(Self::escape_literal(&format!("namespace {symbol}")));
        variants.push(Self::escape_literal(&format!("export default {symbol}")));
        variants.push(Self::escape_literal(&format!("{symbol} props")));
        variants.push(Self::escape_literal(&format!("{symbol}:")));
        if is_hook {
            variants.push(Self::escape_literal(&format!("{symbol}(")));
            variants.push(Self::escape_literal(&format!("{symbol}<{{")));
        }

        if symbol
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
        {
            variants.push(Self::escape_literal(&format!("<{symbol} ")));
            variants.push(Self::escape_literal(&format!("<{symbol} />")));
            variants.push(Self::escape_literal(&format!("{symbol}Props")));
            variants.push(Self::escape_literal(&format!("{symbol}Component")));
        }

        if is_component {
            variants.push(Self::escape_literal(&format!("<{symbol} {{...")));
            variants.push(Self::escape_literal(&format!("React.memo({symbol}")));
            variants.push(Self::escape_literal(&format!("React.forwardRef({symbol}")));
        }

        variants
    }

    fn build_rust_variants(&self, symbol: &str) -> Vec<String> {
        if symbol.is_empty() {
            return Vec::new();
        }

        vec![
            Self::escape_literal(&format!("fn {symbol}")),
            Self::escape_literal(&format!("impl {symbol}")),
            Self::escape_literal(&format!("trait {symbol}")),
            Self::escape_literal(&format!("pub(crate) {symbol}")),
            Self::escape_literal(&format!("{symbol}::<")),
            Self::escape_literal(&format!("::{symbol}")),
            Self::escape_literal(&format!("macro_rules! {symbol}")),
        ]
    }

    fn build_swift_variants(&self, symbol: &str) -> Vec<String> {
        if symbol.is_empty() {
            return Vec::new();
        }

        let is_type_like = symbol
            .chars()
            .next()
            .map(|ch| ch.is_uppercase())
            .unwrap_or(false);

        let mut variants = vec![
            Self::escape_literal(&format!("func {symbol}")),
            Self::escape_literal(&format!("func {symbol}(")),
            Self::escape_literal(&format!("func {symbol}<")),
            Self::escape_literal(&format!("{symbol} async")),
            Self::escape_literal(&format!("@MainActor func {symbol}")),
        ];

        variants.push(Self::escape_literal(&format!("{symbol}(")));
        variants.push(Self::escape_literal(&format!(".{symbol}")));
        variants.push(Self::escape_literal(&format!("self.{symbol}")));
        variants.push(Self::escape_literal(&format!("await {symbol}")));
        if is_type_like {
            variants.push(Self::escape_literal(&format!("@{symbol}")));
            variants.push(Self::escape_literal(&format!(": {symbol}")));
            variants.push(Self::escape_literal(&format!("extension {symbol}")));
            variants.push(Self::escape_literal(&format!("where {symbol}")));
        }

        variants
    }

    fn escape_literal(value: &str) -> String {
        let mut escaped = String::with_capacity(value.len());
        for ch in value.chars() {
            match ch {
                '\\' | '.' | '+' | '*' | '?' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}'
                | '|' => {
                    escaped.push('\\');
                    escaped.push(ch);
                }
                _ => escaped.push(ch),
            }
        }
        escaped
    }
}

fn compute_metrics(
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
    let density = density_raw / (density_raw + 1.0); // squash into (0,1)

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

fn capitalize(segment: &str) -> String {
    let mut chars = segment.chars();
    if let Some(first) = chars.next() {
        let mut result = String::new();
        result.push(first.to_ascii_uppercase());
        result.extend(chars);
        return result;
    }
    segment.to_string()
}

fn dedup_queries<I>(queries: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for query in queries {
        if seen.insert(query.clone()) {
            deduped.push(query);
        }
    }
    deduped
}

fn expand_language_hint(language: Option<&str>) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let Some(raw) = language else {
        return tokens;
    };
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return tokens;
    }

    if normalized.starts_with("auto-") {
        let remainder = normalized.trim_start_matches("auto-");
        let parts: Vec<&str> = remainder
            .split(|ch| matches!(ch, '-' | '+' | '|' | ','))
            .filter(|part| !part.is_empty())
            .collect();
        for part in parts {
            tokens.extend(expand_language_token(part));
        }
    } else {
        let parts: Vec<&str> = normalized
            .split(|ch| matches!(ch, '+' | '|' | ','))
            .filter(|part| !part.is_empty())
            .collect();
        if parts.is_empty() {
            tokens.extend(expand_language_token(&normalized));
        } else {
            for part in parts {
                tokens.extend(expand_language_token(part));
            }
        }
    }

    let mut dedup: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for token in tokens {
        if seen.insert(token.clone()) {
            dedup.push(token);
        }
    }
    dedup
}

fn expand_language_token(token: &str) -> Vec<String> {
    match token {
        "typescript" | "ts" => vec!["ts".to_string(), "tsx".to_string()],
        "tsx" => vec!["tsx".to_string()],
        "swift" => vec!["swift".to_string()],
        "rust" | "rs" => vec!["rust".to_string()],
        "javascript" | "js" => vec!["js".to_string(), "jsx".to_string()],
        "jsx" => vec!["jsx".to_string()],
        "kotlin" | "kt" => vec!["kt".to_string(), "kts".to_string()],
        "kts" => vec!["kts".to_string()],
        "python" | "py" => vec!["py".to_string()],
        "swiftui" => vec!["swift".to_string()],
        other => vec![other.to_string()],
    }
}

fn languages_include(tokens: &[String], needle: &str) -> bool {
    tokens.iter().any(|token| token == needle)
}

fn extensions_for_languages(languages: &[String]) -> Option<Vec<&'static str>> {
    let mut results: Vec<&'static str> = Vec::new();
    for lang in languages {
        match lang.as_str() {
            "swift" => {
                if !results.contains(&"swift") {
                    results.push("swift");
                }
            }
            "tsx" => {
                if !results.contains(&"tsx") {
                    results.push("tsx");
                }
            }
            "ts" | "typescript" => {
                if !results.contains(&"ts") {
                    results.push("ts");
                }
                if !results.contains(&"tsx") {
                    results.push("tsx");
                }
            }
            "rust" => {
                if !results.contains(&"rs") {
                    results.push("rs");
                }
            }
            "js" | "javascript" => {
                if !results.contains(&"js") {
                    results.push("js");
                }
                if !results.contains(&"jsx") {
                    results.push("jsx");
                }
            }
            "jsx" => {
                if !results.contains(&"jsx") {
                    results.push("jsx");
                }
            }
            "kt" | "kts" | "kotlin" => {
                if !results.contains(&"kt") {
                    results.push("kt");
                }
                if !results.contains(&"kts") {
                    results.push("kts");
                }
            }
            "py" | "python" => {
                if !results.contains(&"py") {
                    results.push("py");
                }
            }
            _ => {}
        }
    }
    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}

fn detect_language_from_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    match ext.to_ascii_lowercase().as_str() {
        "rs" => Some("rust"),
        "swift" => Some("swift"),
        "ts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "js" => Some("javascript"),
        "jsx" => Some("jsx"),
        "py" => Some("python"),
        "kt" => Some("kotlin"),
        "kts" => Some("kotlin"),
        _ => None,
    }
}

fn aggregate_language_counts<'a, I>(paths: I) -> BTreeMap<String, usize>
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

fn format_snippet(root: &Path, path: &Path, line: usize, raw: &str) -> Option<String> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    match ext.as_deref() {
        Some("swift") => format_swift_snippet(root, path, line, raw),
        Some("ts") | Some("tsx") => format_typescript_snippet(raw),
        _ => format_default_snippet(raw),
    }
}

fn format_swift_snippet(root: &Path, path: &Path, line: usize, raw: &str) -> Option<String> {
    let trimmed: Vec<String> = raw.lines().map(|entry| entry.trim().to_string()).collect();

    let mut attributes_rev: Vec<String> = Vec::new();

    let mut candidate_idx: Option<usize> = None;
    for (idx, entry) in trimmed.iter().enumerate() {
        if entry.is_empty() || entry.starts_with("//") {
            continue;
        }
        if entry.starts_with("func ")
            || entry.starts_with("protocol ")
            || entry.starts_with("extension ")
            || entry.starts_with("struct ")
            || entry.starts_with("class ")
            || entry.starts_with("actor ")
            || entry.starts_with("init(")
            || entry.starts_with("init ")
            || entry.starts_with("enum ")
        {
            candidate_idx = Some(idx);
            break;
        }
    }

    let (selected_idx, _selected) = if let Some(idx) = candidate_idx {
        (idx, trimmed[idx].clone())
    } else {
        trimmed.iter().enumerate().find_map(|(idx, entry)| {
            if entry.is_empty() {
                None
            } else {
                Some((idx, entry.clone()))
            }
        })?
    };

    let mut signature_segments = vec![trimmed[selected_idx].clone()];
    for entry in trimmed.iter().skip(selected_idx + 1) {
        let trimmed_entry = entry.trim();
        if trimmed_entry.is_empty() {
            break;
        }
        if trimmed_entry.starts_with('@') {
            let attr = collapse_whitespace(trimmed_entry);
            if !attributes_rev.iter().any(|existing| existing == &attr) {
                attributes_rev.push(attr);
            }
            continue;
        }
        if trimmed_entry.starts_with('}') {
            break;
        }
        if trimmed_entry.starts_with(")")
            || trimmed_entry.starts_with("async")
            || trimmed_entry.starts_with("throws")
            || trimmed_entry.starts_with("rethrows")
            || trimmed_entry.starts_with("->")
            || trimmed_entry.starts_with("where ")
            || trimmed_entry.starts_with("some ")
        {
            signature_segments.push(trimmed_entry.to_string());
            continue;
        }
        break;
    }

    let collapsed_signature = collapse_whitespace(&signature_segments.join(" "));
    let mut formatted = collapsed_signature.clone();
    let lowered_sig = collapsed_signature.to_ascii_lowercase();
    if collapsed_signature.contains("async") {
        formatted.push_str(" [async]");
    }
    if lowered_sig.contains("await ") {
        formatted.push_str(" [await]");
    }
    for access in ["public", "internal", "private", "fileprivate", "open"].iter() {
        if lowered_sig.starts_with(access)
            || lowered_sig.contains(&format!(" {access} "))
            || lowered_sig.contains(&format!(" {access}("))
        {
            formatted.push_str(" [");
            formatted.push_str(access);
            formatted.push(']');
            break;
        }
    }
    if collapsed_signature.contains('<') && collapsed_signature.contains('>') {
        formatted.push_str(" [generic]");
    }

    let mut context: Option<String> = None;

    if selected_idx > 0 {
        for entry in trimmed[..selected_idx].iter().rev() {
            if entry.is_empty() {
                continue;
            }
            if entry.starts_with('@') {
                let attr = collapse_whitespace(entry);
                if !attributes_rev.iter().any(|existing| existing == &attr) {
                    attributes_rev.push(attr);
                }
                continue;
            }
            if entry.starts_with("extension ")
                || entry.starts_with("struct ")
                || entry.starts_with("class ")
                || entry.starts_with("protocol ")
                || entry.starts_with("actor ")
                || entry.starts_with("enum ")
            {
                context = Some(collapse_whitespace(entry));
            }
            break;
        }
    }

    if context.is_none() {
        let full_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        if let Ok(contents) = fs::read_to_string(&full_path) {
            let lines: Vec<&str> = contents.lines().collect();
            if !lines.is_empty() {
                let mut idx = line.saturating_sub(1);
                while idx > 0 {
                    idx -= 1;
                    if let Some(candidate) = lines.get(idx) {
                        let trimmed_candidate = candidate.trim();
                        if trimmed_candidate.is_empty() {
                            continue;
                        }
                        if trimmed_candidate.starts_with('@') {
                            let attr = collapse_whitespace(trimmed_candidate);
                            if !attributes_rev.iter().any(|existing| existing == &attr) {
                                attributes_rev.push(attr);
                            }
                            continue;
                        }
                        if trimmed_candidate.starts_with("extension ")
                            || trimmed_candidate.starts_with("struct ")
                            || trimmed_candidate.starts_with("class ")
                            || trimmed_candidate.starts_with("protocol ")
                            || trimmed_candidate.starts_with("actor ")
                            || trimmed_candidate.starts_with("enum ")
                        {
                            context = Some(collapse_whitespace(trimmed_candidate));
                            break;
                        }
                        if trimmed_candidate.starts_with("func ")
                            || trimmed_candidate.starts_with("init")
                            || trimmed_candidate.starts_with("let ")
                            || trimmed_candidate.starts_with("var ")
                        {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
        }
    }

    attributes_rev.reverse();
    if let Some(ctx) = context {
        formatted = format!("{ctx} :: {formatted}");
    }
    for attr in attributes_rev {
        formatted.push_str(" [");
        formatted.push_str(&attr);
        formatted.push(']');
    }

    Some(formatted)
}

fn format_typescript_snippet(raw: &str) -> Option<String> {
    let mut candidate: Option<String> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("<") || trimmed.starts_with("</") {
            return Some(collapse_whitespace(trimmed));
        }
        if trimmed.contains('<') && trimmed.contains('>') {
            candidate = Some(trimmed.to_string());
            break;
        }
        if trimmed.starts_with("export")
            || trimmed.starts_with("type ")
            || trimmed.starts_with("interface ")
            || trimmed.contains("=>")
        {
            candidate = Some(trimmed.to_string());
        }
    }

    let selected = candidate.or_else(|| {
        raw.lines()
            .map(|line| line.trim().to_string())
            .find(|line| !line.is_empty())
    })?;

    let mut formatted = collapse_whitespace(&selected);
    if selected.contains("async") {
        formatted.push_str(" [async]");
    }
    let lowered = selected.trim_start();
    if lowered.starts_with("use") || lowered.contains(" = use") {
        formatted.push_str(" [hook]");
    }
    if lowered.contains("React.FC") || lowered.contains("React.FunctionComponent") {
        formatted.push_str(" [component]");
    }
    if lowered.contains("React.forwardRef") || lowered.contains("React.memo") {
        formatted.push_str(" [component]");
    }
    if lowered.contains("Promise<") {
        formatted.push_str(" [promise]");
    }
    if lowered.contains("=>") {
        formatted.push_str(" [arrow]");
    }
    if lowered.contains("await ") {
        formatted.push_str(" [await]");
    }
    if selected.contains('<') && selected.contains('>') {
        formatted.push_str(" [generic]");
    }
    if lowered.contains("satisfies ") {
        formatted.push_str(" [satisfies]");
    }
    Some(formatted)
}

fn format_default_snippet(raw: &str) -> Option<String> {
    raw.lines()
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
        .map(collapse_whitespace)
}

fn collapse_whitespace(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut last_was_space = false;
    for ch in input.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(ch);
            last_was_space = false;
        }
    }
    result.trim().to_string()
}

fn passes_extension_filter(path: &Path, extensions: Option<&[&str]>) -> bool {
    match extensions {
        Some(exts) => path
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| {
                exts.iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(ext))
            })
            .unwrap_or(false),
        None => true,
    }
}

fn normalize_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let canonical = absolute.canonicalize().unwrap_or_else(|_| absolute.clone());
    Ok(canonical
        .strip_prefix(root)
        .map(|p| p.to_path_buf())
        .unwrap_or(canonical))
}

fn elapsed_std_ms(start: StdInstant) -> u64 {
    let nanos = start.elapsed().as_nanos();
    ((nanos + 999_999) / 1_000_000) as u64
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

fn round_two(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

#[derive(Default)]
struct SearchMetrics {
    precision: f32,
    density: f32,
    cluster_score: f32,
    reward: f32,
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
    fn record_discover_languages(&mut self, candidates: &[PathBuf], latency_ms: u64) {
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

    fn record_probe_languages(&mut self, hits: &[SearchHit], latency_ms: u64) {
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

    fn record_escalate_languages(&mut self, hits: &[SearchHit], latency_ms: u64) {
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

    fn record_disambiguate_languages(&mut self, matches: &[AstGrepMatch], latency_ms: u64) {
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
            metrics.latency.disambiguate_ms =
                metrics.latency.disambiguate_ms.saturating_add(share);
        }
    }

    fn record_verify_languages(&mut self, counts: &BTreeMap<String, usize>, latency_ms: u64) {
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
    pub symbol: String,
    pub queries: Vec<String>,
    pub top_hits: Vec<TopHit>,
    pub deduped: usize,
    pub next_actions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fd_candidates: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ast_hits: Vec<(PathBuf, usize)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_stats: Option<StartupStats>,
    pub stage_stats: StageStats,
    pub reward: f32,
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
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn is_usize_zero(value: &usize) -> bool {
    *value == 0
}

fn distribute_latency(latency_ms: u64, buckets: usize) -> Vec<u64> {
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
    fn is_empty(&self) -> bool {
        self.discover_ms == 0
            && self.probe_ms == 0
            && self.escalate_ms == 0
            && self.disambiguate_ms == 0
            && self.verify_ms == 0
    }
}
