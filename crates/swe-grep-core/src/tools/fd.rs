use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

use super::common::{ChildGuard, drain_stderr};

fn path_from_bytes(bytes: Vec<u8>) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(std::ffi::OsString::from_vec(bytes).into())
    }
    #[cfg(not(unix))]
    {
        Ok(String::from_utf8(bytes)?.into())
    }
}

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
        if self.max_results == 0 {
            return Ok(Vec::new());
        }
        let mut cmd = Command::new("fd");
        cmd.arg("--type")
            .arg("f")
            .arg("--hidden")
            .arg("--color")
            .arg("never")
            .arg("--max-results")
            .arg(self.max_results.to_string())
            .arg("--print0")
            .arg("--")
            .arg(needle)
            .arg(".");
        cmd.current_dir(root);
        self.collect_command(root, cmd).await
    }

    async fn collect_command(&self, root: &Path, mut cmd: Command) -> Result<Vec<PathBuf>> {
        cmd.kill_on_drop(true);
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

        let mut reader = BufReader::new(stdout);
        let mut matches = Vec::new();

        let collect = async {
            let result = async {
                let mut record = Vec::new();
                while matches.len() < self.max_results {
                    record.clear();
                    if reader.read_until(0, &mut record).await? == 0 {
                        break;
                    }
                    if record.last() == Some(&0) {
                        record.pop();
                    }
                    if !record.is_empty() {
                        matches.push(root.join(path_from_bytes(record.clone())?));
                    }
                }
                if matches.len() >= self.max_results {
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
                    anyhow::bail!("fd exited with status {}", status);
                }
                Result::<Vec<PathBuf>>::Ok(matches)
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
        .with_context(|| "fd invocation timed out")?
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_decoding_preserves_spaces_and_newlines() {
        assert_eq!(
            path_from_bytes(b" a\nb ".to_vec()).unwrap(),
            PathBuf::from(" a\nb ")
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_decoding_preserves_non_utf8_names() {
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(
            path_from_bytes(vec![0xff]).unwrap().as_os_str().as_bytes(),
            &[0xff]
        );
    }

    #[tokio::test]
    async fn zero_limit_does_not_spawn_fd() {
        assert!(
            FdTool::new(Duration::ZERO, 0)
                .run(Path::new("missing"), "x")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stderr_larger_than_pipe_does_not_deadlock_stdout() {
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            "dd if=/dev/zero bs=65536 count=4 >&2 2>/dev/null; printf ' a\\nb \\000'",
        ]);
        let tool = FdTool::new(Duration::from_secs(5), 10);
        let paths = tool.collect_command(Path::new("root"), cmd).await.unwrap();
        assert_eq!(paths, vec![PathBuf::from("root/ a\nb ")]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cap_kills_producer_instead_of_waiting_with_full_stdout_pipe() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "while :; do printf 'file\\000'; done"]);
        let tool = FdTool::new(Duration::from_secs(5), 1);
        let paths = tool.collect_command(Path::new("root"), cmd).await.unwrap();
        assert_eq!(paths, vec![PathBuf::from("root/file")]);
    }
}
