use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

#[derive(Clone, Debug)]
pub struct RipgrepTool {
    timeout: Duration,
    max_matches: usize,
}

impl RipgrepTool {
    pub fn new(timeout: Duration, max_matches: usize) -> Self {
        Self {
            timeout,
            max_matches,
        }
    }

    pub async fn search_union(
        &self,
        root: &Path,
        queries: &[String],
        paths: &[PathBuf],
    ) -> Result<Vec<RipgrepMatch>> {
        if queries.is_empty() {
            return Ok(Vec::new());
        }

        let mut cmd = Command::new("rg");
        cmd.arg("--json")
            .arg("--line-number")
            .arg("--column")
            .arg("--max-columns")
            .arg("200")
            .arg("--smart-case")
            .arg("--max-count")
            .arg(self.max_matches.to_string());

        for query in queries {
            cmd.arg("-e").arg(query);
        }

        if paths.is_empty() {
            cmd.arg(".");
        } else {
            for path in paths.iter().take(self.max_matches) {
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
            .with_context(|| "failed to spawn ripgrep; is rg installed and on PATH?")?;

        let stdout = child
            .stdout
            .take()
            .context("ripgrep did not produce stdout pipe")?;
        let mut reader = BufReader::new(stdout).lines();
        let mut matches = Vec::new();
        let max_matches = self.max_matches;

        let collect = async {
            while let Some(line) = reader.next_line().await? {
                if matches.len() >= max_matches {
                    break;
                }
                let parsed: RgMessage = match serde_json::from_str(&line) {
                    Ok(msg) => msg,
                    Err(err) => {
                        eprintln!("warn: failed to parse ripgrep json line: {err}");
                        continue;
                    }
                };
                if let RgMessage::Match { data } = parsed {
                    let path = PathBuf::from(data.path.text);
                    matches.push(RipgrepMatch {
                        path,
                        line_number: data.line_number,
                        lines: data.lines.text,
                    });
                }
            }
            let status = child.wait().await?;
            if !status.success() {
                if status.code() != Some(1) {
                    anyhow::bail!("ripgrep exited with status {}", status);
                }
            }
            Result::<Vec<RipgrepMatch>>::Ok(matches)
        };

        timeout(self.timeout, collect)
            .await
            .with_context(|| "ripgrep invocation timed out")?
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum RgMessage {
    Match {
        data: RgMatchData,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct RgMatchData {
    path: RgPath,
    lines: RgLines,
    line_number: usize,
}

#[derive(Debug, Deserialize)]
struct RgPath {
    text: String,
}

#[derive(Debug, Deserialize)]
struct RgLines {
    text: String,
}

#[derive(Clone, Debug)]
pub struct RipgrepMatch {
    pub path: PathBuf,
    pub line_number: usize,
    pub lines: String,
}
