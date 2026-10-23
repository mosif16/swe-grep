use std::collections::BTreeMap;
use std::fs;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, ensure};
use ignore::WalkBuilder;
use tantivy::collector::TopDocs;
use tantivy::directory::{Directory, Lock, MmapDirectory};
use tantivy::schema::{STORED, STRING, Schema, SchemaBuilder, TEXT};
use tantivy::{Index, IndexReader, ReloadPolicy, Term};
use tokio::task;

#[derive(Clone)]
pub struct TantivyIndex {
    #[allow(dead_code)]
    index: Index,
    reader: IndexReader,
    query_parser: tantivy::query::QueryParser,
    path_field: tantivy::schema::Field,
    root: PathBuf,
    stats: IndexStats,
}

/// Work performed by this particular open (not process-global counters).
#[derive(Clone, Debug, Default)]
pub struct IndexStats {
    pub source_reads: usize,
    pub hashed_files: usize,
    pub indexed_documents: usize,
    pub deleted_documents: usize,
    pub commits: usize,
    pub writer_acquisitions: usize,
}

#[derive(Clone, Debug)]
pub struct IndexConfig {
    pub root: PathBuf,
    pub index_dir: PathBuf,
    pub extensions: Option<Vec<String>>,
}

impl TantivyIndex {
    /// Refresh a persistent snapshot. Warm opens walk/stat sources but neither read their
    /// contents nor acquire a writer on filesystems with reliable Unix identities.
    /// Changed metadata candidates alone are read/hashed; unsupported/coarse identities
    /// conservatively hash content instead of trusting restored write timestamps.
    /// The versioned manifest is the Tantivy commit payload: documents and identities are
    /// published by the same atomic metadata replacement, including after a crash.
    ///
    /// Sources are a best-effort snapshot, not a filesystem transaction. Concurrent edits
    /// observed while reading abort the refresh rather than recording an identity for stale
    /// content. Reopen to observe edits after that snapshot. Non-UTF-8 paths, unreadable or
    /// non-UTF-8 content, symlinks, and this index directory are excluded.
    pub async fn open_or_build(config: IndexConfig) -> Result<Self> {
        task::spawn_blocking(move || Self::open_blocking(config))
            .await
            .context("index open task cancelled")?
    }

    fn open_blocking(config: IndexConfig) -> Result<Self> {
        ensure!(
            !fs::symlink_metadata(&config.root)?.file_type().is_symlink(),
            "source root must not be a symlink"
        );
        let root = fs::canonicalize(&config.root).context("failed to resolve source root")?;
        ensure!(root.is_dir(), "source root must be a directory");
        if let Ok(metadata) = fs::symlink_metadata(&config.index_dir) {
            ensure!(
                !metadata.file_type().is_symlink(),
                "index directory must not be a symlink"
            );
        }
        fs::create_dir_all(&config.index_dir)?;
        let index_dir = fs::canonicalize(&config.index_dir)?;
        ensure!(
            !root.starts_with(&index_dir),
            "index directory must not contain the source root"
        );
        let mut extensions = config.extensions.unwrap_or_default();
        extensions.iter_mut().for_each(|s| s.make_ascii_lowercase());
        extensions.sort();
        extensions.dedup();
        // Use the full normalized configuration, not a collision-prone hash of it.
        let fingerprint = config_fingerprint(&root, &extensions)?;
        let index = open_managed_index(&index_dir)?;
        let stats = refresh(&index, &root, &index_dir, &extensions, &fingerprint)?;
        let reader: IndexReader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommit)
            .try_into()
            .context("failed to create index reader")?;
        reader.reload()?;
        let schema = index.schema();
        let path_field = schema.get_field("path").context("path field missing")?;
        let body_field = schema.get_field("body").context("body field missing")?;
        let query_parser = tantivy::query::QueryParser::for_index(&index, vec![body_field]);
        Ok(Self {
            index,
            reader,
            query_parser,
            path_field,
            root,
            stats,
        })
    }

    pub fn stats(&self) -> &IndexStats {
        &self.stats
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<PathBuf>> {
        if limit == 0 || query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let query_string = query.to_string();
        let parser = self.query_parser.clone();
        let reader = self.reader.clone();
        let path_field = self.path_field;
        let root = self.root.clone();

        task::spawn_blocking(move || {
            let searcher = reader.searcher();
            let query = parser
                .parse_query(&query_string)
                .with_context(|| format!("failed to parse tantivy query `{query_string}`"))?;
            let top_docs = searcher
                .search(&query, &TopDocs::with_limit(limit))
                .context("tantivy search failed")?;

            let mut results = Vec::new();
            for (_score, doc_address) in top_docs {
                let retrieved = searcher.doc(doc_address)?;
                if let Some(value) = retrieved.get_first(path_field) {
                    let text = value.as_text().unwrap_or_default();
                    let path = PathBuf::from(text);
                    results.push(normalize_path(&root, &path));
                }
            }
            Ok::<Vec<PathBuf>, anyhow::Error>(results)
        })
        .await
        .context("tantivy search task cancelled")?
    }
}

