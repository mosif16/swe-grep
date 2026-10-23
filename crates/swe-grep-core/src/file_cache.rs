use lru::LruCache;
use memmap2::Mmap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::str;
use std::sync::{Arc, Mutex};

const DEFAULT_CAPACITY: usize = 128;

#[derive(Debug)]
pub struct MappedFile {
    contents: String,
}

impl MappedFile {
    pub fn new(mmap: Mmap) -> Result<Self, str::Utf8Error> {
        // A read-only file mapping can still be changed by another process.
        // Validate an owned snapshot, never expose unchecked mapped bytes as str.
        let bytes = mmap.to_vec();
        let contents = str::from_utf8(&bytes)?.to_owned();
        Ok(Self { contents })
    }

    pub fn contents(&self) -> &str {
        &self.contents
    }
}

#[derive(Clone, Debug)]
pub struct FileCache {
    cache: Arc<Mutex<LruCache<PathBuf, Arc<MappedFile>>>>,
    capacity: NonZeroUsize,
    revalidate: bool,
}

impl FileCache {
    pub fn new(capacity: usize) -> Self {
        let capacity = NonZeroUsize::new(capacity)
            .unwrap_or_else(|| NonZeroUsize::new(DEFAULT_CAPACITY).unwrap());
        Self {
            cache: Arc::new(Mutex::new(LruCache::new(capacity))),
            capacity,
            revalidate: true,
        }
    }

    /// Reuse immutable snapshots within one search; clear before the next cycle.
    pub(crate) fn for_cycle(capacity: usize) -> Self {
        let mut cache = Self::new(capacity);
        cache.revalidate = false;
        cache
    }

    pub fn read_file(&self, path: &Path) -> Option<Arc<MappedFile>> {
        if !self.revalidate {
            if let Some(entry) = self.cache.lock().ok()?.get(path) {
                return Some(Arc::clone(entry));
            }
        }
        // Metadata alone cannot detect same-size edits with restored timestamps.
        // Reading also avoids SIGBUS when a previously mapped file is truncated.
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(_) => {
                self.cache.lock().ok()?.pop(path);
                return None;
            }
        };
        // Filesystem I/O is outside the cache mutex. A search cycle reuses its
        // immutable text instead of reading the same file once per matching line.
        let mut cache = self.cache.lock().ok()?;
        if let Some(entry) = cache.get(path) {
            if entry.contents() == contents {
                return Some(Arc::clone(entry));
            }
        }
        let snapshot = Arc::new(MappedFile { contents });
        cache.put(path.to_path_buf(), Arc::clone(&snapshot));
        Some(snapshot)
    }

    pub fn clear(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            *cache = LruCache::new(self.capacity);
        }
    }

    pub fn capacity(&self) -> NonZeroUsize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_survive_edits_truncation_and_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("text");
        fs::write(&path, "hello").unwrap();
        let cache = FileCache::new(2);
        let old = cache.read_file(&path).unwrap();
        assert!(Arc::ptr_eq(&old, &cache.read_file(&path).unwrap()));
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, "world").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(cache.read_file(&path).unwrap().contents(), "world");
        assert_eq!(old.contents(), "hello");
        fs::write(&path, "").unwrap();
        assert_eq!(cache.read_file(&path).unwrap().contents(), "");
        fs::write(&path, [0xff]).unwrap();
        assert!(cache.read_file(&path).is_none());
        fs::remove_file(&path).unwrap();
        assert!(cache.read_file(&path).is_none());
        assert_eq!(old.contents(), "hello");
    }

    #[test]
    fn cycle_snapshots_are_reused_until_explicit_invalidation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.rs");
        fs::write(&path, "before").unwrap();
        let cache = FileCache::for_cycle(2);
        let snapshot = cache.read_file(&path).unwrap();
        fs::write(&path, "after").unwrap();
        assert!(Arc::ptr_eq(&snapshot, &cache.read_file(&path).unwrap()));
        assert_eq!(snapshot.contents(), "before");
        cache.clear();
        assert_eq!(cache.read_file(&path).unwrap().contents(), "after");
    }

    #[test]
    fn mapped_constructor_owns_its_validated_snapshot() {
        let mut map = memmap2::MmapMut::map_anon(5).unwrap();
        map.copy_from_slice(b"hello");
        let snapshot = MappedFile::new(map.make_read_only().unwrap()).unwrap();
        assert_eq!(snapshot.contents(), "hello");
    }
}
