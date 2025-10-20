use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use serde::Serialize;

use crate::cli::SearchArgs;
use crate::tools::ast_grep::{AstGrepMatch, AstGrepTool};
use crate::tools::fd::FdTool;
use crate::tools::rg::{RipgrepMatch, RipgrepTool};

/// Execute the phase-2 search workflow built around five explicit stages.
pub async fn execute(args: SearchArgs) -> Result<SearchSummary> {
    let root = args
        .path
        .unwrap_or(std::env::current_dir().context("failed to resolve current directory")?);
    let root = root.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize repository root path: {}",
            root.display()
        )
    })?;

    let timeout = Duration::from_secs(args.timeout_secs);
    let fd_tool = FdTool::new(timeout, 200);
    let rg_tool = RipgrepTool::new(timeout, args.max_matches);
    let ast_tool = AstGrepTool::new(timeout, args.max_matches);

    let mut stage_stats = StageStats::default();

    // --- Discover ---
    let discover_candidates =
        discover_stage(&fd_tool, &root, &args.symbol, args.language.as_deref()).await;
    stage_stats.discover_candidates = discover_candidates.len();
    let fd_set: HashSet<PathBuf> = discover_candidates.iter().cloned().collect();

    // --- Probe ---
    let rewrites = QueryRewriter::for_symbol(&args.symbol).build();
    let (mut hits, scoped_hits_count) = probe_stage(
        &rg_tool,
        &root,
        &rewrites,
        &discover_candidates,
        ProbeKind::Scoped,
    )
    .await;
    stage_stats.probe_hits = scoped_hits_count;

    // --- Escalate (if necessary) ---
    if hits.is_empty() {
        let (global_hits, global_hits_count) =
            probe_stage(&rg_tool, &root, &rewrites, &[], ProbeKind::Global).await;
        stage_stats.escalate_hits = global_hits_count;
        hits = global_hits;
    }

    // --- Disambiguate ---
    let ast_scope: Vec<PathBuf> = hits
        .iter()
        .map(|hit| hit.path.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let ast_matches = disambiguate_stage(
        &ast_tool,
        &root,
        &args.symbol,
        args.language.as_deref(),
        &ast_scope,
    )
    .await;
    stage_stats.ast_matches = ast_matches.len();

    // --- Verify & Summarize ---
    let ast_set: HashSet<(PathBuf, usize)> = ast_matches
        .iter()
        .filter_map(|m| {
            normalize_path(&root, &m.path)
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
        if let HitOrigin::Ripgrep(ProbeKind::Global) = hit.origin {
            hit.score -= 0.05;
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

    let top_hits: Vec<TopHit> = dedup_hits
        .iter()
        .take(5)
        .map(|hit| TopHit {
            path: hit.path.to_string_lossy().to_string(),
            line: hit.line,
            score: (hit.score * 100.0).round() / 100.0,
            origin: hit.origin.as_str().to_string(),
            snippet: Some(hit.snippet.trim().to_string()),
        })
        .collect();

    let next_actions: Vec<String> = top_hits
        .iter()
        .map(|hit| format!("inspect {}:{}", hit.path, hit.line))
        .collect();

    Ok(SearchSummary {
        cycle: 1,
        queries: rewrites,
        top_hits,
        deduped: dedup_hits.len(),
        next_actions,
        fd_candidates: discover_candidates,
        ast_hits: ast_matches
            .into_iter()
            .filter_map(|a| {
                normalize_path(&root, &a.path)
                    .ok()
                    .map(|path| (path, a.line + 1))
            })
            .collect(),
        stage_stats,
    })
}

async fn discover_stage(
    fd_tool: &FdTool,
    root: &Path,
    symbol: &str,
    language: Option<&str>,
) -> Vec<PathBuf> {
    let extensions = language.and_then(language_to_extensions);

    fd_tool
        .run(root, symbol)
        .await
        .unwrap_or_else(|err| {
            eprintln!("warn: fd invocation failed: {err}");
            Vec::new()
        })
        .into_iter()
        .filter_map(|path| normalize_path(root, &path).ok())
        .filter(|path| {
            if let Some(exts) = extensions {
                match path.extension().and_then(|e| e.to_str()) {
                    Some(ext) => exts
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(ext)),
                    None => false,
                }
            } else {
                true
            }
        })
        .collect()
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

async fn probe_stage(
    rg_tool: &RipgrepTool,
    root: &Path,
    rewrites: &[String],
    scope: &[PathBuf],
    kind: ProbeKind,
) -> (Vec<SearchHit>, usize) {
    let root_buf = root.to_path_buf();
    let scope_arc = Arc::new(scope.to_vec());
    let mut futures = FuturesUnordered::new();

    for query in rewrites.iter().cloned() {
        let tool = rg_tool.clone();
        let root_clone = root_buf.clone();
        let scope_clone = scope_arc.clone();
        futures.push(async move {
            let result = tool.search(&root_clone, &query, scope_clone.as_ref()).await;
            (query, result)
        });
    }

    let mut hits = Vec::new();
    let mut total_matches = 0usize;

    while let Some((query, result)) = futures.next().await {
        let matches = match result {
            Ok(res) => res,
            Err(err) => {
                eprintln!("warn: ripgrep query `{query}` failed: {err}");
                continue;
            }
        };
        total_matches += matches.len();
        for m in matches {
            hits.push(SearchHit::from_ripgrep(&root_buf, m, kind.clone()));
        }
    }

    (hits, total_matches)
}

async fn disambiguate_stage(
    ast_tool: &AstGrepTool,
    root: &Path,
    symbol: &str,
    language: Option<&str>,
    scope: &[PathBuf],
) -> Vec<AstGrepMatch> {
    ast_tool
        .search_identifier(root, symbol, language, scope)
        .await
        .unwrap_or_else(|err| {
            eprintln!("warn: ast-grep invocation failed: {err}");
            Vec::new()
        })
}

#[derive(Debug, Clone)]
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
        let origin = HitOrigin::Ripgrep(kind);
        Self {
            path: normalized,
            line,
            snippet: rg_match.lines,
            score: 1.0,
            origin,
        }
    }
}

#[derive(Debug, Clone)]
enum HitOrigin {
    Ripgrep(ProbeKind),
    AstGrep,
}

impl HitOrigin {
    fn as_str(&self) -> &str {
        match self {
            HitOrigin::Ripgrep(ProbeKind::Scoped) => "rg-scoped",
            HitOrigin::Ripgrep(ProbeKind::Global) => "rg-global",
            HitOrigin::AstGrep => "ast-grep",
        }
    }
}

#[derive(Debug, Clone)]
enum ProbeKind {
    Scoped,
    Global,
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

#[derive(Default, Serialize)]
pub struct StageStats {
    pub discover_candidates: usize,
    pub probe_hits: usize,
    pub escalate_hits: usize,
    pub ast_matches: usize,
}

#[derive(Serialize)]
pub struct SearchSummary {
    pub cycle: u32,
    pub queries: Vec<String>,
    pub top_hits: Vec<TopHit>,
    pub deduped: usize,
    pub next_actions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fd_candidates: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ast_hits: Vec<(PathBuf, usize)>,
    pub stage_stats: StageStats,
}

#[derive(Serialize)]
pub struct TopHit {
    pub path: String,
    pub line: usize,
    pub score: f32,
    pub origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}
