use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::cli::SearchArgs;
pub(crate) use crate::language::{
    expand_language_hint, extensions_for_languages, languages_include,
};

pub(crate) const DEFAULT_MAX_COLUMNS: usize = 200;
pub(crate) const DEFAULT_MAX_BODY_BYTES: usize = 512 * 1024;
pub(crate) const DEFAULT_INLINE_CONTEXT: usize = 2;
pub(crate) const TRUNCATED_INLINE_CONTEXT: usize = 4;
pub(crate) const MAX_SYMBOL_LENGTH: usize = 256;

pub(crate) struct SearchConfig {
    pub(crate) root: PathBuf,
    pub(crate) symbol: String,
    #[allow(dead_code)]
    pub(crate) language: Option<String>,
    pub(crate) language_tokens: Vec<String>,
    pub(crate) timeout: Duration,
    pub(crate) max_matches: usize,
    #[allow(dead_code)]
    pub(crate) concurrency: usize,
    pub(crate) use_index: bool,
    pub(crate) index_dir: PathBuf,
    pub(crate) use_rga: bool,
    pub(crate) use_fd: bool,
    pub(crate) use_ast: bool,
    pub(crate) cache_dir: PathBuf,
    pub(crate) log_dir: Option<PathBuf>,
    pub(crate) context_before: usize,
    pub(crate) context_after: usize,
    pub(crate) max_columns: usize,
    pub(crate) body: bool,
    pub(crate) max_body_bytes: usize,
}

impl SearchConfig {
    pub(crate) fn try_from_args(args: SearchArgs) -> Result<Self> {
        let symbol = args.symbol.trim();
        if symbol.is_empty() {
            anyhow::bail!("symbol is required");
        }
        if symbol.len() > MAX_SYMBOL_LENGTH {
            anyhow::bail!(
                "symbol exceeds maximum length of {} characters (got {})",
                MAX_SYMBOL_LENGTH,
                symbol.len()
            );
        }

        let root = args
            .path
            .unwrap_or(std::env::current_dir().context("failed to resolve current directory")?);
        let root = root.canonicalize().with_context(|| {
            format!(
                "failed to canonicalize repository root path: {}",
                root.display()
            )
        })?;

        let concurrency = usize::max(1, args.concurrency);
        let timeout = Duration::from_secs(args.timeout_secs);
        let index_dir = args
            .index_dir
            .clone()
            .unwrap_or_else(|| root.join(".swe-grep-index"));
        let cache_dir = args
            .cache_dir
            .clone()
            .unwrap_or_else(|| root.join(".swe-grep-cache"));
        let log_dir = args.log_dir.map(|dir| {
            if dir.is_absolute() {
                dir
            } else {
                root.join(dir)
            }
        });

        let mut use_index = args.tools.enable_index;
        if use_index && !cfg!(feature = "indexing") {
            tracing::warn!("indexing support not compiled; ignoring --enable-index");
            use_index = false;
        }

        let language = args
            .language
            .map(|lang| lang.trim().to_string())
            .filter(|s| !s.is_empty());
        let language_tokens = expand_language_hint(language.as_deref());

        Ok(Self {
            root,
            symbol: symbol.to_string(),
            language,
            language_tokens,
            timeout,
            max_matches: usize::max(1, args.max_matches),
            concurrency,
            use_index,
            index_dir,
            use_rga: args.tools.enable_rga,
            use_fd: args.tools.use_fd,
            use_ast: args.tools.use_ast_grep,
            cache_dir,
            log_dir,
            context_before: args.context_before,
            context_after: args.context_after,
            max_columns: DEFAULT_MAX_COLUMNS,
            body: args.body,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        })
    }
}

pub(crate) fn passes_extension_filter(path: &Path, extensions: Option<&[&str]>) -> bool {
    match extensions {
        Some(exts) => path
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| {
                exts.iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(ext))
            })
            .unwrap_or(false),
        None => true,
    }
}

pub(crate) fn normalize_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let canonical = absolute.canonicalize().unwrap_or_else(|_| absolute.clone());
    Ok(canonical
        .strip_prefix(root)
        .map(|p| p.to_path_buf())
        .unwrap_or(canonical))
}
