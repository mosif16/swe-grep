use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use tempfile::tempdir;

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo(root: &Path) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["config", "user.name", "Pipeline Test"]);
}

fn commit(root: &Path) {
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "fixture"]);
}

fn search(root: &Path, flags: &[&str], tools: Option<&Path>) -> Value {
    let cache = tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_swe-grep"));
    cmd.args([
        "search",
        "Needle",
        "--json-only",
        "-q",
        "--disable-fd",
        "--path",
    ])
    .arg(root)
    .arg("--cache-dir")
    .arg(cache.path())
    .args(flags);
    if let Some(tools) = tools {
        let mut paths = vec![tools.to_path_buf()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        cmd.env("PATH", std::env::join_paths(paths).unwrap());
    }
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "search failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn files(result: &Value) -> Vec<&str> {
    let mut paths: Vec<_> = result["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["file"].as_str().unwrap())
        .collect();
    paths.sort_unstable();
    paths.dedup();
    paths
}

#[test]
fn clean_tracked_modified_and_since_scopes_stay_empty() {
    let repo = tempdir().unwrap();
    init_repo(repo.path());
    fs::write(repo.path().join("lib.rs"), "pub fn Needle() {}\n").unwrap();
    commit(repo.path());
    for flags in [
        vec!["--tracked-only", "--git-modified", "--disable-ast-grep"],
        vec![
            "--tracked-only",
            "--git-since",
            "HEAD",
            "--disable-ast-grep",
        ],
    ] {
        assert!(files(&search(repo.path(), &flags, None)).is_empty());
    }
}

#[cfg(unix)]
#[test]
fn git_scopes_preserve_whitespace_quotes_and_rename_destinations() {
    let repo = tempdir().unwrap();
    init_repo(repo.path());
    let names = [
        " leading space.rs",
        "quote\"name.rs",
        "tab\tname.rs",
        "line\nbreak.rs",
    ];
    for name in names {
        fs::write(repo.path().join(name), "pub fn Needle() {}\n").unwrap();
    }
    fs::write(repo.path().join("old.rs"), "pub fn Needle() {}\n").unwrap();
    commit(repo.path());
    git(repo.path(), &["mv", "old.rs", "renamed file.rs"]);
    for name in names {
        fs::write(
            repo.path().join(name),
            "pub fn Needle() { /* changed */ }\n",
        )
        .unwrap();
    }
    let mut expected = names.to_vec();
    expected.push("renamed file.rs");
    expected.sort_unstable();
    for flags in [
        vec!["--tracked-only", "--disable-ast-grep"],
        vec!["--git-modified", "--tracked-only", "--disable-ast-grep"],
        vec![
            "--git-since",
            "HEAD",
            "--tracked-only",
            "--disable-ast-grep",
        ],
    ] {
        assert_eq!(files(&search(repo.path(), &flags, None)), expected);
    }
}

#[test]
fn git_subdirectory_scope_resolves_repository_paths_and_untracked_files() {
    let repo = tempdir().unwrap();
    init_repo(repo.path());
    let sub = repo.path().join("nested");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("lib.rs"), "pub fn Needle() {}\n").unwrap();
    fs::write(repo.path().join("outside.rs"), "pub fn Needle() {}\n").unwrap();
    commit(repo.path());
    fs::write(sub.join("lib.rs"), "pub fn Needle() { /* modified */ }\n").unwrap();
    fs::create_dir(sub.join("new dir")).unwrap();
    fs::write(sub.join("new dir/new.rs"), "pub fn Needle() {}\n").unwrap();
    for flags in [
        vec!["--git-modified", "--tracked-only", "--disable-ast-grep"],
        vec![
            "--git-since",
            "HEAD",
            "--tracked-only",
            "--disable-ast-grep",
        ],
        vec!["--tracked-only", "--disable-ast-grep"],
    ] {
        assert_eq!(files(&search(&sub, &flags, None)), vec!["lib.rs"]);
    }
    assert_eq!(
        files(&search(
            &sub,
            &["--git-modified", "--disable-ast-grep"],
            None
        )),
        vec!["lib.rs", "new dir/new.rs"]
    );
}

#[cfg(unix)]
fn fake_ast(tools: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = tools.join("ast-grep");
    fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn ast_hits_are_verified_with_source_lines_bodies_and_filters() {
    let repo = tempdir().unwrap();
    let tools = tempdir().unwrap();
    init_repo(repo.path());
    fs::write(
        repo.path().join("keep.rs"),
        "// header\npub fn Needle() {}\n",
    )
    .unwrap();
    fs::write(repo.path().join("skip.rs"), "pub fn Needle() {}\n").unwrap();
    fs::write(repo.path().join("skip.txt"), "Needle\n").unwrap();
    commit(repo.path());
    let matches = serde_json::json!([
        {"file":"keep.rs","range":{"start":{"line":1,"column":0},"end":{"line":1,"column":18}}},
        {"file":"skip.rs","range":{"start":{"line":0,"column":0},"end":{"line":0,"column":18}}},
        {"file":"skip.txt","range":{"start":{"line":0,"column":0},"end":{"line":0,"column":6}}}
    ]);
    fake_ast(tools.path(), &format!("printf '%s\\n' '{}'", matches));
    let result = search(
        repo.path(),
        &[
            "--mode",
            "ast",
            "--language",
            "rust",
            "--scope-file",
            "keep.rs",
            "--path-glob",
            "*.rs",
            "--exclude",
            "skip.*",
            "--body",
            "--body-limit",
            "12",
        ],
        Some(tools.path()),
    );
    assert_eq!(files(&result), vec!["keep.rs"]);
    assert_eq!(result["hits"][0]["line"], 2);
    assert!(
        result["hits"][0]["snippet"]
            .as_str()
            .unwrap()
            .contains("pub fn Needle()")
    );
    assert_eq!(result["hits"][0]["body"], "// header\npu");
    assert!(
        files(&search(
            repo.path(),
            &[
                "--mode",
                "ast",
                "--language",
                "rust",
                "--git-modified",
                "--tracked-only"
            ],
            Some(tools.path())
        ))
        .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn ast_failure_reports_no_fallback_and_does_not_return_literal_hits() {
    let repo = tempdir().unwrap();
    let tools = tempdir().unwrap();
    let logs = tempdir().unwrap();
    fs::write(repo.path().join("lib.rs"), "pub fn Needle() {}\n").unwrap();
    fake_ast(
        tools.path(),
        "echo 'Pattern contains an ERROR node' >&2; exit 2",
    );
    let result = search(
        repo.path(),
        &[
            "--mode",
            "ast",
            "--language",
            "rust",
            "--verbose-ast",
            "--log-dir",
            logs.path().to_str().unwrap(),
        ],
        Some(tools.path()),
    );
    assert!(files(&result).is_empty());
    let log: Value = serde_json::from_str(
        fs::read_to_string(logs.path().join("search.log.jsonl"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    // False diagnostic flags are omitted from serialized summaries.
    assert!(!log["summary"]["ast_fallback"].as_bool().unwrap_or(false));
    assert!(
        log["summary"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("no fallback applied"))
    );
}

#[cfg(unix)]
#[test]
fn rga_fallback_cannot_escape_file_scope_or_repository_root() {
    use std::os::unix::fs::PermissionsExt;
    let repo = tempdir().unwrap();
    let tools = tempdir().unwrap();
    fs::write(repo.path().join("allowed.rs"), "// no matches\n").unwrap();
    fs::write(repo.path().join("secret.rs"), "pub fn Needle() {}\n").unwrap();
    let outside = tempdir().unwrap();
    let secret = outside.path().join("outside.rs");
    fs::write(&secret, "pub fn Needle() {}\n").unwrap();
    let marker = tools.path().join("invoked");
    let rga = tools.path().join("rga");
    let messages = ["secret.rs".to_owned(), secret.to_string_lossy().into_owned()]
        .map(|file| serde_json::json!({"type":"match", "data":{"path":{"text":file},"lines":{"text":"pub fn Needle() {}"},"line_number":1}}).to_string())
        .join("\n");
    fs::write(
        &rga,
        format!(
            "#!/bin/sh\nprintf called > '{}'\nprintf '%s\\n' '{}'\n",
            marker.display(),
            messages
        ),
    )
    .unwrap();
    fs::set_permissions(&rga, fs::Permissions::from_mode(0o755)).unwrap();
    let result = search(
        &repo.path().join("allowed.rs"),
        &["--text", "--enable-rga", "--disable-ast-grep"],
        Some(tools.path()),
    );
    assert!(marker.exists(), "fixture must exercise the RGA fallback");
    assert!(files(&result).is_empty(), "{result}");
    let result = search(
        repo.path(),
        &[
            "--text",
            "--enable-rga",
            "--disable-ast-grep",
            "--exclude",
            "secret.rs",
        ],
        Some(tools.path()),
    );
    assert!(files(&result).is_empty(), "{result}");
}

#[test]
fn cached_fuzzy_suggestions_obey_file_scope() {
    let repo = tempdir().unwrap();
    let cache = tempdir().unwrap();
    fs::write(repo.path().join("allowed.rs"), "// ordinary content\n").unwrap();
    fs::write(repo.path().join("secret.rs"), "pub fn SecretNeedle() {}\n").unwrap();
    fs::write(
        cache.path().join("state.json"),
        serde_json::json!({
            "symbol_hits":{"SecretNeedle":["secret.rs"]}, "directory_scores":{}
        })
        .to_string(),
    )
    .unwrap();
    for (path, expect_suggestion) in [
        (repo.path().to_path_buf(), true),
        (repo.path().join("allowed.rs"), false),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_swe-grep"))
            .args([
                "search",
                "SecretNeedl",
                "--json-only",
                "--disable-fd",
                "--disable-ast-grep",
                "--disable-telemetry",
                "--path",
            ])
            .arg(path)
            .arg("--cache-dir")
            .arg(cache.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(text.contains("SecretNeedle"), expect_suggestion, "{text}");
    }
}
