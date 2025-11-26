use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::schema::{STORED, Schema, SchemaBuilder, TEXT};
use tantivy::{Index, IndexReader, ReloadPolicy};
use tokio::task;

const INDEX_FILENAME: &str = "meta.json";

#[derive(Clone)]
pub struct TantivyIndex {
    #[allow(dead_code)]
    index: Index,
    reader: IndexReader,
    query_parser: tantivy::query::QueryParser,
    path_field: tantivy::schema::Field,
    #[allow(dead_code)]
    body_field: tantivy::schema::Field,
    root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct IndexConfig {
    pub root: PathBuf,
    pub index_dir: PathBuf,
    pub extensions: Option<Vec<String>>,
}

impl TantivyIndex {
    pub async fn open_or_build(config: IndexConfig) -> Result<Self> {
        let IndexConfig {
            root,
            index_dir,
            extensions,
        } = config;

        let schema = build_schema();
        fs::create_dir_all(&index_dir)
            .with_context(|| format!("failed to create index directory {}", index_dir.display()))?;
        let directory = MmapDirectory::open(&index_dir)
            .with_context(|| format!("failed to open index directory {}", index_dir.display()))?;
        let index = Index::open_or_create(directory, schema.clone())
            .with_context(|| format!("failed to open/create index at {}", index_dir.display()))?;

        let needs_build = !index_dir.join(INDEX_FILENAME).exists();
        if needs_build {
            build_index(index.clone(), &root, extensions.clone()).await?;
        }

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommit)
            .try_into()
            .context("failed to create index reader")?;
        reader.reload()?;

        let path_field = index
            .schema()
            .get_field("path")
            .context("path field missing")?;
        let body_field = index
            .schema()
            .get_field("body")
            .context("body field missing")?;
        let query_parser = tantivy::query::QueryParser::for_index(&index, vec![body_field]);

        Ok(Self {
            index,
            reader,
            query_parser,
            path_field,
            body_field,
            root,
        })
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<PathBuf>> {
        if query.trim().is_empty() {
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
    builder.add_text_field("path", STORED);
    builder.add_text_field("body", TEXT);
    builder.build()
}

async fn build_index(index: Index, root: &Path, extensions: Option<Vec<String>>) -> Result<()> {
    let root = root.to_path_buf();
    task::spawn_blocking(move || {
        let mut writer = index
            .writer(50_000_000)
            .context("failed to create index writer")?;
        let schema = index.schema();
        let path_field = schema.get_field("path").context("path field missing")?;
        let body_field = schema.get_field("body").context("body field missing")?;

        let mut walker = WalkBuilder::new(&root);
        walker
            .hidden(false)
            .follow_links(false)
            .standard_filters(true);

        let exts = extensions.unwrap_or_default();
        let filter_by_ext = !exts.is_empty();

        for result in walker.build() {
            let entry = match result {
                Ok(entry) => entry,
                Err(err) => {
                    tracing::warn!(error = %err, "failed to read entry during indexing");
                    continue;
                }
            };
            if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                continue;
            }
            if filter_by_ext {
                let path_ext = entry.path().extension().and_then(|e| e.to_str());
                if let Some(ext) = path_ext {
                    if !exts
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(ext))
                    {
                        continue;
                    }
                } else {
                    continue;
                }
            }

            let path = entry.path();
            let content = match fs::read_to_string(path) {
                Ok(text) => text,
                Err(_) => continue,
            };

            let mut doc = tantivy::Document::new();
            doc.add_text(path_field, path.display().to_string());
            doc.add_text(body_field, content);
            if let Err(err) = writer.add_document(doc) {
                tracing::warn!(error = %err, "failed to add document to index");
            }
        }

        writer.commit().context("failed to commit index writer")?;
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("index build task cancelled")??;

    Ok(())
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
