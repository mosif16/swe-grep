use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

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

        let mut child = cmd
            .spawn()
            .with_context(|| "failed to spawn fd; is it installed and on PATH?")?;

        let stdout = child
            .stdout
            .take()
            .context("fd did not produce stdout pipe")?;
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
            let status = child.wait().await?;
            if !status.success() {
                // fd returns 1 when no results are found; treat this as a non-fatal outcome.
                if status.code() != Some(1) {
                    anyhow::bail!("fd exited with status {}", status);
                }
            }
            Result::<Vec<PathBuf>>::Ok(matches)
        };

        timeout(self.timeout, collect)
            .await
            .with_context(|| "fd invocation timed out")?
    }
}
