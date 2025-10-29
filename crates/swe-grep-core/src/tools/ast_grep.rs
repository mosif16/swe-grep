use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

#[derive(Clone, Debug)]
pub struct AstGrepTool {
    timeout: Duration,
    max_matches: usize,
}

impl AstGrepTool {
    pub fn new(timeout: Duration, max_matches: usize) -> Self {
        Self {
            timeout,
            max_matches,
        }
    }

    pub async fn search_identifier(
        &self,
        root: &Path,
        symbol: &str,
        languages: &[String],
        paths: &[PathBuf],
    ) -> Result<Vec<AstGrepMatch>> {
        let mut hints: Vec<String> = if languages.is_empty() {
            vec!["rust".to_string()]
        } else {
            languages.iter().cloned().collect()
        };
        if hints.is_empty() {
            hints.push("rust".to_string());
        }
        let mut aggregated: Vec<AstGrepMatch> = Vec::new();
        let mut seen: HashSet<(PathBuf, usize)> = HashSet::new();

        for lang in hints {
            let patterns = patterns_for_language(symbol, &lang);
            for pattern in patterns {
                if aggregated.len() >= self.max_matches {
                    break;
                }
                let remaining = self.max_matches.saturating_sub(aggregated.len());
                let matches = self
                    .run_pattern(root, &lang, &pattern, paths, remaining)
                    .await?;

                for m in matches {
                    let key = (m.path.clone(), m.line);
                    if seen.insert(key) {
                        aggregated.push(m);
                        if aggregated.len() >= self.max_matches {
                            break;
                        }
                    }
                }
            }
        }

        Ok(aggregated)
    }

    async fn run_pattern(
        &self,
        root: &Path,
        lang: &str,
        pattern: &str,
        paths: &[PathBuf],
        limit: usize,
    ) -> Result<Vec<AstGrepMatch>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut cmd = Command::new("ast-grep");
        cmd.arg("--json")
            .arg("--pattern")
            .arg(pattern)
            .arg("--lang")
            .arg(lang);

