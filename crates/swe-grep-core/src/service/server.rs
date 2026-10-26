use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use tokio::try_join;

use crate::cli::{SearchArgs, ServeArgs, SymbolKind, ToolArgs};
use crate::search::{self, SearchSummary};

use super::{grpc, http};

/// Configuration applied when launching the SWE-Grep services.
#[derive(Clone)]
pub struct ServeConfig {
    pub root: PathBuf,
    pub http_addr: SocketAddr,
    pub grpc_addr: SocketAddr,
    pub timeout_secs: u64,
    pub max_matches: usize,
    pub concurrency: usize,
    pub use_index: bool,
    pub use_rga: bool,
    pub use_fd: bool,
    pub use_ast_grep: bool,
    pub index_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub log_dir: Option<PathBuf>,
}

impl ServeConfig {
    /// Build a runtime configuration from the CLI arguments.
    pub fn try_from_args(args: ServeArgs) -> Result<Self> {
        let provided_root = args
            .path
            .unwrap_or(std::env::current_dir().context("failed to resolve current directory")?);
        let root = provided_root.canonicalize().with_context(|| {
            format!(
                "failed to canonicalize repository root path: {}",
                provided_root.display()
            )
        })?;

        let mut use_index = args.tools.enable_index;
        if use_index && !cfg!(feature = "indexing") {
            tracing::warn!("indexing support not compiled; ignoring --enable-index");
            use_index = false;
        }

        Ok(Self {
            root: root.clone(),
            http_addr: args.http_addr,
            grpc_addr: args.grpc_addr,
            timeout_secs: args.timeout_secs,
            max_matches: usize::max(1, args.max_matches),
            concurrency: usize::max(1, args.concurrency),
            use_index,
            use_rga: args.tools.enable_rga,
            use_fd: args.tools.use_fd,
            use_ast_grep: args.tools.use_ast_grep,
            index_dir: normalize_relative(&root, args.index_dir),
            cache_dir: normalize_relative(&root, args.cache_dir),
            log_dir: normalize_relative(&root, args.log_dir),
        })
    }
}

/// Top-level service runner that coordinates both HTTP and gRPC servers.
pub struct SweGrepServer {
    config: ServeConfig,
}

impl SweGrepServer {
    pub fn new(config: ServeConfig) -> Self {
        Self { config }
    }

    /// Run the gRPC and HTTP services until a shutdown signal is received.
    pub async fn run(self) -> Result<()> {
        let grpc_addr = self.config.grpc_addr;
        let http_addr = self.config.http_addr;
        let executor = Arc::new(SearchExecutor::new(self.config));

        try_join!(
            grpc::serve(grpc_addr, executor.clone()),
            http::serve(http_addr, executor)
        )?;

        Ok(())
    }
}

/// Internal helper that converts structured requests into CLI-compatible search executions.
#[derive(Clone)]
pub struct SearchExecutor {
    config: Arc<ServeConfig>,
}

