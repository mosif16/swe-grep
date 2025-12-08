use memmap2::Mmap;
use lru::LruCache;
use std::fs::File;
use std::io;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::str;
use std::sync::{Arc, Mutex};

const DEFAULT_CAPACITY: usize = 128;

#[derive(Debug)]
pub struct MappedFile {
    mmap: Mmap,
}

impl MappedFile {
    pub fn new(mmap: Mmap) -> Result<Self, str::Utf8Error> {
        str::from_utf8(&mmap)?;
        Ok(Self { mmap })
    }

    pub fn contents(&self) -> &str {
        // SAFETY: UTF-8 was validated in `new`.
        unsafe { str::from_utf8_unchecked(&self.mmap) }
    }
}

#[derive(Clone, Debug)]
pub struct FileCache {
    cache: Arc<Mutex<LruCache<PathBuf, Arc<MappedFile>>>>,
    capacity: NonZeroUsize,
}

impl FileCache {
    pub fn new(capacity: usize) -> Self {
        let capacity = NonZeroUsize::new(capacity)
            .unwrap_or_else(|| NonZeroUsize::new(DEFAULT_CAPACITY).unwrap());
        Self {
            cache: Arc::new(Mutex::new(LruCache::new(capacity))),
            capacity,
        }
    }

    pub fn read_file(&self, path: &Path) -> Option<Arc<MappedFile>> {
        let path_buf = path.to_path_buf();

        if let Ok(mut cache) = self.cache.lock() {
            if let Some(entry) = cache.get(&path_buf).cloned() {
                return Some(entry);
            }
        } else {
            return None;
        }

        let mapped_file = Self::map_file(&path_buf).ok().map(Arc::new)?;

        if let Ok(mut cache) = self.cache.lock() {
            if let Some(entry) = cache.get(&path_buf).cloned() {
                return Some(entry);
            }
            cache.put(path_buf, Arc::clone(&mapped_file));
        }

        Some(mapped_file)
    }

    pub fn clear(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            *cache = LruCache::new(self.capacity);
        }
    }

    pub fn capacity(&self) -> NonZeroUsize {
        self.capacity
    }

    fn map_file(path: &Path) -> io::Result<MappedFile> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        MappedFile::new(mmap).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}
