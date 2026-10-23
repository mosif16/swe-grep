use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use grep_matcher::Matcher;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::SearcherBuilder;
use grep_searcher::sinks::UTF8;
use ignore::WalkBuilder;

#[derive(Clone, Debug)]
pub struct EmbeddedMatch {
    pub path: PathBuf,
    pub line_number: usize,
    pub lines: String,
    pub absolute_offset: u64,
}

pub fn search_embedded(
    root: &Path,
    patterns: &[String],
    max_matches: usize,
) -> Result<Vec<EmbeddedMatch>> {
    if patterns.is_empty() || max_matches == 0 {
        return Ok(Vec::new());
    }

    let matcher = RegexMatcherBuilder::new()
        .case_smart(true)
        .build_many(patterns)
        .context("failed to build embedded regex matcher")?;

    let mut searcher = SearcherBuilder::new().line_number(true).build();
    let mut matches = Vec::new();

    let mut walker = WalkBuilder::new(root);
    walker
        .hidden(true)
        .ignore(true)
        .parents(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false);

    for entry in walker.build() {
        if matches.len() >= max_matches {
            break;
        }

        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::debug!(error = %err, "walker error");
                continue;
            }
        };

        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        let path = entry.into_path();
        let file_bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::debug!(error = %err, path = %path.display(), "failed to read file");
                continue;
            }
        };
        let line_offsets = compute_line_offsets(&file_bytes);
        let mut sink = {
            let path_for_result = path.clone();
            let matcher = &matcher;
            let line_offsets = line_offsets;
            let matches_ref = &mut matches;
            let max_matches = max_matches;

            UTF8(move |line_number, line| {
                if matches_ref.len() >= max_matches {
                    return Ok(false);
                }

                let line_index = line_number.saturating_sub(1) as usize;
                let base_offset = line_offsets
                    .get(line_index)
                    .copied()
                    .unwrap_or_else(|| *line_offsets.last().unwrap_or(&0));

                let relative_offset = matcher
                    .find(line.as_bytes())
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?
                    .map(|m| m.start() as u64)
                    .unwrap_or(0);

                matches_ref.push(EmbeddedMatch {
                    path: path_for_result.clone(),
                    line_number: line_number as usize,
                    lines: line.to_string(),
                    absolute_offset: base_offset + relative_offset,
                });

                Ok(matches_ref.len() < max_matches)
            })
        };

        if let Err(err) = searcher.search_slice(&matcher, &file_bytes, &mut sink) {
            tracing::debug!(error = %err, path = %path.display(), "embedded search failed");
        }
    }

    Ok(matches)
}

fn compute_line_offsets(bytes: &[u8]) -> Vec<u64> {
    let mut offsets = Vec::with_capacity(64);
    offsets.push(0);
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            offsets.push((idx + 1) as u64);
        }
    }
    offsets
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_patterns_cannot_escape_their_alternative() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.txt");
        fs::write(&path, "x\nX\n").unwrap();
        let matches = search_embedded(&path, &["(?i)nomatch".into(), "X".into()], 10).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line_number, 2);
    }

    #[test]
    fn offsets_are_bytes_and_limit_is_global() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.txt");
        fs::write(&path, "é\nλ target\nnext target\n").unwrap();
        let matches = search_embedded(&path, &["target".into()], 1).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line_number, 2);
        assert_eq!(matches[0].absolute_offset, 6);
    }
}
