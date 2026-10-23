use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use globset::GlobSet;

use super::config::{SearchConfig, passes_extension_filter};

/// Normalized filesystem scope shared by text and semantic backends.
/// Decisions are memoized for one query, never across filesystem updates.
#[derive(Clone)]
pub struct SearchScope {
    inner: Arc<ScopeInner>,
}

struct ScopeInner {
    root: PathBuf,
    file: Option<PathBuf>,
    git: Option<HashSet<String>>,
    include: Option<GlobSet>,
    exclude: Option<GlobSet>,
    extensions: Option<Vec<String>>,
    decisions: Mutex<HashMap<PathBuf, bool>>,
    #[cfg(test)]
    resolutions: std::sync::atomic::AtomicUsize,
}

impl SearchScope {
    pub(crate) fn new(config: &SearchConfig, git: Option<HashSet<String>>) -> Self {
        Self {
            inner: Arc::new(ScopeInner {
                root: config.root.clone(),
                // Config has already canonicalized and validated this file.
                file: config
                    .scope_file
                    .as_ref()
                    .map(|file| config.root.join(file)),
                git,
                include: config.path_globs.clone(),
                exclude: config.exclude_globs.clone(),
                extensions: config.extension_filter.clone(),
                decisions: Mutex::new(HashMap::new()),
                #[cfg(test)]
                resolutions: std::sync::atomic::AtomicUsize::new(0),
            }),
        }
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    /// Check origin-file eligibility independently of matching strategy.
    /// Relative paths are resolved under the search root.
    pub fn allows(&self, path: &Path) -> bool {
        let absolute = self.inner.root.join(path);
        let Ok(mut decisions) = self.inner.decisions.lock() else {
            return false;
        };
        if let Some(&allowed) = decisions.get(&absolute) {
            return allowed;
        }
        let allowed = self.check(&absolute);
        decisions.insert(absolute, allowed);
        allowed
    }

    fn check(&self, absolute: &Path) -> bool {
        #[cfg(test)]
        self.inner
            .resolutions
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Ok(canonical) = absolute.canonicalize() else {
            return false;
        };
        let Ok(relative) = canonical.strip_prefix(&self.inner.root) else {
            return false;
        };
        if self
            .inner
            .file
            .as_ref()
            .is_some_and(|file| *file != canonical)
        {
            return false;
        }
        if !passes_extension_filter(relative, self.inner.extensions.as_deref()) {
            return false;
        }
        let rel = relative.to_string_lossy().replace(char::from(92), "/");
        if self
            .inner
            .git
            .as_ref()
            .is_some_and(|git| !git.contains(&rel))
        {
            return false;
        }
        let abs = canonical.to_string_lossy().replace(char::from(92), "/");
        if self
            .inner
            .exclude
            .as_ref()
            .is_some_and(|set| set.is_match(&rel) || set.is_match(&abs))
        {
            return false;
        }
        self.inner
            .include
            .as_ref()
            .is_none_or(|set| set.is_match(&rel) || set.is_match(&abs))
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut decisions) = self.inner.decisions.lock() {
            decisions.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::SearchArgs;

    fn scope(root: &Path) -> SearchScope {
        let config = SearchConfig::try_from_args(SearchArgs {
            symbol: "target".into(),
            path: Some(root.to_owned()),
            ..SearchArgs::default()
        })
        .unwrap();
        SearchScope::new(&config, None)
    }

    #[test]
    fn repeated_hits_resolve_each_path_once_per_query() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("file.rs"), "fn target() {}").unwrap();
        let scope = scope(temp.path());
        let shared = scope.clone();
        for _ in 0..1_000 {
            assert!(scope.allows(Path::new("file.rs")));
            assert!(shared.allows(Path::new("file.rs")));
            assert!(!scope.allows(Path::new("missing.rs")));
        }
        assert_eq!(
            scope
                .inner
                .resolutions
                .load(std::sync::atomic::Ordering::Relaxed),
            2
        );
        scope.clear();
        assert!(scope.allows(Path::new("file.rs")));
        assert_eq!(
            scope
                .inner
                .resolutions
                .load(std::sync::atomic::Ordering::Relaxed),
            3
        );
    }

    #[test]
    fn scope_keeps_file_git_and_glob_filters_together() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["allowed.rs", "other.rs", "allowed.py"] {
            std::fs::write(temp.path().join(name), "target").unwrap();
        }
        let config = SearchConfig::try_from_args(SearchArgs {
            symbol: "target".into(),
            path: Some(temp.path().to_owned()),
            scope_file: Some("allowed.rs".into()),
            language: Some("rust".into()),
            path_glob: vec!["*.rs".into()],
            ..SearchArgs::default()
        })
        .unwrap();
        let scope = SearchScope::new(
            &config,
            Some(HashSet::from(["allowed.rs".into(), "other.rs".into()])),
        );
        assert!(scope.allows(Path::new("allowed.rs")));
        assert!(!scope.allows(Path::new("other.rs")));
        assert!(!scope.allows(Path::new("allowed.py")));
    }

    #[cfg(unix)]
    #[test]
    fn memoization_does_not_authorize_outside_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.rs"), "secret").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.rs"),
            root.path().join("link.rs"),
        )
        .unwrap();
        let scope = scope(root.path());
        assert!(!scope.allows(Path::new("link.rs")));
        assert!(!scope.allows(&outside.path().join("secret.rs")));
    }
}
