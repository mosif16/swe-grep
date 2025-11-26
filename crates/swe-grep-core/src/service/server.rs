use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use tokio::try_join;

use crate::cli::{SearchArgs, ServeArgs};
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

        let mut use_index = args.enable_index;
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
            use_rga: args.enable_rga,
            use_fd: args.use_fd,
            use_ast_grep: args.use_ast_grep,
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
            language,
            root,
            timeout_secs,
            max_matches,
            concurrency,
            enable_index,
            enable_rga,
            index_dir,
            cache_dir,
            log_dir,
            context_before,
            context_after,
            body,
            tool_flags,
        } = request;

        if symbol.trim().is_empty() {
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

        let mut args = SearchArgs {
            symbol,
            path: Some(root_path),
            language,
            timeout_secs,
            max_matches,
            concurrency,
            context_before,
            context_after,
            body,
            enable_index,
            index_dir,
            enable_rga,
            cache_dir,
            log_dir,
            use_fd: self.config.use_fd,
            use_ast_grep: self.config.use_ast_grep,
        };

        if !tool_flags.is_empty() {
            args = apply_tool_flags(args, tool_flags);
        }

        search::execute(args).await
    }
}

/// Mutable request wrapper shared by the gRPC and HTTP entry points.
#[derive(Default)]
pub struct SearchInput {
    pub symbol: String,
    pub language: Option<String>,
    pub root: Option<PathBuf>,
    pub timeout_secs: Option<u64>,
    pub max_matches: Option<usize>,
    pub concurrency: Option<usize>,
    pub enable_index: Option<bool>,
    pub enable_rga: Option<bool>,
    pub index_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub log_dir: Option<PathBuf>,
    pub context_before: Option<usize>,
    pub context_after: Option<usize>,
    pub body: Option<bool>,
    pub tool_flags: HashMap<String, bool>,
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

fn apply_tool_flags(mut args: SearchArgs, flags: HashMap<String, bool>) -> SearchArgs {
    for (key, value) in flags {
        let normalized = key.to_ascii_lowercase();
        match normalized.as_str() {
            "fd" | "use_fd" | "disable_fd" => {
                args.use_fd = value;
            }
            "ast-grep" | "ast_grep" | "use_ast_grep" => {
                args.use_ast_grep = value;
            }
            "index" | "use_index" | "enable_index" => {
                args.enable_index = value;
            }
            "rga" | "use_rga" | "enable_rga" => {
                args.enable_rga = value;
            }
            "body" | "fetch_body" | "include_body" => {
                args.body = value;
            }
            _ => {}
        }
    }
    args
}
