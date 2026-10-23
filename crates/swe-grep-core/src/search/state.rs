use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::Result;
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
    enabled: bool,
}

impl PersistentState {
    pub(crate) fn load(root: &Path, cache_dir: &Path) -> Result<Self> {
        let file_path = cache_dir.join("state.json");
        let mut enabled = true;

        let data = if let Err(err) = fs::create_dir_all(cache_dir) {
            tracing::debug!(
                path = %cache_dir.display(),
                error = %err,
                "cache directory unavailable; disabling persistent state"
            );
            enabled = false;
            PersistentStateData::default()
        } else if file_path.exists() {
            match fs::read_to_string(&file_path) {
                Ok(contents) => match serde_json::from_str(&contents) {
                    Ok(parsed) => parsed,
                    Err(err) => {
                        tracing::debug!(
                            path = %file_path.display(),
                            error = %err,
                            "failed to parse persistent state JSON; resetting to default"
                        );
                        PersistentStateData::default()
                    }
                },
                Err(err) => {
                    tracing::debug!(
                        path = %file_path.display(),
                        error = %err,
                        "failed to read persistent state file; disabling persistence"
                    );
                    enabled = false;
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
            enabled,
        })
    }

    pub(crate) fn hints_for_symbol(&self, symbol: &str) -> Vec<PathBuf> {
        self.data
            .symbol_hits
            .get(symbol)
            .into_iter()
            .flat_map(|paths| paths.iter())
            .filter_map(|text| {
                let relative = self.safe_cached_path(text)?;
                self.root.join(&relative).is_file().then_some(relative)
            })
            .collect()
    }

    pub(crate) fn top_directories(&self, limit: usize) -> Vec<PathBuf> {
        let mut dirs: Vec<_> = self.data.directory_scores.iter().collect();
        dirs.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        dirs.into_iter()
            .filter_map(|(dir, _)| {
                let path = self.safe_cached_path(dir)?;
                self.root.join(&path).is_dir().then_some(path)
            })
            .take(limit)
            .collect()
    }

    // Cache files are hints, not authority to search outside the selected root.
    fn safe_cached_path(&self, text: &str) -> Option<PathBuf> {
        let path = Path::new(text);
        if path.is_absolute()
            || path.components().any(|component| match component {
                Component::Normal(name) => name.to_string_lossy().starts_with('.'),
                Component::CurDir => false,
                _ => true,
            })
        {
            return None;
        }
        let canonical = self.root.join(path).canonicalize().ok()?;
        canonical
            .strip_prefix(&self.root)
            .ok()
            .map(Path::to_path_buf)
    }

    pub(crate) fn known_symbols(&self) -> Vec<String> {
        let mut symbols: Vec<_> = self.data.symbol_hits.keys().cloned().collect();
        symbols.sort();
        symbols
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
            dirs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
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
        if !self.enabled {
            self.dirty = false;
            return Ok(());
        }
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = self.file_path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                tracing::debug!(
                    path = %parent.display(),
                    error = %err,
                    "cache directory unavailable; disabling persistent state"
                );
                self.enabled = false;
                self.dirty = false;
                return Ok(());
            }
        }
        // A unique same-directory temporary file prevents concurrent searches
        // from truncating or renaming each other's in-progress state.
        let result = (|| -> Result<()> {
            let parent = self.file_path.parent().expect("state file has a parent");
            let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
            {
                let mut writer = BufWriter::new(temporary.as_file_mut());
                serde_json::to_writer_pretty(&mut writer, &self.data)?;
                writer.flush()?;
            }
            temporary.persist(&self.file_path)?;
            Ok(())
        })();
        if let Err(err) = result {
            tracing::debug!(error = %err, "failed to save persistent state; disabling persistence");
            self.enabled = false;
        }
        self.dirty = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use tempfile::tempdir;

    #[test]
    fn cached_paths_cannot_escape_root() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("valid.rs"), "fn target() {}").unwrap();
        fs::write(temp.path().join("outside.rs"), "secret").unwrap();
        fs::create_dir(root.join(".hidden")).unwrap();
        fs::write(root.join(".hidden/hidden.rs"), "secret").unwrap();
        let root = root.canonicalize().unwrap();
        let mut state = PersistentState::load(&root, &temp.path().join("cache")).unwrap();
        state.data.symbol_hits.insert(
            "target".into(),
            vec![
                "valid.rs".into(),
                "../outside.rs".into(),
                temp.path()
                    .join("outside.rs")
                    .to_string_lossy()
                    .into_owned(),
                ".hidden/hidden.rs".into(),
            ],
        );
        assert_eq!(
            state.hints_for_symbol("target"),
            vec![PathBuf::from("valid.rs")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn cached_symlink_cannot_escape_root() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();
        fs::write(temp.path().join("outside.rs"), "secret").unwrap();
        std::os::unix::fs::symlink(temp.path().join("outside.rs"), root.join("link.rs")).unwrap();
        let root = root.canonicalize().unwrap();
        let mut state = PersistentState::load(&root, &temp.path().join("cache")).unwrap();
        state
            .data
            .symbol_hits
            .insert("target".into(), vec!["link.rs".into()]);
        assert!(state.hints_for_symbol("target").is_empty());
    }

    #[test]
    fn concurrent_saves_do_not_share_temporary_files() {
        let temp = tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let cache = root.join("cache");
        let barrier = Arc::new(Barrier::new(8));
        std::thread::scope(|scope| {
            for index in 0..8 {
                let barrier = Arc::clone(&barrier);
                let root = &root;
                let cache = &cache;
                scope.spawn(move || {
                    let mut state = PersistentState::load(root, cache).unwrap();
                    state
                        .data
                        .symbol_hits
                        .insert(format!("symbol{index}"), vec!["file.rs".into()]);
                    for _ in 0..10 {
                        barrier.wait();
                        state.dirty = true;
                        state.save().unwrap();
                        assert!(
                            state.enabled,
                            "a concurrent writer must not disable persistence"
                        );
                    }
                });
            }
        });
        let saved: PersistentStateData =
            serde_json::from_slice(&fs::read(cache.join("state.json")).unwrap()).unwrap();
        assert_eq!(saved.symbol_hits.len(), 1); // Last writer wins; snapshots are never interleaved.
        assert_eq!(fs::read_dir(cache).unwrap().count(), 1);
    }
}