fn build_schema() -> Schema {
    let mut builder = SchemaBuilder::default();
    // Preserve canonical stored paths and keep deletion keys untokenized.
    builder.add_text_field("path", STORED);
    builder.add_text_field("body", TEXT);
    builder.add_text_field("exact_path", STRING);
    builder.build()
}

/// Legacy indexes (whose path was only STORED) are never mutated or recursively removed.
/// A new, exclusively created child owns the new schema. Corrupt/incompatible children are
/// retained too; numbered fallbacks permit recovery without deleting arbitrary user files.
fn open_managed_index(index_dir: &Path) -> Result<Index> {
    fn existing(path: &Path) -> Option<Index> {
        let metadata = fs::symlink_metadata(path).ok()?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return None;
        }
        let marker = path.join(".swe-grep-owner");
        if fs::symlink_metadata(&marker).ok()?.file_type().is_symlink()
            || fs::read(&marker).ok()?.as_slice() != b"swe-grep-index-v2\n"
        {
            return None;
        }
        let index = Index::open_in_dir(path).ok()?;
        (index.schema() == build_schema()).then_some(index)
    }
    let parent = MmapDirectory::open(index_dir)?;
    let mut init_guard = None;
    let mut generation = 0u64;
    loop {
        let path = index_dir.join(format!(".swe-grep-v2-{generation}"));
        if let Some(index) = existing(&path) {
            return Ok(index);
        }
        if fs::symlink_metadata(&path).is_ok() {
            generation += 1;
            continue;
        }
        if init_guard.is_none() {
            init_guard = Some(parent.acquire_lock(&Lock {
                filepath: PathBuf::from(".swe-grep-init.lock"),
                is_blocking: true,
            })?);
            // Restart from zero: another opener may have seen an earlier child
            // half-created, then finished initializing it while we waited.
            generation = 0;
            continue;
        }
        fs::create_dir(&path)?;
        let index = Index::create_in_dir(&path, build_schema())
            .context("failed to create managed index")?;
        fs::write(path.join(".swe-grep-owner"), b"swe-grep-index-v2\n")?;
        return Ok(index);
    }
}

// All fields are high-resolution; ctime catches same-length writes with restored mtime on Unix.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity([u64; 7]);

impl Identity {
    fn reusable_with(&self, current: &Self) -> bool {
        // Restored write timestamps cannot be trusted without change-time/file identity.
        // Match the analysis cache: unsupported/coarse metadata takes the hash path.
        cfg!(unix)
            && self == current
            && self.0[4] > 0
            && self.0[4] < 1_000_000_000
            && self.0[6] != 0
    }
}