impl SearchExecutor {
    pub fn new(config: ServeConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    pub fn root(&self) -> &Path {
        &self.config.root
    }

    /// Execute a search using values supplied by the calling protocol layer.
    pub async fn execute(&self, request: SearchInput) -> Result<SearchSummary> {
        search::execute(self.request_to_args(request)?).await
    }

    // Validate filesystem authority before handing any paths to the search engine.
    // This is a request boundary, not protection against concurrent local filesystem mutation.
    fn request_to_args(&self, request: SearchInput) -> Result<SearchArgs> {
        let SearchInput {
            symbol,
            concept,
            language,
            kind,
            root,
            timeout_secs,
            max_matches,
            concurrency,
            enable_index,
            enable_rga,
            regex,
            wildcard,
            string,
            include_strings,
            code_only,
            ignore_case,
            fuzzy_threshold,
            definitions_only,
            usages_only,
            path_glob,
            exclude_patterns,
            exclude_tests,
            index_dir,
            cache_dir,
            log_dir,
            context_before,
            context_after,
            body,
            concept_expand,
            git_modified,
            git_since,
            git_info,
            tool_flags,
        } = request;

        if symbol.trim().is_empty()
            && concept
                .as_deref()
                .map(|c| c.trim().is_empty())
                .unwrap_or(true)
        {
            bail!("symbol is required");
        }

        let service_root = self.config.root.canonicalize().with_context(|| {
            format!(
                "cannot access configured service root: {}",
                self.config.root.display()
            )
        })?;
        // A file-root service authorizes only that file, not its siblings. Relative
        // file names are resolved from its parent, while omitted roots use the file.
        let base = if service_root.is_file() {
            service_root
                .parent()
                .context("service file root has no parent")?
        } else {
            &service_root
        };
        let requested_root = match root {
            Some(path) if path.is_absolute() => path,
            Some(path) => base.join(path),
            None => service_root.clone(),
        };
        let root_path = requested_root.canonicalize().with_context(|| {
            format!(
                "request root must exist and be accessible: {}",
                requested_root.display()
            )
        })?;
        if !root_path.starts_with(&service_root) {
            bail!(
                "request root is outside configured service root: {}",
                root_path.display()
            );
        }
        if !root_path.is_dir() && !root_path.is_file() {
            bail!("request root must be a directory or regular file");
        }
        // Core treats these characters as a glob and falls back to CWD. Never
        // allow that interpretation to bypass the canonical root checked above.
        if root_path.to_string_lossy().contains(['*', '?', '[']) {
            bail!("service root paths containing glob characters are not supported");
        }

        let timeout_secs = timeout_secs.unwrap_or(self.config.timeout_secs);
        let max_matches = usize::max(1, max_matches.unwrap_or(self.config.max_matches));
        let concurrency = usize::max(1, concurrency.unwrap_or(self.config.concurrency));
        let enable_index = enable_index.unwrap_or(self.config.use_index);
        let enable_rga = enable_rga.unwrap_or(self.config.use_rga);

        // Storage locations are trusted startup configuration only. Reject even
        // in-root overrides so nonexistent destinations and symlinked parents
        // cannot grant a remote caller arbitrary filesystem write authority.
        for (name, value) in [
            ("index_dir", index_dir),
            ("cache_dir", cache_dir),
            ("log_dir", log_dir),
        ] {
            if value.is_some() {
                bail!(
                    "request {name} overrides are not allowed; configure the directory at service startup"
                );
            }
        }
        let index_dir = self.config.index_dir.clone();
        let cache_dir = self.config.cache_dir.clone();
        let log_dir = self.config.log_dir.clone();

        let context_before = context_before.unwrap_or(0);
        let context_after = context_after.unwrap_or(0);
        let body = body.unwrap_or(false);
        let string = string.unwrap_or(false);
        let path_glob = path_glob.unwrap_or_default();
        let exclude_patterns = exclude_patterns.unwrap_or_default();
        let exclude_tests = exclude_tests.unwrap_or(false);
        let concept_expand = concept_expand.unwrap_or(false);
        let git_info = git_info.unwrap_or(false);
        let git_modified = git_modified.unwrap_or(false);
        let git_since = git_since.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        let ignore_case = ignore_case.unwrap_or(false);
        let fuzzy_threshold = fuzzy_threshold.unwrap_or(0.7).clamp(0.0, 1.0);
        let definitions_only = definitions_only.unwrap_or(false);
        let usages_only = usages_only.unwrap_or(false);
        let code_only = code_only.unwrap_or(false);
        let include_strings = include_strings.unwrap_or(false) && !code_only;

        if definitions_only && usages_only {
            bail!("--definitions-only and --usages-only cannot be combined");
        }
        let kind = match kind {
            Some(raw) => {
                Some(SymbolKind::from_str(&raw, true).map_err(|_| anyhow!("invalid kind: {raw}"))?)
            }
            None => None,
        };

        let mut args = SearchArgs {
            symbol,
            exact_symbol: false,
            text: false,
            concept,
            concept_expand,
            path: Some(root_path),
            scope_file: None,
            language,
            mode: None,
            kind,
            timeout_secs,
            max_matches,
            top_hits: 20,
            max_columns: 200,
            concurrency,
            tools: ToolArgs {
                enable_index,
                enable_rga,
                use_fd: self.config.use_fd,
                use_ast_grep: self.config.use_ast_grep,
            },
            attribute: false,
            context: None,
            context_before,
            context_after,
            body,
            body_limit: None,
            no_truncate: false,
            git_modified,
            git_since,
            git_info,
            tracked_only: false,
            regex: regex.unwrap_or(false),
            literal_regex: false,
            wildcard: wildcard.unwrap_or(false),
            string,
            verbose_ast: false,
            include_strings,
            code_only,
            ignore_case,
            fuzzy_threshold,
            definitions_only,
            usages_only,
            path_glob,
            exclude_patterns,
            exclude_tests,
            index_dir,
            cache_dir,
            log_dir,
        };

        tool_flags.apply_to(&mut args);

        Ok(args)
    }
}

/// Mutable request wrapper shared by the gRPC and HTTP entry points.
///
/// Roots must exist and resolve within the configured service root. Relative
/// roots are based on that root (its parent for a file-root service), never CWD.
/// Literal root paths containing `*`, `?`, or `[` are rejected because the
/// search engine interprets them as globs rather than filesystem roots.
/// Storage directory overrides are rejected; only startup configuration and
/// the search engine's default storage locations are used.
#[derive(Default)]
pub struct SearchInput {
    pub symbol: String,
    pub concept: Option<String>,
    pub concept_expand: Option<bool>,
    pub language: Option<String>,
    pub kind: Option<String>,
    pub root: Option<PathBuf>,
    pub timeout_secs: Option<u64>,
    pub max_matches: Option<usize>,
    pub concurrency: Option<usize>,
    pub enable_index: Option<bool>,
    pub enable_rga: Option<bool>,
    pub regex: Option<bool>,
    pub wildcard: Option<bool>,
    pub string: Option<bool>,
    pub include_strings: Option<bool>,
    pub code_only: Option<bool>,
    pub ignore_case: Option<bool>,
    pub fuzzy_threshold: Option<f32>,
    pub definitions_only: Option<bool>,
    pub usages_only: Option<bool>,
    pub path_glob: Option<Vec<String>>,
    pub exclude_patterns: Option<Vec<String>>,
    pub exclude_tests: Option<bool>,
    pub index_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub log_dir: Option<PathBuf>,
    pub context_before: Option<usize>,
    pub context_after: Option<usize>,
    pub body: Option<bool>,
    pub git_modified: Option<bool>,
    pub git_since: Option<String>,
    pub git_info: Option<bool>,
    pub tool_flags: ToolFlagOverrides,
}

fn normalize_relative(base: &Path, value: Option<PathBuf>) -> Option<PathBuf> {
    value.map(|path| {
        if path.is_absolute() {
            path
        } else {
            base.join(path)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn executor(root: &Path) -> SearchExecutor {
        SearchExecutor::new(ServeConfig {
            root: root.canonicalize().unwrap(),
            http_addr: "127.0.0.1:0".parse().unwrap(),
            grpc_addr: "127.0.0.1:0".parse().unwrap(),
            timeout_secs: 10,
            max_matches: 25,
            concurrency: 2,
            use_index: false,
            use_rga: false,
            use_fd: false,
            use_ast_grep: false,
            index_dir: None,
            cache_dir: None,
            log_dir: None,
        })
    }

    fn request(root: Option<PathBuf>) -> SearchInput {
        SearchInput {
            symbol: "needle".into(),
            root,
            ..SearchInput::default()
        }
    }

    fn error(executor: &SearchExecutor, input: SearchInput) -> String {
        executor
            .request_to_args(input)
            .err()
            .expect("request should fail")
            .to_string()
    }

    #[test]
    fn default_request_preserves_root_and_defaults() {
        let tmp = TempDir::new().unwrap();
        let executor = executor(tmp.path());
        let args = executor.request_to_args(request(None)).unwrap();
        assert_eq!(args.path.as_deref(), Some(executor.root()));
        assert_eq!(args.timeout_secs, 10);
        assert_eq!(args.max_matches, 25);
        assert_eq!(args.concurrency, 2);
        assert!(args.index_dir.is_none());
        assert!(args.cache_dir.is_none());
        assert!(args.log_dir.is_none());
    }

    #[test]
    fn nested_roots_resolve_under_service_root_not_cwd() {
        let tmp = TempDir::new().unwrap();
        let nested = tmp.path().join("nested/child");
        fs::create_dir_all(&nested).unwrap();
        let executor = executor(tmp.path());
        for path in [
            PathBuf::from("nested/child"),
            PathBuf::from("nested/child/.."),
            nested,
        ] {
            let expected = executor.root().join(&path).canonicalize().unwrap();
            let args = executor.request_to_args(request(Some(path))).unwrap();
            assert_eq!(args.path, Some(expected));
        }
    }

    #[test]
    fn rejects_absolute_parent_and_prefix_sibling_escapes() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("repo");
        let sibling = tmp.path().join("repo-other");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&sibling).unwrap();
        let executor = executor(&root);
        for path in [sibling, PathBuf::from(".."), PathBuf::from("../repo-other")] {
            assert!(
                error(&executor, request(Some(path))).contains("outside configured service root")
            );
        }
    }

    #[test]
    fn missing_request_roots_fail_clearly_without_creation() {
        let tmp = TempDir::new().unwrap();
        let executor = executor(tmp.path());
        for path in [PathBuf::from("missing"), tmp.path().join("missing/child")] {
            assert!(error(&executor, request(Some(path))).contains("must exist and be accessible"));
        }
        assert!(!tmp.path().join("missing").exists());
    }

    #[test]
    fn file_roots_remain_files_and_cannot_authorize_siblings() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("allowed.rs");
        let sibling = tmp.path().join("other.rs");
        fs::write(&file, "needle").unwrap();
        fs::write(&sibling, "secret").unwrap();
        let directory_executor = executor(tmp.path());
        let args = directory_executor
            .request_to_args(request(Some("allowed.rs".into())))
            .unwrap();
        assert_eq!(args.path, Some(file.canonicalize().unwrap()));
        let file_executor = executor(&file);
        for root in [None, Some(file.clone()), Some("allowed.rs".into())] {
            assert_eq!(
                file_executor.request_to_args(request(root)).unwrap().path,
                Some(file.canonicalize().unwrap())
            );
        }
        for root in [sibling, tmp.path().to_path_buf(), PathBuf::from(".")] {
            assert!(
                error(&file_executor, request(Some(root)))
                    .contains("outside configured service root")
            );
        }
    }

    #[test]
    fn rejects_all_storage_overrides_even_inside_root() {
        let tmp = TempDir::new().unwrap();
        let executor = executor(tmp.path());
        for name in ["index_dir", "cache_dir", "log_dir"] {
            for path in [
                tmp.path().join("new-storage"),
                PathBuf::from("new-storage"),
                PathBuf::from("../escape"),
            ] {
                let mut input = request(None);
                match name {
                    "index_dir" => input.index_dir = Some(path),
                    "cache_dir" => input.cache_dir = Some(path),
                    _ => input.log_dir = Some(path),
                }
                assert!(
                    error(&executor, input)
                        .contains(&format!("request {name} overrides are not allowed"))
                );
            }
        }
        assert!(!tmp.path().join("new-storage").exists());
    }

    #[test]
    fn trusted_startup_storage_configuration_is_preserved() {
        let tmp = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let mut config = (*executor(tmp.path()).config).clone();
        config.index_dir = Some(outside.path().join("index"));
        config.cache_dir = Some(outside.path().join("cache"));
        config.log_dir = Some(outside.path().join("log"));
        let args = SearchExecutor::new(config.clone())
            .request_to_args(request(None))
            .unwrap();
        assert_eq!(args.index_dir, config.index_dir);
        assert_eq!(args.cache_dir, config.cache_dir);
        assert_eq!(args.log_dir, config.log_dir);
    }

    #[cfg(unix)]
    #[test]
    fn resolves_internal_symlinks_and_rejects_symlink_escapes() {
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("nested")).unwrap();
        fs::write(outside.path().join("secret.rs"), "secret").unwrap();
        symlink("nested", tmp.path().join("internal")).unwrap();
        symlink(outside.path(), tmp.path().join("external")).unwrap();
        symlink(
            outside.path().join("secret.rs"),
            tmp.path().join("file-link"),
        )
        .unwrap();
        symlink("missing", tmp.path().join("dangling")).unwrap();
        let executor = executor(tmp.path());
        assert_eq!(
            executor
                .request_to_args(request(Some("internal".into())))
                .unwrap()
                .path,
            Some(tmp.path().join("nested").canonicalize().unwrap())
        );
        for root in ["external", "external/secret.rs", "file-link", "external/.."] {
            assert!(
                error(&executor, request(Some(root.into())))
                    .contains("outside configured service root")
            );
        }
        assert!(
            error(&executor, request(Some("dangling".into())))
                .contains("must exist and be accessible")
        );
    }

