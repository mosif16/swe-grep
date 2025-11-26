use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

use super::common::{ChildGuard, RgMessage};

#[derive(Clone, Debug)]
pub struct RipgrepTool {
    timeout: Duration,
    max_matches: usize,
    context_before: usize,
    context_after: usize,
    max_columns: usize,
    threads: usize,
}

impl RipgrepTool {
    pub fn new(
        timeout: Duration,
        max_matches: usize,
        context_before: usize,
        context_after: usize,
        max_columns: usize,
        threads: usize,
    ) -> Self {
        Self {
            timeout,
            max_matches,
            context_before,
            context_after,
            max_columns,
            threads: usize::max(1, threads),
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
            .arg("--threads")
            .arg(self.threads.to_string())
            .arg("--max-columns")
            .arg(self.max_columns.to_string())
            .arg("--smart-case")
            .arg("--max-count")
            .arg(self.max_matches.to_string());

        if self.context_before > 0 {
            cmd.arg("--before-context")
                .arg(self.context_before.to_string());
        }
        if self.context_after > 0 {
            cmd.arg("--after-context")
                .arg(self.context_after.to_string());
        }

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
        cmd.stderr(std::process::Stdio::piped());

        let child = cmd
            .spawn()
            .with_context(|| "failed to spawn ripgrep; is rg installed and on PATH?")?;

        // Wrap child in guard to ensure cleanup on timeout/early exit
        let mut guard = ChildGuard::new(child);
        let child_ref = guard.as_mut().context("child process unavailable")?;

        let stdout = child_ref
            .stdout
            .take()
            .context("ripgrep did not produce stdout pipe")?;
        let stderr = child_ref
            .stderr
            .take()
            .context("ripgrep did not produce stderr pipe")?;

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
                        tracing::warn!(error = %err, "failed to parse ripgrep json line");
                        continue;
                    }
                };
                if let RgMessage::Match { data } = parsed {
                    let path = PathBuf::from(data.path.text);
                    matches.push(RipgrepMatch {
                        path,
                        line_number: data.line_number,
                        lines: data.lines.text,
                        raw_json: line.clone(),
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
                    anyhow::bail!("ripgrep exited with status {}", status);
                } else {
                    anyhow::bail!(
                        "ripgrep exited with status {}: {}",
                        status,
                        stderr_output.trim()
                    );
                }
            }
            Result::<Vec<RipgrepMatch>>::Ok(matches)
        };

        timeout(self.timeout, collect)
            .await
            .with_context(|| "ripgrep invocation timed out")?
    }
}

#[derive(Clone, Debug)]
pub struct RipgrepMatch {
    pub path: PathBuf,
    pub line_number: usize,
    pub lines: String,
    pub raw_json: String,
}