fn identity(path: &Path) -> Result<Option<Identity>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Ok(None);
    }
    let (seconds, nanos) = match metadata.modified()?.duration_since(UNIX_EPOCH) {
        Ok(time) => (time.as_secs(), time.subsec_nanos() as u64),
        Err(error) => {
            let time = error.duration();
            // Encode pre-epoch values without collapsing their nanosecond precision.
            (
                time.as_secs().wrapping_neg(),
                (time.subsec_nanos() as u64) | (1 << 63),
            )
        }
    };
    let mut values = [seconds, nanos, metadata.len(), 0, 0, 0, 0];
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        values[3] = metadata.ctime() as u64;
        values[4] = metadata.ctime_nsec() as u64;
        values[5] = metadata.dev();
        values[6] = metadata.ino();
    }
    Ok(Some(Identity(values)))
}

type Snapshot = BTreeMap<String, Identity>;

fn scan(root: &Path, index_dir: &Path, extensions: &[String]) -> Result<Snapshot> {
    let excluded = index_dir.to_path_buf();
    let mut walker = WalkBuilder::new(root);
    walker
        .standard_filters(true)
        .follow_links(false)
        .filter_entry(move |entry| {
            !entry.path().starts_with(&excluded)
                && !entry.file_type().map(|ft| ft.is_symlink()).unwrap_or(false)
        });
    let mut result = BTreeMap::new();
    for entry in walker.build() {
        let entry = entry.context("failed to walk sources")?;
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        if !extensions.is_empty()
            && !path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    extensions
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(ext))
                })
        {
            continue;
        }
        // Never alias distinct non-UTF-8 paths through lossy conversion.
        if let (Some(path_text), Some(identity)) = (path.to_str(), identity(path)?) {
            result.insert(path_text.to_owned(), identity);
        }
    }
    Ok(result)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    identity: Identity,
    hash: [u64; 2],
    indexed: bool,
}

#[derive(Debug)]
struct Manifest {
    fingerprint: String,
    files: BTreeMap<String, Entry>,
}

fn config_fingerprint(root: &Path, extensions: &[String]) -> Result<String> {
    let root = root.to_str().context("source root must be UTF-8")?;
    let mut bytes = Vec::new();
    put_string(&mut bytes, root);
    for ext in extensions {
        put_string(&mut bytes, ext);
    }
    Ok(hex(&bytes))
}

// Two domain-separated deterministic digests; cache keys, not security hashes.
// A future standard-library hash change invalidates the manifest checksum and rebuilds safely.
fn digest(bytes: &[u8]) -> [u64; 2] {
    let mut a = std::collections::hash_map::DefaultHasher::new();
    let mut b = std::collections::hash_map::DefaultHasher::new();
    a.write_u64(0x73676578742d7632);
    b.write_u64(0x9e3779b97f4a7c15);
    a.write(bytes);
    b.write(bytes);
    [a.finish(), b.finish()]
}

fn load_manifest(index: &Index) -> Result<Option<Manifest>> {
    Ok(index
        .load_metas()?
        .payload
        .as_deref()
        .and_then(Manifest::decode))
}

fn unchanged(manifest: Option<&Manifest>, snapshot: &Snapshot, fingerprint: &str) -> bool {
    manifest.is_some_and(|m| {
        m.fingerprint == fingerprint
            && m.files.len() == snapshot.len()
            && snapshot.iter().all(|(path, id)| {
                m.files
                    .get(path)
                    .is_some_and(|e| e.identity.reusable_with(id))
            })
    })
}

