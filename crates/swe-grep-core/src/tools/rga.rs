use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::BufReader;
use tokio::process::Command;
use tokio::time::timeout;

use super::common::{ChildGuard, RgMessage, drain_stderr, next_json_line};

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
        if self.max_matches == 0 {
            return Ok(Vec::new());
        }
        let mut cmd = Command::new("rga");
        cmd.arg("--json")
            .arg("--line-number")
            .arg("--column")
            .arg("--max-columns")
            .arg("200")
            .arg("-e")
            .arg(query)
            .arg(".");
        cmd.current_dir(root);
        cmd.kill_on_drop(true);
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

        let mut reader = BufReader::new(stdout);
        let mut matches = Vec::new();
        let max_matches = self.max_matches;

        let collect = async {
            let result = async {
                while matches.len() < max_matches {
                    let Some(line) = next_json_line(&mut reader).await? else {
                        break;
                    };
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

                if matches.len() >= max_matches {
                    let child = guard.as_mut().context("child process unavailable")?;
                    let _ = child.start_kill();
                    child.wait().await?;
                    return Ok(matches);
                }
                let status = guard
                    .as_mut()
                    .context("child process unavailable")?
                    .wait()
                    .await?;
                if !status.success() && status.code() != Some(1) {
                    anyhow::bail!("rga exited with status {}", status);
                }
                Result::<Vec<RgaMatch>>::Ok(matches)
            }
            .await;
            if result.is_err() {
                if let Some(child) = guard.as_mut() {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                }
            }
            result
        };

        timeout(self.timeout, async {
            let (result, diagnostics) = tokio::join!(collect, drain_stderr(stderr));
            result.with_context(|| diagnostics.unwrap_or_default())
        })
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
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn zero_limit_does_not_spawn_rga() {
        assert!(
            RgaTool::new(Duration::ZERO, 0)
                .search(Path::new("missing"), "x")
                .await
                .unwrap()
                .is_empty()
        );
    }
}
