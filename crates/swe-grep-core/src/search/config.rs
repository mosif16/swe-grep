use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};

use super::query::PatternMode;
use crate::cli::{SearchArgs, SymbolKind};
pub(crate) use crate::language::{
    code_extensions_for_languages, expand_language_hint, extensions_for_languages,
    languages_include,
};

pub(crate) const DEFAULT_MAX_COLUMNS: usize = 200;
pub(crate) const DEFAULT_MAX_BODY_BYTES: usize = 512 * 1024;
pub(crate) const DEFAULT_INLINE_CONTEXT: usize = 2;
pub(crate) const TRUNCATED_INLINE_CONTEXT: usize = 4;
pub(crate) const MAX_SYMBOL_LENGTH: usize = 256;
const TEST_EXCLUDE_GLOBS: &[&str] = &[
    "*_test.rs",
    "*_test.go",
    "*.test.ts",
    "*.test.tsx",
    "*.spec.ts",
    "*Tests.swift",
    "*Spec.swift",
    "tests/**",
    "__tests__/**",
    "test/**",
    "spec/**",
];

#[derive(Clone)]
pub(crate) struct GitFilter {
    pub(crate) modified: bool,
    pub(crate) since: Option<String>,
}

pub(crate) struct SearchConfig {
    pub(crate) root: PathBuf,
    pub(crate) symbol: String,
    pub(crate) concept: Option<String>,
    pub(crate) concept_expand: bool,
    pub(crate) kind: Option<SymbolKind>,
    #[allow(dead_code)]
    pub(crate) language: Option<String>,
    pub(crate) language_tokens: Vec<String>,
    pub(crate) pattern_mode: PatternMode,
    pub(crate) ignore_case: bool,
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
    pub(crate) include_strings: bool,
    #[allow(dead_code)]
    pub(crate) code_only: bool,
    pub(crate) definitions_only: bool,
    pub(crate) usages_only: bool,
    pub(crate) git_info: bool,
    pub(crate) git_filter: Option<GitFilter>,
    pub(crate) path_globs: Option<GlobSet>,
    pub(crate) exclude_globs: Option<GlobSet>,
    pub(crate) extension_filter: Option<Vec<String>>,
}

impl SearchConfig {
    pub(crate) fn try_from_args(args: SearchArgs) -> Result<Self> {
        if args.regex && args.wildcard {
            anyhow::bail!("--regex and --wildcard cannot be combined");
        }

        let concept = args
            .concept
            .as_ref()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty());

        let raw_symbol = args.symbol.trim();
        if raw_symbol.is_empty() && concept.is_none() {
            anyhow::bail!("symbol is required");
        }
        let symbol = if raw_symbol.is_empty() {
            concept
                .as_ref()
                .map(|c| c.as_str())
                .unwrap_or_default()
                .to_string()
        } else {
            raw_symbol.to_string()
        };
        if symbol.len() > MAX_SYMBOL_LENGTH {
            anyhow::bail!(
                "symbol exceeds maximum length of {} characters (got {})",
                MAX_SYMBOL_LENGTH,
                symbol.len()
            );
        }

        if args.definitions_only && args.usages_only {
            anyhow::bail!("--definitions-only and --usages-only cannot be combined");
        }

        let mut path_glob_patterns = args.path_glob.clone();
        let mut path = args.path.clone();
        if let Some(raw_path) = path.clone() {
            let raw_str = raw_path.to_string_lossy();
            if has_glob_chars(&raw_str) {
                path_glob_patterns.push(raw_str.to_string());
                path = None;
            }
        }

        let root =
            path.unwrap_or(std::env::current_dir().context("failed to resolve current directory")?);
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
        let uses_css_like = language_tokens
            .iter()
            .any(|lang| lang.eq_ignore_ascii_case("css") || lang.eq_ignore_ascii_case("scss"));
        let use_ast = args.tools.use_ast_grep && !uses_css_like && !args.string;
        let path_globs = build_globset(&root, &path_glob_patterns)?;
        let mut exclude_patterns = args.exclude_patterns.clone();
        if args.exclude_tests {
            exclude_patterns.extend(TEST_EXCLUDE_GLOBS.iter().map(|s| s.to_string()));
        }
        let exclude_globs = build_globset(&root, &exclude_patterns)?;
        let include_strings = args.include_strings && !args.code_only;
        let extension_filter = if args.code_only {
            Some(code_extensions_for_languages(&language_tokens))
        } else {
            extensions_for_languages(&language_tokens, include_strings).map(|exts| {
                exts.into_iter()
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
            })
        };

        Ok(Self {
            root,
            symbol: symbol.to_string(),
            kind: args.kind,
            language,
            language_tokens,
            ignore_case: args.ignore_case,
            timeout,
            max_matches: usize::max(1, args.max_matches),
            concurrency,
            use_index,
            index_dir,
            use_rga: args.tools.enable_rga,
            use_fd: args.tools.use_fd,
            use_ast,
            cache_dir,
            log_dir,
            context_before: args.context_before,
            context_after: args.context_after,
            max_columns: DEFAULT_MAX_COLUMNS,
            body: args.body,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            concept,
            pattern_mode: if args.regex {
                PatternMode::Regex
            } else if args.wildcard {
                PatternMode::Wildcard
            } else if args.string {
                PatternMode::LiteralString
            } else {
                PatternMode::Literal
            },
            include_strings,
            code_only: args.code_only,
            definitions_only: args.definitions_only,
            usages_only: args.usages_only,
            git_info: args.git_info,
            git_filter: if args.git_modified || args.git_since.is_some() {
                Some(GitFilter {
                    modified: args.git_modified,
                    since: args.git_since.clone(),
                })
            } else {
                None
            },
            path_globs,
            exclude_globs,
            extension_filter,
            concept_expand: args.concept_expand,
        })
    }
}

pub(crate) fn passes_extension_filter(path: &Path, extensions: Option<&[String]>) -> bool {
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

fn has_glob_chars(value: &str) -> bool {
    value.contains('*') || value.contains('?') || value.contains('[')
}

fn build_globset(root: &Path, patterns: &[String]) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for raw in patterns {
        if raw.trim().is_empty() {
            continue;
        }
        let mut variants = Vec::new();
        if Path::new(raw).is_absolute() {
            variants.push(raw.clone());
        } else {
            variants.push(raw.clone());
            variants.push(root.join(raw).to_string_lossy().replace('\\', "/"));
        }
        for pattern in variants {
            let glob =
                Glob::new(&pattern).with_context(|| format!("invalid glob pattern: {raw}"))?;
            builder.add(glob);
        }
    }

    let set = builder.build()?;
    if set.is_empty() {
        Ok(None)
    } else {
        Ok(Some(set))
    }
}
