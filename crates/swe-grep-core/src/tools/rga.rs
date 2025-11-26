use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

use super::common::{ChildGuard, RgMessage};

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
        cmd.stderr(std::process::Stdio::piped());

        let child = cmd
            .spawn()
            .with_context(|| "failed to spawn rga; is ripgrep-all installed and on PATH?")?;

        // Wrap child in guard to ensure cleanup on timeout/early exit
        let mut guard = ChildGuard::new(child);
        let child_ref = guard.as_mut().context("child process unavailable")?;

        let stdout = child_ref
            .stdout
            .take()
            .context("rga did not produce stdout pipe")?;
        let stderr = child_ref
            .stderr
            .take()
            .context("rga did not produce stderr pipe")?;

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
                        tracing::warn!(error = %err, "failed to parse rga json line");
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

            // Take ownership from guard before waiting (prevents kill on normal exit)
            let mut child = guard.take().context("child process already taken")?;
            let status = child.wait().await?;

            if !status.success() && status.code() != Some(1) {
                // Capture stderr for better error diagnostics
                let mut stderr_reader = BufReader::new(stderr).lines();
                let mut stderr_output = String::new();
                while let Ok(Some(line)) = stderr_reader.next_line().await {
                    if !stderr_output.is_empty() {
                        stderr_output.push('\n');
                    }
                    stderr_output.push_str(&line);
                    if stderr_output.len() > 1024 {
                        stderr_output.push_str("\n... (truncated)");
                        break;
                    }
                }
                if stderr_output.is_empty() {
                    anyhow::bail!("rga exited with status {}", status);
                } else {
                    anyhow::bail!("rga exited with status {}: {}", status, stderr_output.trim());
                }
            }
            Result::<Vec<RgaMatch>>::Ok(matches)
        };

        timeout(self.timeout, collect)
            .await
            .with_context(|| "rga invocation timed out")?
    }
}

#[derive(Clone, Debug)]
pub struct RgaMatch {
    pub path: PathBuf,
    pub line_number: usize,
    pub lines: String,
}
