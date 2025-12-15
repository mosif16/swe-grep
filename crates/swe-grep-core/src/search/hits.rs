use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::search::config::normalize_path;
use crate::tools::rg::RipgrepMatch;
use crate::tools::rga::RgaMatch;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitSemanticKind {
    Definition,
    Implementation,
    Import,
    Test,
    Usage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageContextKind {
    Construction,
    MethodCall,
    TypeReference,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticClassification {
    pub kind: HitSemanticKind,
    pub usage_context: Option<UsageContextKind>,
    pub trait_name: Option<String>,
}

pub(crate) fn classify_semantic(
    path: &Path,
    snippet: &str,
    symbol: &str,
) -> SemanticClassification {
    if is_test_path(path) {
        return SemanticClassification {
            kind: HitSemanticKind::Test,
            usage_context: None,
            trait_name: None,
        };
    }

    let snippet = snippet.trim();
    if snippet.is_empty() {
        return SemanticClassification {
            kind: HitSemanticKind::Usage,
            usage_context: Some(UsageContextKind::Other),
            trait_name: None,
        };
    }

    if looks_like_import(snippet, symbol) {
        return SemanticClassification {
            kind: HitSemanticKind::Import,
            usage_context: Some(UsageContextKind::Other),
            trait_name: None,
        };
    }

    if looks_like_definition(snippet, symbol) {
        return SemanticClassification {
            kind: HitSemanticKind::Definition,
            usage_context: None,
            trait_name: None,
        };
    }

    let trait_name = extract_trait_name(snippet, symbol);
    if looks_like_implementation(snippet, symbol) {
        return SemanticClassification {
            kind: HitSemanticKind::Implementation,
            usage_context: None,
            trait_name,
        };
    }

    SemanticClassification {
        kind: HitSemanticKind::Usage,
        usage_context: Some(classify_usage_context(snippet, symbol)),
        trait_name: None,
    }
}

fn is_test_path(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.contains("/tests/")
        || normalized.contains("/__tests__/")
        || normalized.contains("/test/")
        || normalized.contains("/spec/")
    {
        return true;
    }

    let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    filename.ends_with("_test.rs")
        || filename.ends_with("_test.go")
        || filename.ends_with(".test.ts")
        || filename.ends_with(".test.tsx")
        || filename.ends_with(".spec.ts")
        || filename.ends_with("Tests.swift")
        || filename.ends_with("Spec.swift")
}

fn looks_like_import(snippet: &str, symbol: &str) -> bool {
    // Rust
    let trimmed = snippet.trim_start();
    if (trimmed.starts_with("use ") || trimmed.starts_with("pub use ")) && trimmed.contains(symbol)
    {
        return true;
    }
    if trimmed.starts_with("mod ") && trimmed.contains(symbol) {
        return true;
    }

    // TypeScript/JavaScript
    if (trimmed.starts_with("import ") || trimmed.starts_with("export "))
        && trimmed.contains(symbol)
    {
        return true;
    }

    // Python
    if (trimmed.starts_with("from ") || trimmed.starts_with("import ")) && trimmed.contains(symbol)
    {
        return true;
    }

    // Go
    if trimmed.starts_with("import ") && trimmed.contains(symbol) {
        return true;
    }

    false
}

fn looks_like_definition(snippet: &str, symbol: &str) -> bool {
    let trimmed = snippet.trim_start();

    // Rust
    for keyword in ["struct", "enum", "trait", "fn", "type", "mod"] {
        if trimmed.contains(keyword) && trimmed.contains(symbol) {
            let needle = format!("{keyword} {symbol}");
            if trimmed.contains(&needle) {
                return true;
            }
            let needle_pub = format!("pub {keyword} {symbol}");
            if trimmed.contains(&needle_pub) {
                return true;
            }
            let needle_pub_crate = format!("pub(crate) {keyword} {symbol}");
            if trimmed.contains(&needle_pub_crate) {
                return true;
            }
        }
    }

    // Swift
    for keyword in ["class", "struct", "enum", "protocol", "actor", "func"] {
        let needle = format!("{keyword} {symbol}");
        if trimmed.contains(&needle) {
            return true;
        }
    }

    // TypeScript/JavaScript
    for keyword in ["class", "interface", "type", "function", "const"] {
        let needle = format!("{keyword} {symbol}");
        if trimmed.contains(&needle) {
            return true;
        }
        let needle_export = format!("export {keyword} {symbol}");
        if trimmed.contains(&needle_export) {
            return true;
        }
    }

    // Python
    if trimmed.starts_with("class ") && trimmed.contains(symbol) {
        let needle = format!("class {symbol}");
        return trimmed.contains(&needle);
    }

    // Go
    if trimmed.starts_with("type ") && trimmed.contains(symbol) {
        let needle = format!("type {symbol} ");
        return trimmed.contains(&needle);
    }

    false
}

fn looks_like_implementation(snippet: &str, symbol: &str) -> bool {
    let trimmed = snippet.trim_start();

    // Rust
    if trimmed.starts_with("impl ") && trimmed.contains(symbol) {
        return true;
    }

    // Swift extensions
    if trimmed.starts_with("extension ") && trimmed.contains(symbol) {
        return true;
    }

    // TypeScript/JavaScript: class Foo implements Bar
    if trimmed.contains(" implements ") && trimmed.contains(symbol) {
        return true;
    }

    false
}

fn extract_trait_name(snippet: &str, symbol: &str) -> Option<String> {
    let trimmed = snippet.trim_start();
    if !trimmed.starts_with("impl ") {
        return None;
    }

    let after_impl = trimmed.trim_start_matches("impl").trim_start();
    let without_generics = if after_impl.starts_with('<') {
        match after_impl.find('>') {
            Some(end) => after_impl[end + 1..].trim_start(),
            None => after_impl,
        }
    } else {
        after_impl
    };

    let Some(for_pos) = without_generics.find(" for ") else {
        let candidate = without_generics
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches('{')
            .trim_matches('(')
            .trim_matches(')');
        if candidate.is_empty() || candidate.contains(symbol) {
            return None;
        }
        return Some(candidate.to_string());
    };

    let trait_part = without_generics[..for_pos].trim();
    if trait_part.is_empty() || trait_part.contains(symbol) {
        return None;
    }
    Some(trait_part.to_string())
}

fn classify_usage_context(snippet: &str, symbol: &str) -> UsageContextKind {
    if snippet.contains(&format!("new {symbol}")) {
        return UsageContextKind::Construction;
    }
    if snippet.contains(&format!("{symbol}::new"))
        || snippet.contains(&format!("{symbol}("))
        || snippet.contains(&format!("{symbol} <"))
    {
        return UsageContextKind::Construction;
    }
    if snippet.contains(&format!("{symbol}::")) || snippet.contains(&format!("{symbol}.")) {
        return UsageContextKind::MethodCall;
    }
    if snippet.contains(&format!(": {symbol}"))
        || snippet.contains(&format!("<{symbol}>"))
        || snippet.contains(&format!("&{symbol}"))
        || snippet.contains(&format!("*{symbol}"))
    {
        return UsageContextKind::TypeReference;
    }
    UsageContextKind::Other
}

#[derive(Clone, Debug)]
pub(crate) struct SearchHit {
    pub(crate) path: PathBuf,
    pub(crate) line: usize,
    pub(crate) snippet: String,
    pub(crate) score: f32,
    pub(crate) origin: HitOrigin,
    pub(crate) raw_snippet: Option<String>,
    pub(crate) snippet_length: usize,
    pub(crate) raw_snippet_truncated: bool,
}

#[derive(Clone)]
pub(crate) struct BodyPayload {
    pub(crate) body: Option<String>,
    pub(crate) retrieved: bool,
    pub(crate) truncated: bool,
}

impl SearchHit {
    pub(crate) fn from_ripgrep(
        root: &Path,
        rg_match: RipgrepMatch,
        kind: ProbeKind,
        max_columns: usize,
    ) -> Self {
        let absolute = if rg_match.path.is_absolute() {
            rg_match.path.clone()
        } else {
            root.join(&rg_match.path)
        };
        let normalized = normalize_path(root, &absolute).unwrap_or(absolute);
        let line = rg_match.line_number;
        let snippet_length = rg_match.lines.chars().count();
        let raw_snippet_truncated = max_columns != 0 && snippet_length >= max_columns;
        Self {
            path: normalized,
            line,
            snippet: rg_match.lines,
            score: 1.0,
            origin: HitOrigin::Ripgrep(kind),
            raw_snippet: Some(rg_match.raw_json),
            snippet_length,
            raw_snippet_truncated,
        }
    }

    pub(crate) fn from_rga(root: &Path, rga_match: RgaMatch) -> Self {
        let absolute = if rga_match.path.is_absolute() {
            rga_match.path.clone()
        } else {
            root.join(&rga_match.path)
        };
        let normalized = normalize_path(root, &absolute).unwrap_or(absolute);
        let line = rga_match.line_number;
        let snippet_length = rga_match.lines.chars().count();
        Self {
            path: normalized,
            line,
            snippet: rga_match.lines,
            score: 0.9,
            origin: HitOrigin::Rga,
            raw_snippet: None,
            snippet_length,
            raw_snippet_truncated: false,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum HitOrigin {
    Ripgrep(ProbeKind),
    AstGrep,
    Rga,
}

impl HitOrigin {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            HitOrigin::Ripgrep(ProbeKind::Scoped) => "rg-scoped",
            HitOrigin::Ripgrep(ProbeKind::Global) => "rg-global",
            #[cfg(feature = "indexing")]
            HitOrigin::Ripgrep(ProbeKind::Indexed) => "rg-indexed",
            HitOrigin::AstGrep => "ast-grep",
            HitOrigin::Rga => "rga",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ProbeKind {
    Scoped,
    Global,
    #[cfg(feature = "indexing")]
    Indexed,
}

#[derive(Default)]
pub(crate) struct SearchCache {
    seen: HashSet<(String, usize)>,
}

impl SearchCache {
    pub(crate) fn retain_new(&mut self, hits: &mut Vec<SearchHit>) {
        hits.retain(|hit| {
            let key = (hit.path.to_string_lossy().to_string(), hit.line);
            if self.seen.contains(&key) {
                false
            } else {
                self.seen.insert(key);
                true
            }
        });
    }
}
