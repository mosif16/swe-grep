use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::language::detect_language_from_path;
use crate::search::passes_extension_filter;

use super::callgraph::{CallEdge, CallGraphIndex};
use super::extract::ExtractFactsOptions;
use super::facts::{FACTS_SCHEMA_VERSION, FileFacts};
use super::types::{TypeGraphIndex, TypeRelationEdge};

#[derive(Debug, Default, Clone, Copy)]
pub struct CacheStats {
    pub processed_files: usize,
    pub cache_hits: usize,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f32 {
        if self.processed_files == 0 {
            return 0.0;
        }
        self.cache_hits as f32 / self.processed_files as f32
    }
}

pub struct AnalysisCache {
    root: PathBuf,
    facts_dir: PathBuf,
}

impl AnalysisCache {
    pub fn new(root: &Path, cache_dir: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            facts_dir: cache_dir.join("facts"),
        }
    }

    pub fn ensure_ready(&self) -> Result<()> {
        fs::create_dir_all(&self.facts_dir).with_context(|| {
            format!(
                "failed to create facts cache dir {}",
                self.facts_dir.display()
            )
        })
    }

    pub fn load(&self, rel_path: &str, mtime: u64, size: u64) -> Result<Option<FileFacts>> {
        let cache_path = self.cache_path_for(rel_path);
        let bytes = match fs::read(&cache_path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", cache_path.display()));
            }
        };

        let parsed: FileFacts = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse cached facts {}", cache_path.display()))?;

        if parsed.version != FACTS_SCHEMA_VERSION
            || parsed.file != rel_path
            || parsed.mtime != mtime
            || parsed.size != size
        {
            return Ok(None);
        }

        Ok(Some(parsed))
    }

    pub fn store(&self, facts: &FileFacts) -> Result<()> {
        let cache_path = self.cache_path_for(&facts.file);
        let tmp = cache_path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(facts)?;
        fs::write(&tmp, &bytes).with_context(|| format!("failed to write {}", tmp.display()))?;
        fs::rename(&tmp, &cache_path).with_context(|| {
            format!(
                "failed to rename {} -> {}",
                tmp.display(),
                cache_path.display()
            )
        })?;
        Ok(())
    }

    pub fn abs_path(&self, rel_path: &str) -> PathBuf {
        self.root.join(rel_path)
    }

    fn cache_path_for(&self, rel_path: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        rel_path.hash(&mut hasher);
        let digest = hasher.finish();
        self.facts_dir.join(format!("{:016x}.json", digest))
    }
}

pub struct GraphCache {
    root: PathBuf,
    graphs_dir: PathBuf,
    graphs_path: PathBuf,
}

#[derive(Debug, Default, Clone)]
pub struct GraphCacheStats {
    pub scanned_files: usize,
    pub updated_files: usize,
    pub removed_files: usize,
    pub parse_failures: usize,
}

