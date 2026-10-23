use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::file_identity::{FileIdentity, read_snapshot};
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

// Cache snapshots may be written by multiple CLI invocations or service requests.
// Unique same-directory temporaries provide atomic replacement without shared .tmp races.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .context("cache file has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).with_context(|| {
        format!(
            "failed to create temporary cache file in {}",
            parent.display()
        )
    })?;
    temporary.write_all(bytes)?;
    temporary
        .persist(path)
        .with_context(|| format!("failed to replace cache file {}", path.display()))?;
    Ok(())
}

pub struct AnalysisCache {
    root: PathBuf,
    facts_dir: PathBuf,
}

// Cache-only envelope: keep content fingerprints out of public FileFacts JSON.
#[derive(Serialize, Deserialize)]
pub(super) struct CachedFacts {
    identity: FileIdentity,
    pub(super) content_hash: u64,
    pub(super) facts: FileFacts,
}

impl CachedFacts {
    pub(super) fn reusable_with(&self, identity: &FileIdentity) -> bool {
        self.identity.reusable_with(identity)
    }
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

    pub(super) fn load(&self, rel_path: &str) -> Result<Option<CachedFacts>> {
        let cache_path = self.cache_path_for(rel_path);
        let bytes = match fs::read(&cache_path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to read {}", cache_path.display()));
            }
        };
        let cached: CachedFacts = match serde_json::from_slice(&bytes) {
            Ok(cached) => cached,
            Err(_) => return Ok(None),
        };
        if !cached.identity.is_current_version()
            || cached.facts.version != FACTS_SCHEMA_VERSION
            || cached.facts.file != rel_path
        {
            return Ok(None);
        }
        Ok(Some(cached))
    }

    pub(super) fn store(
        &self,
        facts: &FileFacts,
        identity: FileIdentity,
        content_hash: u64,
    ) -> Result<()> {
        let bytes = serde_json::to_vec(&CachedFacts {
            identity,
            content_hash,
            facts: facts.clone(),
        })?;
        atomic_write(&self.cache_path_for(&facts.file), &bytes)
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
    // Derived indexes are rebuilt from the persisted per-file edges.
    #[serde(skip)]
    call_graph: CallGraphIndex,
    #[serde(skip)]
    type_graph: TypeGraphIndex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedFileGraphs {
    identity: FileIdentity,
    content_hash: u64,
    #[serde(with = "cached_call_edges")]
    call_edges: Vec<CallEdge>,
    type_edges: Vec<TypeRelationEdge>,
}

// CallEdge's JSON representation omits absent fields. Bincode requires every
// positional field, including None, so use an independent cache-only wire type.
mod cached_call_edges {
    use super::CallEdge;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    #[serde(remote = "CallEdge")]
    struct WireEdge {
        caller: String,
        callee: String,
        file: String,
        line: usize,
        caller_signature: Option<String>,
        callee_signature: Option<String>,
        call_context: Option<String>,
    }

    #[derive(Serialize, Deserialize)]
    struct Edge(#[serde(with = "WireEdge")] CallEdge);

    pub fn serialize<S: Serializer>(edges: &[CallEdge], serializer: S) -> Result<S::Ok, S::Error> {
        edges
            .iter()
            .cloned()
            .map(Edge)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<CallEdge>, D::Error> {
        Ok(Vec::<Edge>::deserialize(deserializer)?
            .into_iter()
            .map(|edge| edge.0)
            .collect())
    }
}

const GRAPH_CACHE_VERSION: u32 = 3;

impl GraphCache {
    pub fn new(root: &Path, cache_dir: &Path) -> Self {
        let graphs_dir = cache_dir.join("graphs");
        let graphs_path = graphs_dir.join("analysis_graphs_v3.bin");
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

        let mut parsed: GraphCacheData = match bincode::deserialize(&bytes) {
            Ok(data) => data,
            Err(_) => {
                // Disposable cache: do not unlink a concurrent writer's replacement.
                return Ok(None);
            }
        };
        if parsed.version != GRAPH_CACHE_VERSION
            || parsed
                .files
                .values()
                .any(|entry| !entry.identity.is_current_version())
        {
            // Version mismatch; rebuild without unlinking a concurrent replacement.
            return Ok(None);
        }
        for entry in parsed.files.values() {
            parsed.call_graph.add_edges(entry.call_edges.clone());
            parsed.type_graph.add_edges(entry.type_edges.clone());
        }
        Ok(Some(parsed))
    }

    fn store(&self, data: &GraphCacheData) -> Result<()> {
        let bytes =
            bincode::serialize(data).context("failed to serialize analysis graphs cache")?;
        atomic_write(&self.graphs_path, &bytes)?;
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
                let identity = FileIdentity::from_metadata(&meta);

                Some(FileCandidate {
                    abs_path,
                    rel_path,
                    language: language.to_string(),
                    identity,
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
                        identity: entry.identity,
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
                    identity,
                    content_hash,
                } => {
                    if let Some(entry) = data.files.get_mut(&rel_path) {
                        entry.identity = identity;
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
                    // Never leave known-dirty edges live after a read/parse failure.
                    if let Some(old) = data.files.remove(&rel_path) {
                        data.call_graph.remove_edges(old.call_edges);
                        data.type_graph.remove_edges(old.type_edges);
                    }
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
        if cached.is_some_and(|old| old.identity.reusable_with(&candidate.identity)) {
            return FileOutcome::Unchanged;
        }
        let (bytes, stable_identity) = match read_snapshot(&candidate.abs_path, candidate.identity)
        {
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
        let Some(identity) = stable_identity else {
            return FileOutcome::Failed {
                rel_path: candidate.rel_path,
                warning: format!(
                    "analysis graph: source changed while reading {}",
                    candidate.abs_path.display()
                ),
            };
        };
        let content_hash = content_hash64(&bytes);

        if let Some(cached) = cached {
            if cached.content_hash == content_hash {
                return FileOutcome::MetaOnly {
                    rel_path: candidate.rel_path,
                    identity,
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

        let normalized_lines: Vec<String> = source
            .lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        let mut signature_index: HashMap<String, String> = HashMap::new();
        for def in &facts.definitions {
            let sig = def
                .signature
                .as_deref()
                .map(|s| s.to_string())
                .or_else(|| normalized_lines.get(def.line.saturating_sub(1)).cloned());
            let Some(sig) = sig else { continue };
            let key = super::callgraph::normalize_symbol(&def.symbol);
            signature_index
                .entry(def.symbol.clone())
                .or_insert_with(|| sig.clone());
            signature_index.entry(key).or_insert_with(|| sig.clone());
            if let Some(simple) = def.symbol.rsplit("::").next() {
                signature_index
                    .entry(simple.to_string())
                    .or_insert_with(|| sig.clone());
            }
        }

        let call_edges: Vec<CallEdge> = facts
            .calls
            .into_iter()
            .map(|call| {
                let caller = call.caller;
                let callee = call.callee;
                CallEdge {
                    caller_signature: signature_index.get(&caller).cloned().or_else(|| {
                        signature_index
                            .get(&super::callgraph::normalize_symbol(&caller))
                            .cloned()
                    }),
                    callee_signature: signature_index.get(&callee).cloned().or_else(|| {
                        signature_index
                            .get(&super::callgraph::normalize_symbol(&callee))
                            .cloned()
                    }),
                    call_context: normalized_lines.get(call.line.saturating_sub(1)).cloned(),
                    caller,
                    callee,
                    file: candidate.rel_path.clone(),
                    line: call.line,
                }
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
                identity,
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
    identity: FileIdentity,
}

#[derive(Clone, Copy)]
struct CachedFileMeta {
    identity: FileIdentity,
    content_hash: u64,
}

enum FileOutcome {
    Unchanged,
    MetaOnly {
        rel_path: String,
        identity: FileIdentity,
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

pub(super) fn content_hash64(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::super::extract::extract_repo_facts;
    use super::*;
    use std::time::Duration;

    fn write_source(path: &Path, source: &str) {
        fs::write(path, source).unwrap();
        // Deliberately preserve the exact timestamp, not just its seconds component.
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(1_700_000_000))
            .unwrap();
    }

    #[test]
    fn facts_cache_detects_same_size_same_mtime_edits() {
        let repo = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let path = repo.path().join("sample.rs");
        let options = ExtractFactsOptions::for_search(Some(path.clone()), None);
        write_source(&path, "fn alpha() {}\n");
        let (cold, facts) = extract_repo_facts(repo.path(), cache_dir.path(), &options).unwrap();
        assert_eq!(cold.processed_files, 1);
        assert_eq!(cold.cache_hits, 0);
        assert_eq!(facts[0].facts.definitions[0].symbol, "alpha");
        let (warm, _) = extract_repo_facts(repo.path(), cache_dir.path(), &options).unwrap();
        assert_eq!(warm.cache_hits, 1);

        write_source(&path, "fn bravo() {}\n");
        let (changed, facts) = extract_repo_facts(repo.path(), cache_dir.path(), &options).unwrap();
        assert_eq!(changed.cache_hits, 0);
        assert_eq!(facts[0].facts.definitions[0].symbol, "bravo");
        let (warm, _) = extract_repo_facts(repo.path(), cache_dir.path(), &options).unwrap();
        assert_eq!(warm.cache_hits, 1);
    }

    #[test]
    fn corrupt_and_legacy_facts_are_cache_misses() {
        let repo = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let path = repo.path().join("sample.rs");
        let options = ExtractFactsOptions::for_search(Some(path.clone()), None);
        write_source(&path, "fn alpha() {}\n");
        let (_, facts) = extract_repo_facts(repo.path(), cache_dir.path(), &options).unwrap();
        let cache = AnalysisCache::new(repo.path(), cache_dir.path());
        let cache_path = cache.cache_path_for("sample.rs");
        for invalid in [
            b"{truncated".to_vec(),
            serde_json::to_vec(&facts[0]).unwrap(),
        ] {
            fs::write(&cache_path, invalid).unwrap();
            let (stats, rebuilt) =
                extract_repo_facts(repo.path(), cache_dir.path(), &options).unwrap();
            assert_eq!(stats.cache_hits, 0);
            assert_eq!(rebuilt[0].facts.definitions[0].symbol, "alpha");
            let (warm, _) = extract_repo_facts(repo.path(), cache_dir.path(), &options).unwrap();
            assert_eq!(warm.cache_hits, 1);
        }
    }

    #[test]
    fn graph_cache_detects_same_size_same_mtime_edits() {
        let repo = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let path = repo.path().join("sample.rs");
        let options = ExtractFactsOptions::for_search(Some(path.clone()), None);
        let cache = GraphCache::new(repo.path(), cache_dir.path());
        write_source(&path, "fn entry() { alpha(); }\n");
        let (cold, graphs, warnings) = cache.load_or_update(&options).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(cold.updated_files, 1);
        assert_eq!(graphs.call_graph.caller_edges_for("alpha").len(), 1);
        let (warm, graphs, _) = cache.load_or_update(&options).unwrap();
        assert_eq!(warm.updated_files, 0);
        assert_eq!(graphs.call_graph.caller_edges_for("alpha").len(), 1);

        write_source(&path, "fn entry() { bravo(); }\n");
        let (changed, graphs, warnings) = cache.load_or_update(&options).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(changed.updated_files, 1);
        assert!(graphs.call_graph.caller_edges_for("alpha").is_empty());
        assert_eq!(graphs.call_graph.caller_edges_for("bravo").len(), 1);
        let (warm, graphs, _) = cache.load_or_update(&options).unwrap();
        assert_eq!(warm.updated_files, 0);
        assert_eq!(graphs.call_graph.caller_edges_for("bravo").len(), 1);
    }

    #[test]
    fn warm_facts_and_graphs_skip_source_reads() {
        let repo = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let path = repo.path().join("sample.rs");
        write_source(&path, "fn entry() { alpha(); }\n");
        let identity = FileIdentity::read(&path).unwrap();
        let options = ExtractFactsOptions::for_search(Some(path.clone()), None);
        let graphs = GraphCache::new(repo.path(), cache_dir.path());
        extract_repo_facts(repo.path(), cache_dir.path(), &options).unwrap();
        graphs.load_or_update(&options).unwrap();
        assert_eq!(crate::file_identity::source_reads(&path), 2);
        let (stats, _) = extract_repo_facts(repo.path(), cache_dir.path(), &options).unwrap();
        let (graph_stats, _, _) = graphs.load_or_update(&options).unwrap();
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(graph_stats.updated_files, 0);
        assert_eq!(
            crate::file_identity::source_reads(&path),
            if identity.reliable() { 2 } else { 4 }
        );

        // A metadata-only change is hashed once but does not reparse.
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(1_700_000_001))
            .unwrap();
        let before = crate::file_identity::source_reads(&path);
        assert_eq!(
            extract_repo_facts(repo.path(), cache_dir.path(), &options)
                .unwrap()
                .0
                .cache_hits,
            1
        );
        assert_eq!(graphs.load_or_update(&options).unwrap().0.updated_files, 0);
        assert_eq!(crate::file_identity::source_reads(&path), before + 2);
    }

    #[test]
    fn atomic_replacement_and_deletion_invalidate_facts_and_graphs() {
        let repo = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let path = repo.path().join("sample.rs");
        let options = ExtractFactsOptions::for_search(Some(path.clone()), None);
        let cache = GraphCache::new(repo.path(), cache_dir.path());
        write_source(&path, "fn entry() { alpha(); }\n");
        extract_repo_facts(repo.path(), cache_dir.path(), &options).unwrap();
        cache.load_or_update(&options).unwrap();
        let replacement = repo.path().join("replacement");
        write_source(&replacement, "fn entry() { bravo(); }\n");
        fs::rename(&replacement, &path).unwrap();
        let (stats, facts) = extract_repo_facts(repo.path(), cache_dir.path(), &options).unwrap();
        assert_eq!(stats.cache_hits, 0);
        assert_eq!(facts[0].facts.calls[0].callee, "bravo");
        let (stats, graphs, _) = cache.load_or_update(&options).unwrap();
        assert_eq!(stats.updated_files, 1);
        assert!(graphs.call_graph.caller_edges_for("alpha").is_empty());
        assert_eq!(graphs.call_graph.caller_edges_for("bravo").len(), 1);
        fs::remove_file(&path).unwrap();
        assert!(
            extract_repo_facts(repo.path(), cache_dir.path(), &options)
                .unwrap()
                .1
                .is_empty()
        );
        let (stats, graphs, _) = cache.load_or_update(&options).unwrap();
        assert_eq!(stats.removed_files, 1);
        assert!(graphs.call_graph.caller_edges_for("bravo").is_empty());
    }

    #[test]
    fn changed_stat_declines_graph_snapshot_and_facts_identity() {
        let repo = tempfile::tempdir().unwrap();
        let path = repo.path().join("sample.rs");
        write_source(&path, "fn entry() { alpha(); }\n");
        let before = FileIdentity::read(&path).unwrap();
        fs::write(&path, "fn different_size() {}\n").unwrap();
        let (_, identity) = read_snapshot(&path, before).unwrap();
        assert!(identity.is_none());
        let cache = GraphCache::new(repo.path(), repo.path());
        let outcome = cache.process_candidate(
            None,
            FileCandidate {
                abs_path: path,
                rel_path: "sample.rs".into(),
                language: "rust".into(),
                identity: before,
            },
        );
        assert!(matches!(outcome, FileOutcome::Failed { .. }));
    }

    #[test]
    fn corrupt_and_legacy_graphs_rebuild() {
        let repo = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let path = repo.path().join("sample.rs");
        write_source(&path, "fn entry() { alpha(); }\n");
        let options = ExtractFactsOptions::for_search(Some(path), None);
        let cache = GraphCache::new(repo.path(), cache_dir.path());
        cache.load_or_update(&options).unwrap();
        let mut legacy = cache.load().unwrap().unwrap();
        legacy.version = GRAPH_CACHE_VERSION - 1;
        for bytes in [b"truncated".to_vec(), bincode::serialize(&legacy).unwrap()] {
            fs::write(&cache.graphs_path, bytes).unwrap();
            let (stats, graphs, warnings) = cache.load_or_update(&options).unwrap();
            assert!(warnings.is_empty());
            assert_eq!(stats.updated_files, 1);
            assert_eq!(graphs.call_graph.caller_edges_for("alpha").len(), 1);
            assert!(cache.load().unwrap().is_some());
        }
    }

    #[test]
    fn concurrent_cache_writers_use_independent_temporaries() {
        let repo = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let path = repo.path().join("sample.rs");
        write_source(&path, "fn entry() { alpha(); }\n");
        let options = ExtractFactsOptions::for_search(Some(path), None);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let (repo, cache_dir, options) = (&repo, &cache_dir, &options);
                scope.spawn(move || {
                    extract_repo_facts(repo.path(), cache_dir.path(), options).unwrap();
                    GraphCache::new(repo.path(), cache_dir.path())
                        .load_or_update(options)
                        .unwrap();
                });
            }
        });
        let (stats, facts) = extract_repo_facts(repo.path(), cache_dir.path(), &options).unwrap();
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(facts.len(), 1);
        let cache = GraphCache::new(repo.path(), cache_dir.path());
        assert!(cache.load().unwrap().is_some());
        for directory in [
            cache_dir.path().join("facts"),
            cache_dir.path().join("graphs"),
        ] {
            assert_eq!(fs::read_dir(directory).unwrap().count(), 1);
        }
    }

    #[test]
    fn graph_cache_round_trips_all_optional_edge_fields_without_changing_json() {
        let repo = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let cache = GraphCache::new(repo.path(), cache_dir.path());
        cache.ensure_ready().unwrap();
        let edges: Vec<_> = (0..8)
            .map(|mask| CallEdge {
                caller: "entry".into(),
                callee: "external".into(),
                file: "sample.rs".into(),
                line: mask + 1,
                caller_signature: (mask & 1 != 0).then(|| "fn entry()".into()),
                callee_signature: (mask & 2 != 0).then(|| "fn external()".into()),
                call_context: (mask & 4 != 0).then(|| "external();".into()),
            })
            .collect();
        let mut data = GraphCacheData::empty();
        data.files.insert(
            "sample.rs".into(),
            CachedFileGraphs {
                identity: FileIdentity::read(repo.path()).unwrap(),
                content_hash: 1,
                call_edges: edges.clone(),
                type_edges: vec![],
            },
        );
        cache.store(&data).unwrap();
        let loaded = cache
            .load()
            .unwrap()
            .expect("binary cache should deserialize");
        let actual = &loaded.files["sample.rs"].call_edges;
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(&edges).unwrap()
        );
        assert_eq!(loaded.call_graph.caller_edges_for("external").len(), 8);
        assert_eq!(loaded.call_graph.callee_edges_for("entry").len(), 8);
        let json = serde_json::to_value(&edges[0]).unwrap();
        assert!(json.get("caller_signature").is_none());
        assert!(json.get("callee_signature").is_none());
        assert!(json.get("call_context").is_none());
    }
}
