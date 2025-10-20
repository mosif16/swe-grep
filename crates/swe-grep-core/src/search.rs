use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{self, json};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
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
    language: Option<String>,
    timeout: Duration,
    max_matches: usize,
    concurrency: usize,
    enable_index: bool,
    index_dir: PathBuf,
    enable_rga: bool,
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

        let mut enable_index = args.enable_index;
        if enable_index && !cfg!(feature = "indexing") {
            eprintln!("warn: indexing support not compiled; ignoring --enable-index");
            enable_index = false;
        }

        Ok(Self {
            root,
            symbol: args.symbol,
            language: args.language,
            timeout,
            max_matches: usize::max(1, args.max_matches),
            concurrency,
            enable_index,
            index_dir,
            enable_rga: args.enable_rga,
            cache_dir,
            log_dir,
        })
    }
}

struct SearchEngine {
    config: SearchConfig,
    fd_tool: FdTool,
    rg_tool: RipgrepTool,
    rga_tool: Option<RgaTool>,
    ast_tool: AstGrepTool,
    #[cfg(feature = "indexing")]
    index: Option<TantivyIndex>,
    dedup_cache: SearchCache,
    state: PersistentState,
    reward_total: f32,
}

impl SearchEngine {
    fn new(config: SearchConfig) -> Result<Self> {
        let fd_tool = FdTool::new(config.timeout, 200);
        let rg_tool = RipgrepTool::new(config.timeout, config.max_matches);
        let ast_tool = AstGrepTool::new(config.timeout, config.max_matches);
        if config.enable_index {
            fs::create_dir_all(&config.index_dir).with_context(|| {
                format!(
                    "failed to create index directory {}",
                    config.index_dir.display()
                )
            })?;
        }
        fs::create_dir_all(&config.cache_dir).with_context(|| {
            format!(
                "failed to create cache directory {}",
                config.cache_dir.display()
            )
        })?;
        let state = PersistentState::load(&config.root, &config.cache_dir)?;
        let rga_tool = if config.enable_rga {
            Some(RgaTool::new(config.timeout, config.max_matches))
        } else {
            None
        };
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
        })
    }

    async fn run_cycle(&mut self) -> Result<SearchSummary> {
        let mut stage_stats = StageStats::default();

        // --- Discover ---
        let discover_start = Instant::now();
        let discover_candidates = self.discover().await;
        stage_stats.discover_ms = elapsed_ms(discover_start);
        stage_stats.discover_candidates = discover_candidates.len();
        let discover_set: HashSet<PathBuf> = discover_candidates.iter().cloned().collect();

        // --- Probe (Scoped) ---
        let rewrites = QueryRewriter::for_symbol(&self.config.symbol).build();
        let probe_start = Instant::now();
        let (mut hits, scoped_hits_count) = self
            .probe(&rewrites, &discover_candidates, ProbeKind::Scoped)
            .await;
        stage_stats.probe_ms = elapsed_ms(probe_start);
        stage_stats.probe_hits = scoped_hits_count;

        // --- Escalate to global if needed ---
        if hits.is_empty() {
            let escalate_start = Instant::now();
            let (global_hits, global_hits_count) =
                self.probe(&rewrites, &[], ProbeKind::Global).await;
            stage_stats.escalate_ms = elapsed_ms(escalate_start);
            stage_stats.escalate_hits = global_hits_count;
            hits = global_hits;
        }

        #[cfg(feature = "indexing")]
        if hits.is_empty() && self.config.enable_index {
            let index_stage_start = Instant::now();
            let symbol = self.config.symbol.clone();
            let max_matches = self.config.max_matches;
            match self.ensure_index().await {
                Ok(index) => match index.search(&symbol, max_matches).await {
                    Ok(candidates) => {
                        stage_stats.index_candidates = candidates.len();
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
                },
                Err(err) => {
                    eprintln!("warn: failed to initialize index: {err}");
                }
            }
            stage_stats.index_ms = elapsed_ms(index_stage_start);
        }

        if hits.is_empty() {
            if let Some(rga_tool) = &self.rga_tool {
                let rga_start = Instant::now();
                match rga_tool
                    .search(&self.config.root, &self.config.symbol)
                    .await
                {
                    Ok(matches) => {
                        stage_stats.rga_hits = matches.len();
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

        // --- Verify & Summarize ---
        let verify_start = Instant::now();
        let verification = self
            .verify(hits, ast_matches, discover_set, discover_candidates.clone())
            .await?;
        stage_stats.verify_ms = elapsed_ms(verify_start);

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

        let summary = SearchSummary {
            cycle: 1,
            symbol: self.config.symbol.clone(),
            queries: rewrites,
            top_hits: verification.top_hits,
            deduped: verification.dedup_count,
            next_actions: verification.next_actions,
            fd_candidates: verification.fd_candidates,
            ast_hits: verification.ast_hits,
            stage_stats,
            reward: round_two(self.reward_total),
        };

        self.log_summary(&summary).await?;

        Ok(summary)
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
            "enable_index": self.config.enable_index,
            "enable_rga": self.config.enable_rga,
            "summary": summary,
        });

        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        file.write_all(&line).await?;

        Ok(())
    }

    async fn discover(&self) -> Vec<PathBuf> {
        let extensions = self
            .config
            .language
            .as_deref()
            .and_then(language_to_extensions);
        let mut candidates: Vec<PathBuf> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        let fd_results = self
            .fd_tool
            .run(&self.config.root, &self.config.symbol)
            .await
            .unwrap_or_else(|err| {
                eprintln!("warn: fd invocation failed: {err}");
                Vec::new()
            });

        for path in fd_results {
            if let Ok(normalized) = normalize_path(&self.config.root, &path) {
                if passes_extension_filter(&normalized, extensions)
                    && seen.insert(normalized.clone())
                {
                    candidates.push(normalized);
                }
            }
        }

        for hint in self.state.hints_for_symbol(&self.config.symbol) {
            if passes_extension_filter(&hint, extensions) && seen.insert(hint.clone()) {
                candidates.push(hint);
            }
        }

        for dir in self.state.top_directories(3) {
            let dir_path = self.config.root.join(&dir);
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
                            if let Ok(normalized) = normalize_path(&self.config.root, &path) {
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

        candidates
    }

    async fn probe(
        &self,
        rewrites: &[String],
        scope: &[PathBuf],
        kind: ProbeKind,
    ) -> (Vec<SearchHit>, usize) {
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency));
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let scope_arc = Arc::new(scope.to_vec());
        let mut workers = JoinSet::new();

        for query in rewrites.iter().cloned() {
            let tool = self.rg_tool.clone();
            let root = self.config.root.clone();
            let scope_clone = scope_arc.clone();
            let semaphore = semaphore.clone();
            let cancel = cancel_flag.clone();
            workers.spawn(async move {
                let permit = semaphore.acquire_owned().await.expect("semaphore closed");
                if cancel.load(Ordering::Relaxed) {
                    drop(permit);
                    return (query, Ok(Vec::new()));
                }
                let result = tool.search(&root, &query, scope_clone.as_ref()).await;
                drop(permit);
                (query, result)
            });
        }

        let mut hits = Vec::new();
        let mut total_matches = 0usize;

        while let Some(res) = workers.join_next().await {
            match res {
                Ok((_query, Ok(matches))) => {
                    total_matches += matches.len();
                    for m in matches {
                        hits.push(SearchHit::from_ripgrep(&self.config.root, m, kind.clone()));
                    }
                    if hits.len() >= self.config.max_matches {
                        cancel_flag.store(true, Ordering::Relaxed);
                        workers.shutdown().await;
                        break;
                    }
                }
                Ok((query, Err(err))) => {
                    eprintln!("warn: ripgrep query `{query}` failed: {err}");
                }
                Err(join_err) => {
                    eprintln!("warn: ripgrep worker task failed: {join_err}");
                }
            }
        }

        (hits, total_matches)
    }

    async fn disambiguate(&self, scope: &[PathBuf]) -> Vec<AstGrepMatch> {
        self.ast_tool
            .search_identifier(
                &self.config.root,
                &self.config.symbol,
                self.config.language.as_deref(),
                scope,
            )
            .await
            .unwrap_or_else(|err| {
                eprintln!("warn: ast-grep invocation failed: {err}");
                Vec::new()
            })
    }

    #[cfg(feature = "indexing")]
    async fn ensure_index(&mut self) -> Result<&TantivyIndex> {
        if self.index.is_none() {
            let extensions = self
                .config
                .language
                .as_deref()
                .and_then(language_to_extensions)
                .map(|exts| exts.iter().map(|s| s.to_string()).collect::<Vec<_>>());
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
                snippet: Some(hit.snippet.trim().to_string()),
            })
            .collect();

        let next_actions: Vec<String> = top_hits
            .iter()
            .map(|hit| format!("inspect {}:{}", hit.path, hit.line))
            .collect();

        let metrics = compute_metrics(&dedup_hits, &ast_set, fd_set.len());

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
}

impl QueryRewriter {
    fn for_symbol(symbol: &str) -> Self {
        Self {
            symbol: symbol.to_string(),
        }
    }

    fn build(&self) -> Vec<String> {
        let s = self.symbol.trim();
        let type_hint = self.derive_type_hint();
        vec![
            s.to_string(),
            format!("{s} {type_hint}"),
            format!("{s} error"),
            format!("{type_hint}.{s}"),
        ]
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

fn language_to_extensions(language: &str) -> Option<&'static [&'static str]> {
    match language.to_ascii_lowercase().as_str() {
        "rust" => Some(&["rs"]),
        "swift" => Some(&["swift"]),
        "typescript" | "ts" => Some(&["ts", "tsx"]),
        "tsx" => Some(&["tsx"]),
        "javascript" | "js" => Some(&["js", "jsx"]),
        "python" | "py" => Some(&["py"]),
        "kotlin" => Some(&["kt", "kts"]),
        _ => None,
    }
}

fn passes_extension_filter(path: &Path, extensions: Option<&'static [&'static str]>) -> bool {
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
    pub stage_stats: StageStats,
    pub reward: f32,
}

#[derive(Clone, Serialize)]
pub struct TopHit {
    pub path: String,
    pub line: usize,
    pub score: f32,
    pub origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}
