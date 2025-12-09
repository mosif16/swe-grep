use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::search::hits::SearchHit;

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct PersistentStateData {
    pub(crate) symbol_hits: HashMap<String, Vec<String>>,
    pub(crate) directory_scores: HashMap<String, u32>,
}

pub(crate) struct PersistentState {
    root: PathBuf,
    file_path: PathBuf,
    data: PersistentStateData,
    dirty: bool,
}

impl PersistentState {
    pub(crate) fn load(root: &Path, cache_dir: &Path) -> Result<Self> {
        let file_path = cache_dir.join("state.json");
        let data = if file_path.exists() {
            match fs::read_to_string(&file_path) {
                Ok(contents) => match serde_json::from_str(&contents) {
                    Ok(parsed) => parsed,
                    Err(err) => {
                        tracing::warn!(
                            path = %file_path.display(),
                            error = %err,
                            "failed to parse persistent state JSON; resetting to default"
                        );
                        PersistentStateData::default()
                    }
                },
                Err(err) => {
                    tracing::warn!(
                        path = %file_path.display(),
                        error = %err,
                        "failed to read persistent state file"
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

    pub(crate) fn hints_for_symbol(&self, symbol: &str) -> Vec<PathBuf> {
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

    pub(crate) fn top_directories(&self, limit: usize) -> Vec<PathBuf> {
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

    pub(crate) fn known_symbols(&self) -> Vec<String> {
        self.data.symbol_hits.keys().cloned().collect()
    }

    const MAX_SYMBOLS: usize = 500;
    const MAX_DIRECTORIES: usize = 200;

    pub(crate) fn observe(&mut self, symbol: &str, hits: &[SearchHit]) {
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

        self.evict_if_needed();

        self.dirty = true;
    }

    fn evict_if_needed(&mut self) {
        if self.data.symbol_hits.len() > Self::MAX_SYMBOLS {
            let excess = self.data.symbol_hits.len() - Self::MAX_SYMBOLS;
            let keys_to_remove: Vec<String> =
                self.data.symbol_hits.keys().take(excess).cloned().collect();
            for key in keys_to_remove {
                self.data.symbol_hits.remove(&key);
            }
            tracing::debug!(
                evicted = excess,
                remaining = self.data.symbol_hits.len(),
                "evicted symbol cache entries"
            );
        }

        if self.data.directory_scores.len() > Self::MAX_DIRECTORIES {
            let mut dirs: Vec<_> = self
                .data
                .directory_scores
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect();
            dirs.sort_by(|a, b| b.1.cmp(&a.1));
            let to_keep: HashSet<String> = dirs
                .into_iter()
                .take(Self::MAX_DIRECTORIES)
                .map(|(k, _)| k)
                .collect();
            let before = self.data.directory_scores.len();
            self.data
                .directory_scores
                .retain(|k, _| to_keep.contains(k));
            tracing::debug!(
                evicted = before - self.data.directory_scores.len(),
                remaining = self.data.directory_scores.len(),
                "evicted directory cache entries"
            );
        }
    }

    pub(crate) fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create cache directory {}", parent.display())
            })?;
        }
        let tmp_path = self.file_path.with_extension("json.tmp");
        let file = fs::File::create(&tmp_path)
            .with_context(|| format!("failed to create {}", tmp_path.display()))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &self.data)
            .context("failed to serialize persistent state")?;
        writer
            .flush()
            .with_context(|| format!("failed to flush {}", tmp_path.display()))?;
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