fn refresh(
    index: &Index,
    root: &Path,
    index_dir: &Path,
    extensions: &[String],
    fingerprint: &str,
) -> Result<IndexStats> {
    let mut stats = IndexStats::default();
    let manifest = load_manifest(index)?;
    let snapshot = scan(root, index_dir, extensions)?;
    if unchanged(manifest.as_ref(), &snapshot, fingerprint) {
        return Ok(stats);
    }

    // Serialize our refreshes across processes; unlike the warm path this may block. Tantivy's
    // writer lock still protects against other Tantivy users. Reload AFTER acquiring it: a
    // pre-lock scan/manifest cannot be used as the base for a commit by a competing opener.
    let directory = index.directory().clone();
    let _refresh_guard = directory.acquire_lock(&Lock {
        filepath: PathBuf::from(".swe-grep-refresh.lock"),
        is_blocking: true,
    })?;
    let mut writer = index
        .writer(50_000_000)
        .context("failed to acquire index writer")?;
    stats.writer_acquisitions += 1;
    let manifest = load_manifest(index)?;
    let snapshot = scan(root, index_dir, extensions)?;
    if unchanged(manifest.as_ref(), &snapshot, fingerprint) {
        return Ok(stats);
    }
    let recover = manifest.is_none();
    let previous = manifest.map(|m| m.files).unwrap_or_default();
    let schema = index.schema();
    let path_field = schema.get_field("path").context("path field missing")?;
    let body_field = schema.get_field("body").context("body field missing")?;
    let exact_field = schema
        .get_field("exact_path")
        .context("exact_path field missing")?;
    if recover {
        writer.delete_all_documents()?;
    }
    for (path, entry) in &previous {
        if !snapshot.contains_key(path) && entry.indexed {
            writer.delete_term(Term::from_field_text(exact_field, path));
            stats.deleted_documents += 1;
        }
    }
    let mut files = BTreeMap::new();
    for (path, scanned_identity) in snapshot {
        let old = previous.get(&path);
        if let Some(old) = old.filter(|e| e.identity.reusable_with(&scanned_identity)) {
            files.insert(path, old.clone());
            continue;
        }
        let path_ref = Path::new(&path);
        let before =
            identity(path_ref)?.context("source disappeared while indexing; reopen to retry")?;
        stats.source_reads += 1;
        let content = fs::read(path_ref);
        let after =
            identity(path_ref)?.context("source disappeared while indexing; reopen to retry")?;
        ensure!(
            before == after,
            "source changed while indexing {}; reopen to retry",
            path
        );
        let bytes = content.ok();
        let hash = bytes.as_deref().map(digest).unwrap_or([0, 0]);
        stats.hashed_files += usize::from(bytes.is_some());
        let text = bytes
            .as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok());
        let indexed = text.is_some();
        if !old.is_some_and(|e| e.hash == hash && e.indexed == indexed) {
            if old.is_some_and(|e| e.indexed) {
                writer.delete_term(Term::from_field_text(exact_field, &path));
                stats.deleted_documents += 1;
            }
            if let Some(text) = text {
                let mut doc = tantivy::Document::new();
                doc.add_text(path_field, &path);
                doc.add_text(exact_field, &path);
                doc.add_text(body_field, text);
                writer.add_document(doc)?;
                stats.indexed_documents += 1;
            }
        }
        files.insert(
            path,
            Entry {
                identity: after,
                hash,
                indexed,
            },
        );
    }
    let manifest = Manifest {
        fingerprint: fingerprint.to_owned(),
        files,
    };
    let mut commit = writer.prepare_commit()?;
    commit.set_payload(&manifest.encode());
    commit
        .commit()
        .context("failed to commit index and manifest")?;
    stats.commits += 1;
    Ok(stats)
}

