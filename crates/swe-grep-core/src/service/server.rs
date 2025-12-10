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

    fn normalize_with_root(&self, path: PathBuf) -> PathBuf {
        if path.is_absolute() {
            path
        } else {
            self.config.root.join(path)
        }
    }

    /// Execute a search using values supplied by the calling protocol layer.
    pub async fn execute(&self, request: SearchInput) -> Result<SearchSummary> {
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

        let root_path = root
            .map(|p| self.normalize_with_root(p))
            .unwrap_or_else(|| self.config.root.clone());

        let timeout_secs = timeout_secs.unwrap_or(self.config.timeout_secs);
        let max_matches = usize::max(1, max_matches.unwrap_or(self.config.max_matches));
        let concurrency = usize::max(1, concurrency.unwrap_or(self.config.concurrency));
        let enable_index = enable_index.unwrap_or(self.config.use_index);
        let enable_rga = enable_rga.unwrap_or(self.config.use_rga);

        let index_dir = index_dir
            .map(|p| self.normalize_with_root(p))
            .or_else(|| self.config.index_dir.clone());

        let cache_dir = cache_dir
            .map(|p| self.normalize_with_root(p))
            .or_else(|| self.config.cache_dir.clone());

        let log_dir = log_dir
            .map(|p| self.normalize_with_root(p))
            .or_else(|| self.config.log_dir.clone());

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
            concept,
            concept_expand,
            path: Some(root_path),
            language,
            kind,
            timeout_secs,
            max_matches,
            concurrency,
            tools: ToolArgs {
                enable_index,
                enable_rga,
                use_fd: self.config.use_fd,
                use_ast_grep: self.config.use_ast_grep,
            },
            context_before,
            context_after,
            body,
            git_modified,
            git_since,
            git_info,
            regex: regex.unwrap_or(false),
            wildcard: wildcard.unwrap_or(false),
            string,
            include_strings,
            code_only,
            ignore_case,
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

        search::execute(args).await
    }
}

/// Mutable request wrapper shared by the gRPC and HTTP entry points.
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
