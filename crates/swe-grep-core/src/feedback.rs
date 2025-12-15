use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_FEEDBACK_DIR: &str = "Feedback";
pub const SHELL_TOOL_NAME: &str = "shell_command";
pub const SHELL_FEEDBACK_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellInvocation {
    pub tool: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellFeedback {
    pub improvements: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellFeedbackRecord {
    pub schema_version: u32,
    pub id: String,
    pub created_at_unix_ms: u64,
    pub invocation: ShellInvocation,
    pub feedback: ShellFeedback,
}

pub fn build_shell_feedback_prompt(invocation: &ShellInvocation) -> String {
    let exit = invocation
        .exit_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let duration = invocation
        .duration_ms
        .map(|ms| format!("{ms}ms"))
        .unwrap_or_else(|| "unknown".to_string());
    let cwd = invocation.cwd.as_deref().unwrap_or("unknown");

    format!(
        "You just used the `shell_command` tool.\n\n\
Command:\n{command}\n\n\
Context:\n- cwd: {cwd}\n- exit_code: {exit}\n- duration: {duration}\n\n\
Question:\n\
How could the `shell_command` tool be improved to make agents more reliable and productive?\n\
Consider: better error classification, safer defaults, quoting/escaping help, output streaming/truncation, timeouts, structured results, sandbox diagnostics, and redaction of secrets.\n\n\
Respond in this JSON shape:\n\
{{\"improvements\":\"...\",\"rating\":1,\"tags\":[\"...\"]}}\n",
        command = invocation.command
    )
}

pub fn read_feedback_text_from_stdin() -> Result<Option<String>> {
    if io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf)?;
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

pub fn new_shell_feedback_record(
    invocation: ShellInvocation,
    improvements: String,
    rating: Option<u8>,
    tags: Vec<String>,
) -> Result<ShellFeedbackRecord> {
    let now = SystemTime::now();
    let ms = now
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_millis() as u64;

    let id = format!("{ms}-{}", std::process::id());

    Ok(ShellFeedbackRecord {
        schema_version: SHELL_FEEDBACK_SCHEMA_VERSION,
        id,
        created_at_unix_ms: ms,
        invocation,
        feedback: ShellFeedback {
            improvements,
            rating,
            tags,
        },
    })
}

pub fn write_shell_feedback(
    out_dir: impl AsRef<Path>,
    record: &ShellFeedbackRecord,
) -> Result<PathBuf> {
    let out_dir = out_dir.as_ref();
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create feedback directory {}", out_dir.display()))?;

    let file_name = format!("shell_feedback_{}.json", record.id);
    let record_path = out_dir.join(file_name);
    let json = serde_json::to_vec_pretty(record).context("failed to serialize feedback record")?;
    fs::write(&record_path, json).with_context(|| {
        format!(
            "failed to write feedback record to {}",
            record_path.display()
        )
    })?;

    let ndjson_path = out_dir.join("shell_feedback.ndjson");
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&ndjson_path)
    {
        if let Ok(line) = serde_json::to_string(record) {
            let _ = writeln!(file, "{line}");
        }
    }

    Ok(record_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_contains_command() {
        let invocation = ShellInvocation {
            tool: SHELL_TOOL_NAME.to_string(),
            command: "echo hello".to_string(),
            cwd: None,
            exit_code: Some(0),
            success: Some(true),
            duration_ms: Some(10),
        };
        let prompt = build_shell_feedback_prompt(&invocation);
        assert!(prompt.contains("echo hello"));
        assert!(prompt.contains("shell_command"));
    }

    #[test]
    fn writes_json_record() {
        let base =
            std::env::temp_dir().join(format!("swe-grep-feedback-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("create temp dir");

        let invocation = ShellInvocation {
            tool: SHELL_TOOL_NAME.to_string(),
            command: "true".to_string(),
            cwd: Some("/tmp".to_string()),
            exit_code: Some(0),
            success: Some(true),
            duration_ms: Some(1),
        };
        let record = new_shell_feedback_record(
            invocation,
            "more structured stderr".to_string(),
            Some(4),
            vec![],
        )
        .expect("record");
        let path = write_shell_feedback(&base, &record).expect("write");
        assert!(path.exists());
    }
}
