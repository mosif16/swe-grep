use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use rayon::prelude::*;

use crate::language::detect_language_from_path;
use crate::search::passes_extension_filter;

use super::cache::{AnalysisCache, CacheStats};
use super::facts::{FACTS_SCHEMA_VERSION, FactBundle, FileFacts};

#[derive(Debug, Clone)]
pub struct ExtractFactsOptions {
    pub scope_file: Option<PathBuf>,
    pub extension_filter: Option<Vec<String>>,
}

impl ExtractFactsOptions {
    pub fn for_search(scope_file: Option<PathBuf>, extension_filter: Option<Vec<String>>) -> Self {
        Self {
            scope_file,
            extension_filter,
        }
    }
}

pub fn extract_repo_facts(
    root: &Path,
    cache_dir: &Path,
    options: &ExtractFactsOptions,
) -> Result<(CacheStats, Vec<FileFacts>)> {
    let cache = AnalysisCache::new(root, cache_dir);
    cache.ensure_ready()?;

    let mut stats = CacheStats::default();
    let mut out = Vec::new();

    let files = if let Some(scope_file) = &options.scope_file {
        vec![make_absolute(root, scope_file)]
    } else {
        collect_files(root)?
    };

    let results: Vec<Result<Option<(bool, FileFacts)>>> = files
        .par_iter()
        .map(|abs_path| {
            if !passes_extension_filter(abs_path, options.extension_filter.as_deref()) {
                return Ok(None);
            }

            let Some(language) = detect_language_from_path(abs_path) else {
                return Ok(None);
            };
            if !matches!(
                language,
                "rust" | "swift" | "typescript" | "tsx" | "go" | "python"
            ) {
                return Ok(None);
            }

            let rel_path = rel_path_str(root, abs_path);
            let meta = fs::metadata(abs_path)
                .with_context(|| format!("failed to stat {}", abs_path.display()))?;
            let size = meta.len();
            let mtime = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|dur| dur.as_secs())
                .unwrap_or(0);

            if let Some(cached) = cache.load(&rel_path, mtime, size)? {
                return Ok(Some((true, cached)));
            }

            let source = fs::read_to_string(abs_path)
                .with_context(|| format!("failed to read {}", abs_path.display()))?;
            let facts = match extract_file_facts(language, &rel_path, &source) {
                Ok(facts) => facts,
                Err(_) => {
                    // Facts extraction is a best-effort helper; skip files whose parsers/grammars
                    // are unavailable in the current build/runtime environment.
                    return Ok(None);
                }
            };

            let record = FileFacts {
                version: FACTS_SCHEMA_VERSION,
                file: rel_path,
                mtime,
                size,
                language: language.to_string(),
                facts,
            };
            cache.store(&record)?;
            Ok(Some((false, record)))
        })
        .collect();

    for result in results {
        let Some((hit, record)) = result? else {
            continue;
        };
        stats.processed_files += 1;
        if hit {
            stats.cache_hits += 1;
        }
        out.push(record);
    }

    Ok((stats, out))
}

fn extract_file_facts(language: &str, file: &str, source: &str) -> Result<FactBundle> {
    match language {
        "rust" => super::rust::extract(file, source),
        "swift" => super::swift::extract(file, source),
        "typescript" | "tsx" => super::typescript::extract(file, source, language),
        "go" => super::go::extract(file, source),
        "python" => super::python::extract(file, source),
        other => anyhow::bail!("unsupported language for facts extraction: {other}"),
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
