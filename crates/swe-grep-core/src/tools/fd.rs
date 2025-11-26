use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

use super::common::ChildGuard;

/// Async wrapper around the `fd` command.
#[derive(Clone, Debug)]
pub struct FdTool {
    timeout: Duration,
    max_results: usize,
}

impl FdTool {
    pub fn new(timeout: Duration, max_results: usize) -> Self {
        Self {
            timeout,
            max_results,
        }
    }

    pub async fn run(&self, root: &Path, needle: &str) -> Result<Vec<PathBuf>> {
        let mut cmd = Command::new("fd");
        cmd.arg("--type")
            .arg("f")
            .arg("--hidden")
            .arg("--color")
            .arg("never")
            .arg("--max-results")
            .arg(self.max_results.to_string())
            .arg(needle)
            .arg(".");
        cmd.current_dir(root);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let child = cmd
            .spawn()
            .with_context(|| "failed to spawn fd; is it installed and on PATH?")?;

        // Wrap child in guard to ensure cleanup on timeout/early exit
        let mut guard = ChildGuard::new(child);
        let child_ref = guard.as_mut().context("child process unavailable")?;

        let stdout = child_ref
            .stdout
            .take()
            .context("fd did not produce stdout pipe")?;
        let stderr = child_ref
            .stderr
            .take()
            .context("fd did not produce stderr pipe")?;

        let mut reader = BufReader::new(stdout).lines();
        let mut matches = Vec::new();

        let collect = async {
            while let Some(line) = reader.next_line().await? {
                if line.trim().is_empty() {
                    continue;
                }
                let joined = root.join(line.trim());
                matches.push(joined);
            }

            // Take ownership from guard before waiting (prevents kill on normal exit)
            let mut child = guard.take().context("child process already taken")?;
            let status = child.wait().await?;

            if !status.success() && status.code() != Some(1) {
                // fd returns 1 when no results are found; treat this as a non-fatal outcome.
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
                    anyhow::bail!("fd exited with status {}", status);
                } else {
                    anyhow::bail!("fd exited with status {}: {}", status, stderr_output.trim());
                }
            }
            Result::<Vec<PathBuf>>::Ok(matches)
        };

        timeout(self.timeout, collect)
            .await
            .with_context(|| "fd invocation timed out")?
    }
}
