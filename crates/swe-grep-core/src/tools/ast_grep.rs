use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

#[derive(Clone, Debug)]
pub struct AstGrepTool {
    timeout: Duration,
    max_matches: usize,
}

impl AstGrepTool {
    pub fn new(timeout: Duration, max_matches: usize) -> Self {
        Self {
            timeout,
            max_matches,
        }
    }

    pub async fn search_identifier(
        &self,
        root: &Path,
        symbol: &str,
        language: Option<&str>,
        paths: &[PathBuf],
    ) -> Result<Vec<AstGrepMatch>> {
        let lang = language.unwrap_or("rust");
        let pattern = format!("(identifier) @id (#eq? @id \"{symbol}\")");

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
        cmd.stdout(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| "failed to spawn ast-grep; is it installed and on PATH?")?;

        let stdout = child
            .stdout
            .take()
            .context("ast-grep did not produce stdout pipe")?;
        let mut reader = BufReader::new(stdout);
        let mut buffer = Vec::new();

        let collect = async {
            reader.read_to_end(&mut buffer).await?;
            let status = child.wait().await?;
            if !status.success() {
                if status.code() != Some(1) {
                    anyhow::bail!("ast-grep exited with status {}", status);
                }
            }

            // ast-grep emits either a JSON array or newline-delimited objects depending on version.
            // Try to parse both.
            let text = String::from_utf8_lossy(&buffer);
            let mut matches = Vec::new();

            if text.trim().is_empty() {
                return Ok(matches);
            }

            if let Ok(parsed) = serde_json::from_str::<Vec<AstGrepMessage>>(&text) {
                for msg in parsed.into_iter().take(self.max_matches) {
                    matches.push(msg.into());
                }
                return Ok(matches);
            }

            for line in text.lines() {
                match serde_json::from_str::<AstGrepMessage>(line) {
                    Ok(msg) => {
                        if matches.len() >= self.max_matches {
                            break;
                        }
                        matches.push(msg.into());
                    }
                    Err(err) => {
                        eprintln!("warn: failed to parse ast-grep json line: {err}");
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
