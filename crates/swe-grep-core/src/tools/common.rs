use serde::Deserialize;
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

/// Shared JSON message format for ripgrep-style output.
/// Used by both `rg` and `rga` tools.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RgMessage {
    Match { data: RgMatchData },
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
