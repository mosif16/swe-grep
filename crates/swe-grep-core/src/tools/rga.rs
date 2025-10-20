use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

#[derive(Clone, Debug)]
pub struct RgaTool {
    timeout: Duration,
    max_matches: usize,
}

impl RgaTool {
    pub fn new(timeout: Duration, max_matches: usize) -> Self {
        Self {
            timeout,
            max_matches,
        }
    }

    pub async fn search(&self, root: &Path, query: &str) -> Result<Vec<RgaMatch>> {
        let mut cmd = Command::new("rga");
        cmd.arg("--json")
            .arg("--line-number")
            .arg("--column")
            .arg("--max-columns")
            .arg("200")
            .arg(query)
            .arg(".");
        cmd.current_dir(root);
        cmd.stdout(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| "failed to spawn rga; is ripgrep-all installed and on PATH?")?;

        let stdout = child
            .stdout
            .take()
            .context("rga did not produce stdout pipe")?;
        let mut reader = BufReader::new(stdout).lines();
        let mut matches = Vec::new();

        let collect = async {
            while let Some(line) = reader.next_line().await? {
                if matches.len() >= self.max_matches {
                    break;
                }
                let parsed: RgMessage = match serde_json::from_str(&line) {
                    Ok(msg) => msg,
                    Err(err) => {
                        eprintln!("warn: failed to parse rga json line: {err}");
                        continue;
                    }
                };
                if let RgMessage::Match { data } = parsed {
                    let path = PathBuf::from(data.path.text);
                    matches.push(RgaMatch {
                        path,
                        line_number: data.line_number,
                        lines: data.lines.text,
                    });
                }
            }
            let status = child.wait().await?;
            if !status.success() {
                if status.code() != Some(1) {
                    anyhow::bail!("rga exited with status {}", status);
                }
            }
            Result::<Vec<RgaMatch>>::Ok(matches)
        };

        timeout(self.timeout, collect)
            .await
            .with_context(|| "rga invocation timed out")?
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
pub struct RgaMatch {
    pub path: PathBuf,
    pub line_number: usize,
    pub lines: String,
}
