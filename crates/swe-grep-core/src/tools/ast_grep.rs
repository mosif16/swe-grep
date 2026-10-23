use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::language;

#[derive(Clone, Debug)]
pub struct AstGrepTool {
    timeout: Duration,
    max_matches: usize,
    verbose: bool,
}

impl AstGrepTool {
    pub fn new(timeout: Duration, max_matches: usize, verbose: bool) -> Self {
        Self {
            timeout,
            max_matches,
            verbose,
        }
    }

    pub async fn search_identifier(
        &self,
        root: &Path,
        symbol: &str,
        languages: &[String],
        paths: &[PathBuf],
    ) -> Result<Vec<AstGrepMatch>> {
        // Default to Rust if no languages specified
        let hints: Vec<String> = if languages.is_empty() {
            vec!["rust".to_string()]
        } else {
            languages.iter().cloned().collect()
        };
        let mut aggregated: Vec<AstGrepMatch> = Vec::new();
        let mut seen: HashSet<(PathBuf, usize)> = HashSet::new();

        for lang in hints {
            let patterns = language::ast_patterns_for(&lang, symbol);
            for pattern in patterns {
                if aggregated.len() >= self.max_matches {
                    break;
                }
                let remaining = self.max_matches.saturating_sub(aggregated.len());
                let matches = self
                    .run_pattern(root, &lang, &pattern, paths, remaining)
                    .await?;

                for m in matches {
                    let key = (m.path.clone(), m.line);
                    if seen.insert(key) {
                        aggregated.push(m);
                        if aggregated.len() >= self.max_matches {
                            break;
                        }
                    }
                }
            }
        }

        Ok(aggregated)
    }

    async fn run_pattern(
        &self,
        root: &Path,
        lang: &str,
        pattern: &str,
        paths: &[PathBuf],
        limit: usize,
    ) -> Result<Vec<AstGrepMatch>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut cmd = Command::new("ast-grep");
        cmd.arg("--json")
            .arg("--pattern")
            .arg(pattern)
            .arg("--lang")
            .arg(lang);

        if paths.is_empty() {
            cmd.arg(".");
        } else {
            for path in paths {
                let absolute = if path.is_absolute() {
                    path.clone()
                } else {
                    root.join(path)
                };
                let relative = absolute
                    .strip_prefix(root)
                    .map(|p| p.to_path_buf())
                    .unwrap_or(absolute);
                cmd.arg(relative);
            }
        }
        cmd.current_dir(root);
        cmd.kill_on_drop(true);

        let collect = async {
            let output = super::common::bounded_output(&mut cmd)
                .await
                .with_context(|| "failed to spawn ast-grep; is it installed and on PATH?")?;

            let stderr_text = String::from_utf8_lossy(&output.stderr);
            if let Some(diagnostic) = stderr_text
                .lines()
                .find(|line| line.contains("Pattern contains an ERROR node"))
            {
                return Err(AstPatternError::new(
                    pattern.to_string(),
                    diagnostic.trim().to_string(),
                )
                .into());
            }

            if !output.status.success() && output.status.code() != Some(1) {
                let trimmed = stderr_text.trim();
                if !trimmed.is_empty() {
                    warn!(
                        target: "swe_grep::tools::ast_grep",
                        "ast-grep stderr: {trimmed}"
                    );
                }
                anyhow::bail!("ast-grep exited with status {}", output.status);
            }

            if let Some(line) = stderr_text.lines().map(str::trim).find(|s| !s.is_empty()) {
                debug!(
                    target: "swe_grep::tools::ast_grep",
                    "ast-grep diagnostic: {line}"
                );
            }

            let text = String::from_utf8_lossy(&output.stdout);
            let mut matches = Vec::new();

            if text.trim().is_empty() {
                return Ok(matches);
            }

            if let Ok(parsed) = serde_json::from_str::<Vec<AstGrepMessage>>(&text) {
                for msg in parsed.into_iter().take(limit) {
                    matches.push(msg.into());
                }
                return Ok(matches);
            }

            for line in text.lines() {
                match serde_json::from_str::<AstGrepMessage>(line) {
                    Ok(msg) => {
                        if matches.len() >= limit {
                            break;
                        }
                        matches.push(msg.into());
                    }
                    Err(err) => {
                        if self.verbose {
                            tracing::warn!(error = %err, "failed to parse ast-grep json line");
                        } else {
                            tracing::debug!(error = %err, "failed to parse ast-grep json line");
                        }
                    }
                }
            }

            Ok(matches)
        };

        timeout(self.timeout, collect)
            .await
            .with_context(|| "ast-grep invocation timed out")?
    }
}

#[derive(Debug, Deserialize)]
struct AstGrepMessage {
    #[serde(alias = "file")]
    path: String,
    range: AstGrepRange,
}

#[derive(Debug, Deserialize)]
struct AstGrepRange {
    start: AstGrepPosition,
    #[allow(dead_code)]
    end: AstGrepPosition,
}

#[derive(Debug, Deserialize)]
struct AstGrepPosition {
    line: usize,
    #[allow(dead_code)]
    column: usize,
}

#[derive(Clone, Debug)]
pub struct AstGrepMatch {
    pub path: PathBuf,
    pub line: usize,
}

impl From<AstGrepMessage> for AstGrepMatch {
    fn from(value: AstGrepMessage) -> Self {
        Self {
            path: PathBuf::from(value.path),
            line: value.range.start.line,
        }
    }
}

#[derive(Debug)]
pub struct AstPatternError {
    pattern: String,
    message: String,
}

impl AstPatternError {
    pub fn new(pattern: String, message: String) -> Self {
        Self { pattern, message }
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for AstPatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ast-grep pattern error for '{}': {}",
            self.pattern, self.message
        )
    }
}

impl std::error::Error for AstPatternError {}
