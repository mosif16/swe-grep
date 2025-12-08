use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::search::config::normalize_path;
use crate::tools::rg::RipgrepMatch;
use crate::tools::rga::RgaMatch;

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
        let raw_snippet_truncated = snippet_length >= max_columns;
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