    #[cfg(unix)]
    #[test]
    fn literal_glob_directory_cannot_trigger_cwd_fallback() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("[glob]")).unwrap();
        let executor = executor(tmp.path());
        assert!(error(&executor, request(Some("[glob]".into()))).contains("glob characters"));
    }
}

#[derive(Clone, Default)]
pub struct ToolFlagOverrides {
    fd: Option<bool>,
    ast_grep: Option<bool>,
    index: Option<bool>,
    rga: Option<bool>,
    body: Option<bool>,
}

impl ToolFlagOverrides {
    pub fn from_map(flags: HashMap<String, bool>) -> Self {
        let mut overrides = Self::default();
        for (key, value) in flags {
            overrides.apply_flag(&key, value);
        }
        overrides
    }

    pub fn set_fd(&mut self, value: Option<bool>) {
        if let Some(v) = value {
            self.fd = Some(v);
        }
    }

    pub fn set_ast_grep(&mut self, value: Option<bool>) {
        if let Some(v) = value {
            self.ast_grep = Some(v);
        }
    }

    pub fn set_index(&mut self, value: Option<bool>) {
        if let Some(v) = value {
            self.index = Some(v);
        }
    }

    pub fn set_rga(&mut self, value: Option<bool>) {
        if let Some(v) = value {
            self.rga = Some(v);
        }
    }

    pub fn set_body(&mut self, value: Option<bool>) {
        if let Some(v) = value {
            self.body = Some(v);
        }
    }

    pub fn apply_to(&self, args: &mut SearchArgs) {
        if let Some(value) = self.fd {
            args.tools.use_fd = value;
        }
        if let Some(value) = self.ast_grep {
            args.tools.use_ast_grep = value;
        }
        if let Some(value) = self.index {
            args.tools.enable_index = value;
        }
        if let Some(value) = self.rga {
            args.tools.enable_rga = value;
        }
        if let Some(value) = self.body {
            args.body = value;
        }
    }

    fn apply_flag(&mut self, key: &str, value: bool) {
        let normalized = key.to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "fd" | "use-fd" | "disable-fd" => self.fd = Some(value),
            "ast-grep" | "use-ast-grep" => self.ast_grep = Some(value),
            "index" | "use-index" | "enable-index" => self.index = Some(value),
            "rga" | "use-rga" | "enable-rga" => self.rga = Some(value),
            "body" | "fetch-body" | "include-body" => self.body = Some(value),
            _ => {}
        }
    }
}