#[derive(Debug, Clone)]
pub struct RepoGraphs {
    pub call_graph: CallGraphIndex,
    pub type_graph: TypeGraphIndex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphCacheData {
    version: u32,
    files: HashMap<String, CachedFileGraphs>,
    call_graph: CallGraphIndex,
    type_graph: TypeGraphIndex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedFileGraphs {
    mtime: u64,
    size: u64,
    content_hash: u64,
    call_edges: Vec<CallEdge>,
    type_edges: Vec<TypeRelationEdge>,
}

const GRAPH_CACHE_VERSION: u32 = 1;

impl GraphCache {
    pub fn new(root: &Path, cache_dir: &Path) -> Self {
        let graphs_dir = cache_dir.join("graphs");
        let graphs_path = graphs_dir.join("analysis_graphs_v1.bin");
        Self {
            root: root.to_path_buf(),
            graphs_dir,
            graphs_path,
        }
    }

    pub fn load_or_update(
        &self,
        options: &ExtractFactsOptions,
    ) -> Result<(GraphCacheStats, RepoGraphs, Vec<String>)> {
        self.ensure_ready()?;
        let mut data = self.load()?.unwrap_or_else(GraphCacheData::empty);

        let (stats, warnings) = self.update_from_repo(&mut data, options)?;
        self.store(&data)?;

        Ok((
            stats,
            RepoGraphs {
                call_graph: data.call_graph,
                type_graph: data.type_graph,
            },
            warnings,
        ))
    }

    fn ensure_ready(&self) -> Result<()> {
        fs::create_dir_all(&self.graphs_dir).with_context(|| {
            format!(
                "failed to create analysis graphs cache dir {}",
                self.graphs_dir.display()
            )
        })
    }

    fn load(&self) -> Result<Option<GraphCacheData>> {
        let bytes = match fs::read(&self.graphs_path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to read {}", self.graphs_path.display()));
            }
        };

        let parsed: GraphCacheData = bincode::deserialize(&bytes)
            .with_context(|| format!("failed to deserialize {}", self.graphs_path.display()))?;
        if parsed.version != GRAPH_CACHE_VERSION {
            return Ok(None);
        }
        Ok(Some(parsed))
    }

    fn store(&self, data: &GraphCacheData) -> Result<()> {
        let tmp = self.graphs_path.with_extension("bin.tmp");
        let bytes =
            bincode::serialize(data).context("failed to serialize analysis graphs cache")?;
        fs::write(&tmp, &bytes).with_context(|| format!("failed to write {}", tmp.display()))?;
        fs::rename(&tmp, &self.graphs_path).with_context(|| {
            format!(
                "failed to rename {} -> {}",
                tmp.display(),
                self.graphs_path.display()
            )
        })?;
        Ok(())
    }

    fn update_from_repo(
        &self,
        data: &mut GraphCacheData,
        options: &ExtractFactsOptions,
    ) -> Result<(GraphCacheStats, Vec<String>)> {
        let mut warnings: Vec<String> = Vec::new();
        let mut stats = GraphCacheStats::default();

        let files = if let Some(scope_file) = &options.scope_file {
            vec![make_absolute(&self.root, scope_file)]
        } else {
            collect_files(&self.root)?
        };

        let candidates: Vec<FileCandidate> = files
            .into_par_iter()
            .filter_map(|abs_path| {
                if !passes_extension_filter(&abs_path, options.extension_filter.as_deref()) {
                    return None;
                }
                let language = detect_language_from_path(&abs_path)?;
                if !matches!(
                    language,
                    "rust" | "swift" | "typescript" | "tsx" | "go" | "python"
                ) {
                    return None;
                }
                let rel_path = rel_path_str(&self.root, &abs_path);
                let meta = fs::metadata(&abs_path).ok()?;
                let size = meta.len();
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|dur| dur.as_secs())
                    .unwrap_or(0);

                Some(FileCandidate {
                    abs_path,
                    rel_path,
                    language: language.to_string(),
                    mtime,
                    size,
                })
            })
            .collect();

        stats.scanned_files = candidates.len();
        let current: HashSet<String> = candidates.iter().map(|c| c.rel_path.clone()).collect();

        let removed: Vec<String> = data
            .files
            .keys()
            .filter(|rel| !current.contains(*rel))
            .cloned()
            .collect();
        for rel in removed {
            if let Some(old) = data.files.remove(&rel) {
                data.call_graph.remove_edges(old.call_edges);
                data.type_graph.remove_edges(old.type_edges);
                stats.removed_files += 1;
            }
        }

        let cached_meta: HashMap<String, CachedFileMeta> = data
            .files
            .iter()
            .map(|(rel, entry)| {
                (
                    rel.clone(),
                    CachedFileMeta {
                        mtime: entry.mtime,
                        size: entry.size,
                        content_hash: entry.content_hash,
                    },
                )
            })
            .collect();

        let outcomes: Vec<FileOutcome> = candidates
            .into_par_iter()
            .map(|candidate| {
                let cached = cached_meta.get(&candidate.rel_path).copied();
                self.process_candidate(cached, candidate)
            })
            .collect();

        for outcome in outcomes {
            match outcome {
                FileOutcome::Unchanged => {}
                FileOutcome::MetaOnly {
                    rel_path,
                    mtime,
                    size,
                    content_hash,
                } => {
                    if let Some(entry) = data.files.get_mut(&rel_path) {
                        entry.mtime = mtime;
                        entry.size = size;
                        entry.content_hash = content_hash;
                    }
                }
                FileOutcome::Updated { rel_path, entry } => {
                    if let Some(old) = data.files.insert(rel_path.clone(), entry.clone()) {
                        data.call_graph.remove_edges(old.call_edges);
                        data.type_graph.remove_edges(old.type_edges);
                    }
                    data.call_graph.add_edges(entry.call_edges.clone());
                    data.type_graph.add_edges(entry.type_edges.clone());
                    stats.updated_files += 1;
                }
                FileOutcome::Failed { rel_path, warning } => {
                    let _ = rel_path;
                    stats.parse_failures += 1;
                    warnings.push(warning);
                }
            }
        }

        Ok((stats, warnings))
    }

