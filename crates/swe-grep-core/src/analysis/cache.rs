use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::facts::{FileFacts, FACTS_SCHEMA_VERSION};

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
        fs::create_dir_all(&self.facts_dir)
            .with_context(|| format!("failed to create facts cache dir {}", self.facts_dir.display()))
    }

    pub fn load(
        &self,
        rel_path: &str,
        mtime: u64,
        size: u64,
    ) -> Result<Option<FileFacts>> {
        let cache_path = self.cache_path_for(rel_path);
        let bytes = match fs::read(&cache_path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).with_context(|| format!("failed to read {}", cache_path.display())),
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
        fs::write(&tmp, &bytes)
            .with_context(|| format!("failed to write {}", tmp.display()))?;
        fs::rename(&tmp, &cache_path)
            .with_context(|| format!("failed to rename {} -> {}", tmp.display(), cache_path.display()))?;
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
