//! Bounded collection of lease-aware runtime caches. Build artifacts and indexes
//! have separate lifetimes and are deliberately outside this collector.

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

const FORMAT: &str = "swe-grep-managed-cache-v1";
const OWNER: &str = ".swe-grep-owner.json";
const LEASE: &str = ".swe-grep-lease";
const USED: &str = ".swe-grep-last-used";
const COLLECTOR: &str = ".swe-grep-collector";
const COLLECTED: &str = ".swe-grep-last-gc";
const PAYLOAD_CURSOR: &str = ".swe-grep-payload-cursor";
const AUTO_INTERVAL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone)]
pub struct GcPolicy {
    pub max_age: Duration,
    pub max_bytes: u64,
    pub max_namespaces: usize,
    pub max_files: usize,
}

impl Default for GcPolicy {
    fn default() -> Self {
        Self {
            max_age: Duration::from_secs(30 * 24 * 3600),
            max_bytes: 512 * 1024 * 1024,
            max_namespaces: 128,
            max_files: 4096,
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct GcReport {
    pub base: PathBuf,
    pub applied: bool,
    pub scanned_namespaces: usize,
    pub scanned_files: usize,
    pub total_bytes: u64,
    pub eligible_files: usize,
    pub eligible_bytes: u64,
    pub removed_files: usize,
    pub removed_bytes: u64,
    pub remaining_bytes: u64,
    pub projected_bytes: u64,
    pub skipped_active: usize,
    pub active_bytes_unmeasured: bool,
    pub skipped_unmanaged: usize,
    pub skipped_symlinks: usize,
    pub skipped_unknown: usize,
    pub scan_limited: bool,
    pub collector_busy: bool,
    pub entries: Vec<GcEntry>,
}

#[derive(Debug, Serialize)]
pub struct GcEntry {
    pub path: PathBuf,
    pub bytes: u64,
    pub reason: &'static str,
    pub removed: bool,
}

#[derive(Serialize, Deserialize)]
struct Owner {
    format: String,
    root: PathBuf,
}

/// A shared lease spans the complete cache-using operation. It is intentionally
/// not Clone: closing all independently acquired leases releases the OS locks.
pub struct CacheLease {
    _file: Option<File>,
    auto_base: Option<PathBuf>,
    namespace: Option<PathBuf>,
}

/// The dedicated managed cache base, or None when neither cache environment
/// variable is available. Never fall back to scanning the current directory.
pub fn managed_base() -> Option<PathBuf> {
    let absolute_env = |name| {
        std::env::var(name)
            .ok()
            .map(|value| PathBuf::from(value.trim()))
            .filter(|path| path.is_absolute())
    };
    let value = absolute_env("XDG_CACHE_HOME")
        .or_else(|| absolute_env("HOME").map(|path| path.join(".cache")))?;
    Some(
        canonical_cache_parent(&value)
            .join("swe-grep")
            .join("managed-v1"),
    )
}

/// Resolve the trusted environment prefix (including normal macOS /var and
/// /tmp aliases), then append our controlled levels without following them.
pub(crate) fn canonical_cache_parent(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => canonical_cache_parent(parent).join(name),
        _ => path.to_path_buf(),
    }
}

fn root_key(root: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    root.to_string_lossy().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn is_key(name: &str) -> bool {
    name.len() == 16
        && name
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn regular(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_file() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn directory(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn validate_base(base: &Path) -> Result<()> {
    ensure!(
        base.is_absolute(),
        "GC requires an absolute managed cache base"
    );
    ensure!(
        base.file_name().is_some_and(|name| name == "managed-v1"),
        "GC requires a managed-v1 cache base"
    );
    ensure!(
        directory(base)?,
        "managed cache base is missing, not a directory, or a symlink"
    );
    // Check the dedicated parent as well, including a symlinked swe-grep base.
    ensure!(
        directory(base.parent().context("cache base has no parent")?)?,
        "cache base parent must not be a symlink"
    );
    Ok(())
}

fn open_control(path: &Path, create: bool) -> Result<File> {
    if fs::symlink_metadata(path).is_ok() {
        ensure!(
            regular(path)?,
            "cache control file is not a regular file: {}",
            path.display()
        );
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .open(path)?;
    ensure!(
        regular(path)?,
        "cache control file became a symlink: {}",
        path.display()
    );
    Ok(file)
}

fn owner_at(namespace: &Path) -> Result<Option<Owner>> {
    let path = namespace.join(OWNER);
    if !regular(&path)? || fs::metadata(&path)?.len() > 65536 {
        return Ok(None);
    }
    let Ok(owner) = serde_json::from_slice::<Owner>(&fs::read(path)?) else {
        return Ok(None);
    };
    if owner.format != FORMAT
        || !owner.root.is_absolute()
        || owner.root.starts_with(namespace)
        || namespace.file_name().and_then(|name| name.to_str())
            != Some(root_key(&owner.root).as_str())
    {
        return Ok(None);
    }
    Ok(Some(owner))
}

impl CacheLease {
    pub fn acquire(root: &Path, cache_dir: &Path) -> Result<Self> {
        let expected = crate::search::default_cache_dir_for_root(root);
        if cache_dir != expected {
            // A custom writer cannot bypass leases by pointing into another
            // managed namespace (including through a path alias).
            let custom = canonical_cache_parent(cache_dir);
            for ancestor in custom.ancestors() {
                ensure!(
                    owner_at(ancestor)?.is_none(),
                    "custom cache directory overlaps a managed namespace"
                );
            }
            return Ok(Self {
                _file: None,
                auto_base: None,
                namespace: None,
            });
        }
        let base = expected
            .parent()
            .context("managed namespace has no parent")?;
        // Never follow a preexisting symlink while creating our managed levels.
        if let Some(parent) = base.parent() {
            if fs::symlink_metadata(parent).is_ok() {
                ensure!(directory(parent)?, "cache parent must not be a symlink");
            }
        }
        if fs::symlink_metadata(base).is_ok() {
            ensure!(directory(base)?, "managed cache base must not be a symlink");
        }
        fs::create_dir_all(base)?;
        validate_base(base)?;
        if fs::symlink_metadata(cache_dir).is_ok() {
            ensure!(
                directory(cache_dir)?,
                "managed namespace must not be a symlink"
            );
        } else {
            match fs::create_dir(cache_dir) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    ensure!(
                        directory(cache_dir)?,
                        "managed namespace must not be a symlink"
                    );
                }
                Err(error) => return Err(error.into()),
            }
        }
        let file = open_control(&cache_dir.join(LEASE), true)?;
        // Initialization alone needs exclusivity. Existing namespaces can take
        // another shared lease even when this process already holds one.
        while owner_at(cache_dir)?.is_none() {
            match file.try_lock() {
                Ok(()) => {
                    initialize_namespace(root, cache_dir)?;
                    file.unlock()?;
                    break;
                }
                Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(1)),
                Err(TryLockError::Error(error)) => return Err(error.into()),
            }
        }
        file.lock_shared()?;
        match owner_at(cache_dir)? {
            Some(owner) => ensure!(
                owner.root == root,
                "managed namespace owner does not match source root"
            ),
            None => anyhow::bail!("managed owner registration missing"),
        }
        open_control(&cache_dir.join(USED), true)?.set_modified(SystemTime::now())?;
        Ok(Self {
            _file: Some(file),
            auto_base: managed_base().filter(|candidate| candidate == base),
            namespace: Some(cache_dir.to_path_buf()),
        })
    }
}

impl Drop for CacheLease {
    fn drop(&mut self) {
        // The just-finished namespace must be eligible too: sweeping while its
        // shared lease is held would permanently exempt a single busy project.
        drop(self._file.take());
        let (Some(base), Some(namespace)) = (&self.auto_base, &self.namespace) else {
            return;
        };
        let result = (|| -> Result<()> {
            let probe = open_control(&namespace.join(LEASE), false)?;
            match probe.try_lock() {
                Ok(()) => drop(probe),
                // Do not advance the hourly marker on an inner lease's drop;
                // the final lease should still have a chance to collect.
                Err(TryLockError::WouldBlock) => return Ok(()),
                Err(TryLockError::Error(error)) => return Err(error.into()),
            }
            maybe_collect(base)
        })();
        if let Err(error) = result {
            tracing::debug!(%error, "automatic runtime cache collection skipped");
        }
    }
}

fn initialize_namespace(root: &Path, cache_dir: &Path) -> Result<()> {
    match owner_at(cache_dir)? {
        Some(owner) => ensure!(
            owner.root == root,
            "managed namespace owner does not match source root"
        ),
        None => {
            // Do not adopt arbitrary preexisting payloads, even if their
            // names look like caches. New namespaces contain controls only.
            for entry in fs::read_dir(cache_dir)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                ensure!(
                    name == LEASE || name.starts_with(".swe-owner-"),
                    "unmarked cache namespace contains existing files; refusing adoption"
                );
            }
            let owner = Owner {
                format: FORMAT.into(),
                root: root.to_path_buf(),
            };
            let mut temporary = tempfile::Builder::new()
                .prefix(".swe-owner-")
                .tempfile_in(cache_dir)?;
            serde_json::to_writer(temporary.as_file_mut(), &owner)?;
            temporary.as_file_mut().flush()?;
            if let Err(error) = temporary.persist_noclobber(cache_dir.join(OWNER)) {
                if error.error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(error.error.into());
                }
            }
            ensure!(
                owner_at(cache_dir)?.is_some_and(|owner| owner.root == root),
                "managed owner registration failed"
            );
        }
    }
    Ok(())
}

fn maybe_collect(base: &Path) -> Result<()> {
    if fs::symlink_metadata(base.join(COLLECTED)).is_ok() {
        ensure!(
            regular(&base.join(COLLECTED))?,
            "GC usage marker must be a regular file"
        );
        if fs::metadata(base.join(COLLECTED))?
            .modified()?
            .elapsed()
            .unwrap_or_default()
            < AUTO_INTERVAL
        {
            return Ok(());
        }
    }
    collect_inner(base, &GcPolicy::default(), true, None, true)?;
    Ok(())
}

struct Payload {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

struct Namespace {
    path: PathBuf,
    used: SystemTime,
    _lock: Option<File>,
    files: Vec<Payload>,
    next_payload_cursor: usize,
}

/// Collect only registered runtime payloads. Dry runs acquire locks but do not
/// create, touch, rename, or delete files. Limits make the quota a soft bound.
pub fn collect(base: &Path, policy: &GcPolicy, apply: bool) -> Result<GcReport> {
    collect_inner(base, policy, apply, None, false)
}

/// Purge one default managed namespace. Call before acquiring an operation's
/// shared lease; a live user is reported as skipped_active without waiting.
pub fn purge(root: &Path) -> Result<GcReport> {
    let namespace = crate::search::default_cache_dir_for_root(root);
    let base = namespace
        .parent()
        .context("managed namespace has no parent")?;
    collect_inner(
        base,
        &GcPolicy {
            max_age: Duration::ZERO,
            max_bytes: 0,
            ..GcPolicy::default()
        },
        true,
        Some(&namespace),
        false,
    )
}

fn collect_inner(
    base: &Path,
    policy: &GcPolicy,
    apply: bool,
    only: Option<&Path>,
    throttled: bool,
) -> Result<GcReport> {
    let mut report = GcReport {
        base: base.to_path_buf(),
        applied: apply,
        ..GcReport::default()
    };
    if !base.try_exists()? {
        // A dangling symlink must not masquerade as an absent managed base.
        ensure!(
            fs::symlink_metadata(base).is_err(),
            "managed cache base is a symlink"
        );
        return Ok(report);
    }
    validate_base(base)?;
    let collector_path = base.join(COLLECTOR);
    let mut collector = if apply || collector_path.try_exists()? {
        let file = open_control(&collector_path, apply)?;
        match file.try_lock() {
            Ok(()) => Some(file),
            Err(TryLockError::WouldBlock) => {
                report.collector_busy = true;
                return Ok(report);
            }
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
    } else {
        None
    };
    if throttled
        && regular(&base.join(COLLECTED))?
        && fs::metadata(base.join(COLLECTED))?
            .modified()?
            .elapsed()
            .unwrap_or_default()
            < AUTO_INTERVAL
    {
        return Ok(report);
    }
    let mut namespaces = Vec::new();
    let mut inspected = 0usize;
    let mut next_cursor = 0usize;
    let paths: Vec<PathBuf> = if let Some(target) = only {
        if fs::symlink_metadata(target).is_ok() {
            vec![target.to_path_buf()]
        } else {
            vec![]
        }
    } else {
        // Cursor counts directory entries, including unknown ones, so an early
        // block of unmanaged directories cannot monopolize every bounded sweep.
        let cursor = collector
            .as_ref()
            .map(read_cursor)
            .transpose()?
            .unwrap_or(0);
        let mut entries = fs::read_dir(base)?.skip(cursor);
        let mut paths = Vec::new();
        for _ in 0..policy.max_namespaces.saturating_add(2) {
            let Some(entry) = entries.next() else { break };
            paths.push(entry?.path());
        }
        if entries.next().is_some() {
            next_cursor = cursor.saturating_add(paths.len());
            report.scan_limited = true;
        }
        paths
    };
    let namespace_scan_limited = report.scan_limited;
    report.scan_limited = false;
    for path in paths {
        let name = path.file_name().context("namespace path has no name")?;
        let name = name.to_string_lossy();
        if name == COLLECTOR || name == COLLECTED {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            report.skipped_symlinks += 1;
            continue;
        }
        if !metadata.is_dir() || !is_key(&name) {
            report.skipped_unmanaged += 1;
            continue;
        }
        report.scanned_namespaces += 1;
        if owner_at(&path)?.is_none() || !regular(&path.join(LEASE))? || !regular(&path.join(USED))?
        {
            report.skipped_unmanaged += 1;
            continue;
        }
        let file = open_control(&path.join(LEASE), false)?;
        let lock = match file.try_lock() {
            Ok(()) => Some(file),
            Err(TryLockError::WouldBlock) => {
                report.skipped_active += 1;
                report.active_bytes_unmeasured = true;
                continue;
            }
            Err(TryLockError::Error(error)) => return Err(error.into()),
        };
        let mut namespace = Namespace {
            used: fs::metadata(path.join(USED))?.modified()?,
            path,
            _lock: lock,
            files: Vec::new(),
            next_payload_cursor: 0,
        };
        scan_payloads(
            &mut namespace,
            &mut report,
            &mut inspected,
            policy.max_files,
            only.is_some(),
        )?;
        namespaces.push(namespace);
        if report.scan_limited {
            break;
        }
    }
    namespaces.sort_by_key(|namespace| namespace.used);
    let now = SystemTime::now();
    let mut projected = report.total_bytes;
    for namespace in namespaces {
        let old = now.duration_since(namespace.used).unwrap_or_default() >= policy.max_age;
        for payload in namespace.files {
            if !old && projected <= policy.max_bytes {
                continue;
            }
            report.eligible_files += 1;
            report.eligible_bytes = report.eligible_bytes.saturating_add(payload.bytes);
            projected = projected.saturating_sub(payload.bytes);
            let mut removed = false;
            if apply {
                // Revalidate immediately before unlinking. Never recursively
                // remove a namespace or follow a payload/subdirectory symlink.
                if directory(&namespace.path)?
                    && directory(payload.path.parent().unwrap())?
                    && regular(&payload.path)?
                {
                    let current = fs::symlink_metadata(&payload.path)?;
                    if current.len() == payload.bytes && current.modified()? == payload.modified {
                        fs::remove_file(&payload.path)?;
                        removed = true;
                        report.removed_files += 1;
                        report.removed_bytes = report.removed_bytes.saturating_add(payload.bytes);
                    }
                }
            }
            report.entries.push(GcEntry {
                path: payload.path,
                bytes: payload.bytes,
                reason: if old { "age" } else { "size" },
                removed,
            });
        }
        if apply {
            let mut cursor = open_control(&namespace.path.join(PAYLOAD_CURSOR), true)?;
            cursor.set_len(0)?;
            write!(cursor, "{}", namespace.next_payload_cursor)?;
            cursor.flush()?;
        }
    }
    report.remaining_bytes = report.total_bytes.saturating_sub(report.removed_bytes);
    report.projected_bytes = projected;
    report.scan_limited |= namespace_scan_limited;
    if apply {
        if only.is_none() {
            if let Some(file) = collector.as_mut() {
                file.rewind()?;
                file.set_len(0)?;
                write!(file, "{next_cursor}")?;
                file.flush()?;
            }
        }
        open_control(&base.join(COLLECTED), true)?.set_modified(SystemTime::now())?;
    }
    Ok(report)
}

fn read_cursor(file: &File) -> Result<usize> {
    let mut handle = file;
    handle.seek(SeekFrom::Start(0))?;
    let mut text = String::new();
    handle.take(64).read_to_string(&mut text)?;
    Ok(text.trim().parse().unwrap_or(0))
}

fn scan_payloads(
    namespace: &mut Namespace,
    report: &mut GcReport,
    inspected: &mut usize,
    limit: usize,
    from_start: bool,
) -> Result<()> {
    let cursor_path = namespace.path.join(PAYLOAD_CURSOR);
    let start = if fs::symlink_metadata(&cursor_path).is_ok() {
        ensure!(
            regular(&cursor_path)?,
            "payload scan cursor must be a regular file"
        );
        if from_start {
            0
        } else {
            read_cursor(&File::open(&cursor_path)?)?
        }
    } else {
        0
    };
    let mut position = 0usize;
    let mut finished = true;
    'scan: for entry in fs::read_dir(&namespace.path)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if [OWNER, LEASE, USED, PAYLOAD_CURSOR].contains(&name.as_ref()) {
            continue;
        }
        if (name == "facts" || name == "graphs") && directory(&path)? {
            for child in fs::read_dir(&path)? {
                let child = child?;
                let child_name = child.file_name();
                let child_name = child_name.to_string_lossy();
                let owned = if name == "facts" {
                    child_name.strip_suffix(".json").is_some_and(is_key)
                } else {
                    child_name == "analysis_graphs_v3.bin"
                };
                if !scan_entry(
                    child.path(),
                    owned,
                    namespace,
                    report,
                    inspected,
                    limit,
                    start,
                    &mut position,
                )? {
                    finished = false;
                    break 'scan;
                }
            }
        } else if !scan_entry(
            path,
            name == "state.json",
            namespace,
            report,
            inspected,
            limit,
            start,
            &mut position,
        )? {
            finished = false;
            break;
        }
    }
    namespace.next_payload_cursor = if finished { 0 } else { position };
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn scan_entry(
    path: PathBuf,
    owned: bool,
    namespace: &mut Namespace,
    report: &mut GcReport,
    inspected: &mut usize,
    limit: usize,
    start: usize,
    position: &mut usize,
) -> Result<bool> {
    if *position < start {
        *position += 1;
        return Ok(true);
    }
    if *inspected >= limit {
        report.scan_limited = true;
        return Ok(false);
    }
    *inspected += 1;
    *position += 1;
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        report.skipped_symlinks += 1;
    } else if metadata.is_file() && owned {
        add_payload(path, metadata, namespace, report)?;
    } else {
        report.skipped_unknown += 1;
    }
    Ok(true)
}

fn add_payload(
    path: PathBuf,
    metadata: fs::Metadata,
    namespace: &mut Namespace,
    report: &mut GcReport,
) -> Result<()> {
    report.scanned_files += 1;
    report.total_bytes = report.total_bytes.saturating_add(metadata.len());
    namespace.files.push(Payload {
        path,
        bytes: metadata.len(),
        modified: metadata.modified()?,
    });
    Ok(())
}