        if paths.is_empty() {
            cmd.arg(".");
        } else {
            for path in paths {
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

        let mut child = cmd
            .spawn()
            .with_context(|| "failed to spawn ast-grep; is it installed and on PATH?")?;

        let stdout = child
            .stdout
            .take()
            .context("ast-grep did not produce stdout pipe")?;
        let mut reader = BufReader::new(stdout);
        let mut buffer = Vec::new();

        let collect = async {
            reader.read_to_end(&mut buffer).await?;
            let status = child.wait().await?;
            if !status.success() {
                if status.code() != Some(1) {
                    anyhow::bail!("ast-grep exited with status {}", status);
                }
            }

            let text = String::from_utf8_lossy(&buffer);
            let mut matches = Vec::new();

            if text.trim().is_empty() {
                return Ok(matches);
            }

            if let Ok(parsed) = serde_json::from_str::<Vec<AstGrepMessage>>(&text) {
                for msg in parsed.into_iter().take(limit) {
                    matches.push(msg.into());
                }
                return Ok(matches);
            }

            for line in text.lines() {
                match serde_json::from_str::<AstGrepMessage>(line) {
                    Ok(msg) => {
                        if matches.len() >= limit {
                            break;
                        }
                        matches.push(msg.into());
                    }
                    Err(err) => {
                        eprintln!("warn: failed to parse ast-grep json line: {err}");
                    }
                }
            }

            Ok(matches)
        };

        timeout(self.timeout, collect)
            .await
            .with_context(|| "ast-grep invocation timed out")?
    }
}

#[derive(Debug, Deserialize)]
struct AstGrepMessage {
    path: String,
    range: AstGrepRange,
}

#[derive(Debug, Deserialize)]
struct AstGrepRange {
    start: AstGrepPosition,
    #[allow(dead_code)]
    end: AstGrepPosition,
}

#[derive(Debug, Deserialize)]
struct AstGrepPosition {
    line: usize,
    #[allow(dead_code)]
    column: usize,
}

#[derive(Clone, Debug)]
pub struct AstGrepMatch {
    pub path: PathBuf,
    pub line: usize,
}

impl From<AstGrepMessage> for AstGrepMatch {
    fn from(value: AstGrepMessage) -> Self {
        Self {
            path: PathBuf::from(value.path),
            line: value.range.start.line,
        }
    }
}

fn patterns_for_language(symbol: &str, language: &str) -> Vec<String> {
    let needle = symbol.trim();
    if needle.is_empty() {
        return vec![String::from("(identifier) @id")];
    }

    match language.to_ascii_lowercase().as_str() {
        "swift" => vec![
            format!("(function_declaration name: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!("(protocol_declaration name: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!(
                "(protocol_member_declaration (function_declaration name: (identifier) @id) (#eq? @id \"{needle}\"))"
            ),
            format!("(initializer_declaration name: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!(
                "(class_declaration body: (member_declaration_list (member_declaration (function_declaration name: (identifier) @id (#eq? @id \"{needle}\")))))"
            ),
            format!(
                "(struct_declaration body: (member_declaration_list (member_declaration (function_declaration name: (identifier) @id (#eq? @id \"{needle}\")))))"
            ),
            format!(
                "(extension_declaration body: (member_declaration_list (member_declaration (function_declaration name: (identifier) @id (#eq? @id \"{needle}\")))))"
            ),
            format!(
                "(actor_declaration body: (member_declaration_list (member_declaration (function_declaration name: (identifier) @id (#eq? @id \"{needle}\")))))"
            ),
            format!("(member_access_expression name: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!(
                "(function_call_expression function: (identifier) @id (#eq? @id \"{needle}\"))"
            ),
            format!(
                "(await_expression (function_call_expression function: (identifier) @id (#eq? @id \"{needle}\")))"
            ),
            format!("(attribute attribute_name: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!(
                "(extension_declaration protocol_conformance: (identifier) @id (#eq? @id \"{needle}\"))"
            ),
            format!("(generic_argument_clause (identifier) @id (#eq? @id \"{needle}\"))"),
        ],
        "typescript" | "ts" | "tsx" => vec![
            format!("(identifier) @id (#eq? @id \"{needle}\")"),
            format!("(call_expression function: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!(
                "(call_expression function: (member_expression property: (property_identifier) @id (#eq? @id \"{needle}\")))"
            ),
            format!("(class_declaration name: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!("(interface_declaration name: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!("(interface_declaration name: (type_identifier) @id (#eq? @id \"{needle}\"))"),
            format!("(type_alias_declaration name: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!("(type_alias_declaration name: (type_identifier) @id (#eq? @id \"{needle}\"))"),
            format!("(method_definition name: (property_identifier) @id (#eq? @id \"{needle}\"))"),
            format!(
                "(lexical_declaration (variable_declarator name: (identifier) @id (#eq? @id \"{needle}\"))))"
            ),
            format!("(jsx_opening_element name: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!(
                "(lexical_declaration (variable_declarator name: (identifier) @id (#eq? @id \"{needle}\") value: (arrow_function)))"
            ),
            format!("(export_statement value: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!(
                "(export_statement (export_clause (export_specifier name: (identifier) @id (#eq? @id \"{needle}\"))))"
            ),
            format!("(jsx_attribute name: (property_identifier) @id (#eq? @id \"{needle}\"))"),
            format!(
                "(binary_expression left: (identifier) @id (#eq? @id \"{needle}\") operator: \"satisfies\")"
            ),
        ],
        "rust" => vec![
            format!("(function_item name: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!(
                "(impl_item type: (type_path (path_segment name: (identifier) @id (#eq? @id \"{needle}\"))))"
            ),
            format!("(trait_item name: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!("(struct_item name: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!("(enum_item name: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!("(macro_invocation macro: (identifier) @id (#eq? @id \"{needle}\"))"),
            format!(
                "(impl_item trait: (trait_ref path: (scoped_identifier path: (identifier) @id (#eq? @id \"{needle}\"))))"
            ),
            format!(
                "(impl_item trait: (trait_ref path: (type_identifier) @id (#eq? @id \"{needle}\")))"
            ),
        ],
        _ => vec![format!("(identifier) @id (#eq? @id \"{needle}\")")],
    }
}
