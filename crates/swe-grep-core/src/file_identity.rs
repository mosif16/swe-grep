//! Cache-only metadata identity. Never substitute seconds-only mtime checks.
use std::fs::{self, Metadata};
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

const IDENTITY_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity {
    version: u32,
    pub size: u64,
    modified: Option<(u64, u32)>,
    change: Option<(i64, i64, u64, u64)>,
}

impl FileIdentity {
    pub fn from_metadata(meta: &Metadata) -> Self {
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|time| (time.as_secs(), time.subsec_nanos()));
        #[cfg(unix)]
        let change = {
            use std::os::unix::fs::MetadataExt;
            Some((meta.ctime(), meta.ctime_nsec(), meta.dev(), meta.ino()))
        };
        // std Windows metadata exposes write/creation times but not a reliable
        // change time. Restoring write time defeats those, so hash conservatively.
        #[cfg(not(unix))]
        let change = None;
        Self {
            version: IDENTITY_VERSION,
            size: meta.len(),
            modified,
            change,
        }
    }

    pub fn read(path: &Path) -> std::io::Result<Self> {
        fs::metadata(path).map(|meta| Self::from_metadata(&meta))
    }

    pub fn mtime_seconds(&self) -> u64 {
        self.modified.map(|(seconds, _)| seconds).unwrap_or(0)
    }

    pub fn is_current_version(&self) -> bool {
        self.version == IDENTITY_VERSION
    }

    pub fn reliable(&self) -> bool {
        self.is_current_version() && self.modified.is_some()
            // A zero/subsecond-unavailable ctime is conservatively hashed.
            && self.change.is_some_and(|(_, nanos, _, ino)| nanos > 0 && nanos < 1_000_000_000 && ino != 0)
    }

    pub fn reusable_with(&self, current: &Self) -> bool {
        self.reliable() && current.reliable() && self == current
    }
}

/// Read exactly once. Only associate metadata with the bytes when the path's
/// identity was unchanged across the read (including atomic replacement).
pub(crate) fn read_snapshot(
    path: &Path,
    before: FileIdentity,
) -> std::io::Result<(Vec<u8>, Option<FileIdentity>)> {
    #[cfg(test)]
    {
        let mut counts = READ_COUNTS.lock().unwrap();
        *counts.entry(path.to_path_buf()).or_default() += 1;
    }
    let bytes = fs::read(path)?;
    let stable = FileIdentity::read(path)
        .ok()
        .filter(|after| *after == before && after.size == bytes.len() as u64);
    Ok((bytes, stable))
}

#[cfg(test)]
static READ_COUNTS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, usize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_and_coarse_identities_never_get_metadata_hits() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut identity = FileIdentity::read(file.path()).unwrap();
        identity.change = None;
        assert!(!identity.reusable_with(&identity));
        identity.change = Some((1, 0, 1, 1));
        assert!(!identity.reusable_with(&identity));
        identity.change = Some((1, 1, 1, 1));
        identity.version = IDENTITY_VERSION + 1;
        assert!(!identity.reusable_with(&identity));
    }
}

#[cfg(test)]
pub(crate) fn source_reads(path: &Path) -> usize {
    READ_COUNTS.lock().unwrap().get(path).copied().unwrap_or(0)
}
