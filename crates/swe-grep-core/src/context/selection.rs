use std::borrow::Cow;
use std::path::{Component, Path};

use anyhow::{Context, Result, bail};

use super::{SymbolCandidate, SymbolResolution};
use crate::analysis::{FileFacts, normalize_symbol};

pub fn resolve_target_file(root: &Path, file: Option<&Path>) -> Result<Option<String>> {
    let Some(file) = file else { return Ok(None) };
    if file.is_absolute()
        || file
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        bail!("target file must be relative to the analysis directory without parent traversal");
    }
    let canonical = root
        .join(file)
        .canonicalize()
        .context("cannot resolve target file")?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        bail!("target file must be a regular file inside the analysis directory");
    }
    Ok(Some(
        canonical
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/"),
    ))
}

pub fn resolve_symbol(
    files: &[FileFacts],
    symbol: &str,
    target_file: Option<&str>,
    target_line: Option<usize>,
) -> SymbolResolution {
    let target = normalize_symbol(symbol);
    let qualified_suffix = format!("::{symbol}");
    let mut best_score = 0;
    let mut candidates = Vec::new();
    for file in files {
        if target_file.is_some_and(|target| target != file.file) {
            continue;
        }
        for def in &file.facts.definitions {
            if target_line.is_some_and(|line| line != def.line) {
                continue;
            }
            let score = if def.symbol == symbol {
                3
            } else if def.symbol.ends_with(&qualified_suffix) {
                2
            } else if normalize_symbol(&def.symbol) == target {
                1
            } else {
                0
            };
            if score == 0 || score < best_score {
                continue;
            }
            if score > best_score {
                candidates.clear();
                best_score = score;
            }
            candidates.push(SymbolCandidate {
                symbol: def.symbol.clone(),
                file: file.file.clone(),
                line: def.line,
            });
        }
    }
    candidates.sort_by(|a, b| (&a.file, a.line, &a.symbol).cmp(&(&b.file, b.line, &b.symbol)));
    candidates.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.symbol == b.symbol);
    SymbolResolution {
        status: match candidates.len() {
            0 => "not_found",
            1 => "resolved",
            _ => "ambiguous",
        }
        .to_string(),
        candidates,
    }
}

/// Restrict outgoing calls of the selected symbol to its declaration body, while
/// retaining other functions and files for caller discovery and transitive edges.
pub fn select_symbol_facts_in_place(files: &mut [FileFacts], target: &SymbolCandidate) {
    let target_name = simple_symbol(&target.symbol);
    // Compute the same-name ownership spans once, before pruning any definitions.
    // Reusing owned extraction results avoids cloning the repository fact corpus.
    let owners: Vec<_> = files
        .iter()
        .find(|file| file.file == target.file)
        .into_iter()
        .flat_map(|file| &file.facts.definitions)
        .filter(|def| simple_symbol(&def.symbol) == target_name)
        .filter_map(|def| {
            def.span.map(|(start, end)| {
                (
                    start,
                    end,
                    def.line == target.line && def.symbol == target.symbol,
                )
            })
        })
        .collect();
    for file in files {
        file.facts.calls.retain(|call| {
            if simple_symbol(&call.caller) != target_name {
                return true;
            }
            if file.file != target.file {
                return false;
            }
            // Nested declarations can share a name; the smallest enclosing
            // definition owns a call, not every outer span that contains it.
            owners
                .iter()
                .filter(|(start, end, _)| call.line >= *start && call.line <= *end)
                .min_by_key(|(start, end, _)| end.saturating_sub(*start))
                .is_some_and(|(_, _, selected)| *selected)
        });
        file.facts.definitions.retain(|def| {
            simple_symbol(&def.symbol) != target_name
                || (file.file == target.file
                    && def.line == target.line
                    && def.symbol == target.symbol)
        });
    }
}

pub fn target_has_ambiguous_callers(files: &[FileFacts], target: &SymbolCandidate) -> bool {
    // The name-based graph cannot prove which declaration an unqualified call
    // targets when multiple definitions share the same simple name.
    let name = simple_symbol(&target.symbol);
    files
        .iter()
        .flat_map(|f| f.facts.definitions.iter())
        .filter(|d| simple_symbol(&d.symbol) == name)
        .take(2)
        .count()
        > 1
}

fn simple_symbol(symbol: &str) -> Cow<'_, str> {
    let trimmed = symbol.trim();
    // Fact names are normally clean ASCII identifiers or ::-qualified names.
    // Borrow their last component; keep the existing normalization for other
    // syntax, including Unicode and punctuation, rather than changing identity.
    if !trimmed.is_empty()
        && trimmed.split("::").all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'$')
        })
    {
        return Cow::Borrowed(trimmed.rsplit("::").next().unwrap_or(trimmed));
    }
    let normalized = normalize_symbol(symbol);
    Cow::Owned(
        normalized
            .rsplit("::")
            .next()
            .unwrap_or(&normalized)
            .rsplit('.')
            .next()
            .unwrap_or(&normalized)
            .to_string(),
    )
}