    fn process_candidate(
        &self,
        cached: Option<CachedFileMeta>,
        candidate: FileCandidate,
    ) -> FileOutcome {
        if let Some(cached) = cached {
            if cached.mtime == candidate.mtime && cached.size == candidate.size {
                return FileOutcome::Unchanged;
            }
        }

        let bytes = match fs::read(&candidate.abs_path) {
            Ok(bytes) => bytes,
            Err(err) => {
                return FileOutcome::Failed {
                    rel_path: candidate.rel_path,
                    warning: format!(
                        "analysis graph: failed to read {}: {}",
                        candidate.abs_path.display(),
                        err
                    ),
                };
            }
        };
        let content_hash = content_hash64(&bytes);

        if let Some(cached) = cached {
            if cached.content_hash == content_hash {
                return FileOutcome::MetaOnly {
                    rel_path: candidate.rel_path,
                    mtime: candidate.mtime,
                    size: candidate.size,
                    content_hash,
                };
            }
        }

        let source = match String::from_utf8(bytes) {
            Ok(source) => source,
            Err(_) => {
                return FileOutcome::Failed {
                    rel_path: candidate.rel_path,
                    warning: format!(
                        "analysis graph: skipped non-utf8 file {}",
                        candidate.abs_path.display()
                    ),
                };
            }
        };

        let facts = match extract_file_facts(&candidate.language, &candidate.rel_path, &source) {
            Ok(facts) => facts,
            Err(err) => {
                return FileOutcome::Failed {
                    rel_path: candidate.rel_path,
                    warning: format!(
                        "analysis graph: failed to parse {}: {err}",
                        candidate.abs_path.display()
                    ),
                };
            }
        };

        let call_edges: Vec<CallEdge> = facts
            .calls
            .into_iter()
            .map(|call| CallEdge {
                caller: call.caller,
                callee: call.callee,
                file: candidate.rel_path.clone(),
                line: call.line,
            })
            .collect();
        let type_edges: Vec<TypeRelationEdge> = facts
            .relations
            .into_iter()
            .map(|rel| TypeRelationEdge {
                from: rel.from,
                to: rel.to,
                kind: rel.kind,
                file: candidate.rel_path.clone(),
                line: rel.line,
            })
            .collect();

        FileOutcome::Updated {
            rel_path: candidate.rel_path,
            entry: CachedFileGraphs {
                mtime: candidate.mtime,
                size: candidate.size,
                content_hash,
                call_edges,
                type_edges,
            },
        }
    }
}

impl GraphCacheData {
    fn empty() -> Self {
        Self {
            version: GRAPH_CACHE_VERSION,
            files: HashMap::new(),
            call_graph: CallGraphIndex::default(),
            type_graph: TypeGraphIndex::default(),
        }
    }
}

struct FileCandidate {
    abs_path: PathBuf,
    rel_path: String,
    language: String,
    mtime: u64,
    size: u64,
}

#[derive(Clone, Copy)]
struct CachedFileMeta {
    mtime: u64,
    size: u64,
    content_hash: u64,
}

enum FileOutcome {
    Unchanged,
    MetaOnly {
        rel_path: String,
        mtime: u64,
        size: u64,
        content_hash: u64,
    },
    Updated {
        rel_path: String,
        entry: CachedFileGraphs,
    },
    Failed {
        rel_path: String,
        warning: String,
    },
}

fn extract_file_facts(
    language: &str,
    file: &str,
    source: &str,
) -> Result<super::facts::FactBundle> {
    match language {
        "rust" => super::rust::extract(file, source),
        "swift" => super::swift::extract(file, source),
        "typescript" | "tsx" => super::typescript::extract(file, source, language),
        "go" => super::go::extract(file, source),
        "python" => super::python::extract(file, source),
        other => anyhow::bail!("unsupported language for analysis graph extraction: {other}"),
    }
}

fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .ignore(true)
        .parents(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false);

    let mut files = Vec::new();
    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            files.push(entry.into_path());
        }
    }
    Ok(files)
}

fn make_absolute(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn rel_path_str(root: &Path, abs: &Path) -> String {
    abs.strip_prefix(root)
        .unwrap_or(abs)
        .to_string_lossy()
        .replace('\\', "/")
}

fn content_hash64(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}