// A small length-prefixed format avoids extra serialization dependencies and handles arbitrary
// UTF-8 filenames (including newlines). Header/version and checksum reject partial/old payloads.
const MANIFEST_HEADER: &str = "swe-grep-manifest-v2:";

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend(value.to_le_bytes());
}
fn put_string(bytes: &mut Vec<u8>, value: &str) {
    put_u64(bytes, value.len() as u64);
    bytes.extend(value.as_bytes());
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 15) as usize] as char);
    }
    output
}
fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |b| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                _ => None,
            };
            Some(digit(pair[0])? * 16 + digit(pair[1])?)
        })
        .collect()
}
struct Decoder<'a>(&'a [u8]);
impl<'a> Decoder<'a> {
    fn number(&mut self) -> Option<u64> {
        let (value, rest) = self.0.split_at_checked(8)?;
        self.0 = rest;
        Some(u64::from_le_bytes(value.try_into().ok()?))
    }
    fn string(&mut self) -> Option<String> {
        let len = usize::try_from(self.number()?).ok()?;
        let (value, rest) = self.0.split_at_checked(len)?;
        self.0 = rest;
        String::from_utf8(value.to_vec()).ok()
    }
}
impl Manifest {
    fn encode(&self) -> String {
        let mut bytes = Vec::new();
        put_string(&mut bytes, &self.fingerprint);
        put_u64(&mut bytes, self.files.len() as u64);
        for (path, entry) in &self.files {
            put_string(&mut bytes, path);
            for value in entry.identity.0 {
                put_u64(&mut bytes, value);
            }
            for value in entry.hash {
                put_u64(&mut bytes, value);
            }
            put_u64(&mut bytes, u64::from(entry.indexed));
        }
        for value in digest(&bytes) {
            put_u64(&mut bytes, value);
        }
        format!("{MANIFEST_HEADER}{}", hex(&bytes))
    }
    fn decode(payload: &str) -> Option<Self> {
        let bytes = unhex(payload.strip_prefix(MANIFEST_HEADER)?)?;
        let data_len = bytes.len().checked_sub(16)?;
        let (data, checksum) = bytes.split_at(data_len);
        let mut check = Decoder(checksum);
        if digest(data) != [check.number()?, check.number()?] {
            return None;
        }
        let mut decoder = Decoder(data);
        let fingerprint = decoder.string()?;
        let count = decoder.number()?;
        // Each entry needs at least a length plus ten u64s; do not trust persisted counts.
        if count > data.len() as u64 / 88 {
            return None;
        }
        let mut files = BTreeMap::new();
        for _ in 0..count {
            let path = decoder.string()?;
            if !Path::new(&path).is_absolute() {
                return None;
            }
            let mut values = [0; 7];
            for value in &mut values {
                *value = decoder.number()?;
            }
            let hash = [decoder.number()?, decoder.number()?];
            let indexed = match decoder.number()? {
                0 => false,
                1 => true,
                _ => return None,
            };
            if files
                .insert(
                    path,
                    Entry {
                        identity: Identity(values),
                        hash,
                        indexed,
                    },
                )
                .is_some()
            {
                return None;
            }
        }
        if !decoder.0.is_empty() {
            return None;
        }
        Some(Self { fingerprint, files })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips_escaped_paths_and_rejects_corruption() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join("line\n分隔\t.rs")
            .to_str()
            .unwrap()
            .to_owned();
        let manifest = Manifest {
            fingerprint: "root/filter configuration".into(),
            files: BTreeMap::from([(
                path,
                Entry {
                    identity: Identity([1, 2, 3, 4, 5, 6, 7]),
                    hash: digest(b"body"),
                    indexed: true,
                },
            )]),
        };
        let encoded = manifest.encode();
        let decoded = Manifest::decode(&encoded).unwrap();
        assert_eq!(decoded.fingerprint, manifest.fingerprint);
        assert_eq!(decoded.files, manifest.files);
        assert!(Manifest::decode("swe-grep-manifest-v1:00").is_none());
        assert!(Manifest::decode(&encoded[..encoded.len() - 2]).is_none());
        let mut corrupted = encoded.clone().into_bytes();
        let last = corrupted.last_mut().unwrap();
        *last = if *last == b'0' { b'1' } else { b'0' };
        assert!(Manifest::decode(std::str::from_utf8(&corrupted).unwrap()).is_none());
        assert!(Manifest::decode(&format!("{MANIFEST_HEADER}{}", "f".repeat(256))).is_none());
    }

    #[test]
    fn empty_manifest_is_valid_and_trailing_data_is_rejected() {
        let manifest = Manifest {
            fingerprint: String::new(),
            files: BTreeMap::new(),
        };
        assert!(
            Manifest::decode(&manifest.encode())
                .unwrap()
                .files
                .is_empty()
        );
        assert!(Manifest::decode(&(manifest.encode() + "00")).is_none());
    }
}

fn normalize_path(root: &Path, path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    match absolute.strip_prefix(root) {
        Ok(relative) => relative.to_path_buf(),
        Err(_) => absolute,
    }
}
