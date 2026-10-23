use serde::Deserialize;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt};
use tokio::process::Child;

/// Guard that ensures a child process is killed when dropped.
/// This prevents orphaned processes when timeouts occur.
pub struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    pub fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    /// Take ownership of the child, preventing it from being killed on drop.
    /// Use this when you want to wait for the child to complete normally.
    pub fn take(&mut self) -> Option<Child> {
        self.child.take()
    }

    /// Get a mutable reference to the child process.
    pub fn as_mut(&mut self) -> Option<&mut Child> {
        self.child.as_mut()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Attempt to kill the child process. We use start_kill() which
            // is non-blocking and doesn't wait for the process to exit.
            let _ = child.start_kill();
        }
    }
}

/// Drain a pipe concurrently with stdout, retaining only bounded diagnostics.
/// Merely reading stderr after wait can deadlock when the child's pipe fills.
pub async fn drain_stderr(mut stderr: impl AsyncRead + Unpin) -> std::io::Result<String> {
    const LIMIT: usize = 4096;
    let mut captured = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = stderr.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let keep = count.min(LIMIT - captured.len());
        captured.extend_from_slice(&buffer[..keep]);
    }
    Ok(String::from_utf8_lossy(&captured).into_owned())
}

/// Capture array-style JSON tools with an explicit output budget.
pub async fn bounded_output(
    cmd: &mut tokio::process::Command,
) -> std::io::Result<std::process::Output> {
    const LIMIT: u64 = 16 * 1024 * 1024;
    cmd.kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut guard = ChildGuard::new(cmd.spawn()?);
    let child = guard.as_mut().expect("new guard contains child");
    let stdout = child.stdout.take().expect("stdout configured as pipe");
    let stderr = child.stderr.take().expect("stderr configured as pipe");
    let collect = async {
        let result = async {
            let mut bytes = Vec::new();
            stdout.take(LIMIT + 1).read_to_end(&mut bytes).await?;
            if bytes.len() as u64 > LIMIT {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "tool output exceeds 16 MiB",
                ));
            }
            let status = guard.as_mut().unwrap().wait().await?;
            Ok((status, bytes))
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
    let (output, stderr) = tokio::join!(collect, drain_stderr(stderr));
    let (status, stdout) = output?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr: stderr?.into_bytes(),
    })
}

/// Read one JSON record without allowing an unbounded line allocation.
pub async fn next_json_line(
    reader: &mut (impl AsyncBufRead + Unpin),
) -> std::io::Result<Option<String>> {
    const LIMIT: usize = 8 * 1024 * 1024;
    let mut record = Vec::new();
    loop {
        let bytes = reader.fill_buf().await?;
        if bytes.is_empty() {
            if record.is_empty() {
                return Ok(None);
            }
            break;
        }
        let end = bytes.iter().position(|&byte| byte == b'\n');
        let count = end.map_or(bytes.len(), |index| index + 1);
        if count > LIMIT - record.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "tool JSON line exceeds 8 MiB",
            ));
        }
        record.extend_from_slice(&bytes[..count]);
        reader.consume(count);
        if end.is_some() {
            break;
        }
    }
    String::from_utf8(record)
        .map(Some)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
}

/// Shared JSON message format for ripgrep-style output.
/// Used by both `rg` and `rga` tools.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RgMessage {
    Match {
        data: RgMatchData,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub struct RgMatchData {
    pub path: RgPath,
    pub lines: RgLines,
    pub line_number: usize,
}

#[derive(Debug, Deserialize)]
pub struct RgPath {
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct RgLines {
    pub text: String,
}
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn stderr_is_fully_drained_but_capture_is_bounded() {
        let (mut writer, reader) = tokio::io::duplex(1024);
        let write = async move {
            writer.write_all(&vec![b'x'; 1024 * 1024]).await.unwrap();
        };
        let ((), captured) = tokio::join!(write, drain_stderr(reader));
        assert_eq!(captured.unwrap(), "x".repeat(4096));
    }

    #[tokio::test]
    async fn json_records_have_a_hard_size_limit() {
        let input = vec![b'x'; 8 * 1024 * 1024 + 1];
        let mut reader = &input[..];
        assert_eq!(
            next_json_line(&mut reader).await.unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        let mut reader = &b"{}\n{\"x\":1}"[..];
        assert_eq!(next_json_line(&mut reader).await.unwrap().unwrap(), "{}\n");
        assert_eq!(
            next_json_line(&mut reader).await.unwrap().unwrap(),
            "{\"x\":1}"
        );
        assert!(next_json_line(&mut reader).await.unwrap().is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn array_output_budget_kills_unbounded_producer() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", "exec yes x"]);
        let error =
            tokio::time::timeout(std::time::Duration::from_secs(5), bounded_output(&mut cmd))
                .await
                .unwrap()
                .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_output_drains_stderr_while_collecting_stdout() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args([
            "-c",
            "dd if=/dev/zero bs=65536 count=4 >&2 2>/dev/null; printf '[]'",
        ]);
        let output =
            tokio::time::timeout(std::time::Duration::from_secs(5), bounded_output(&mut cmd))
                .await
                .unwrap()
                .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"[]");
        assert_eq!(output.stderr.len(), 4096);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_guard_closes_running_child_stdout() {
        let mut child = tokio::process::Command::new("sh")
            .args(["-c", "exec sleep 30"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let guard = ChildGuard::new(child);
        drop(guard);
        let mut bytes = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            stdout.read_to_end(&mut bytes),
        )
        .await
        .unwrap()
        .unwrap();
    }
}
