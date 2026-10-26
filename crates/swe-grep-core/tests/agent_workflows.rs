use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::tempdir;

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_swe-grep"))
        .current_dir(root)
        .env("XDG_CACHE_HOME", root.join(".cache"))
        .args(["--no-config", "--disable-telemetry", "--quiet-hint"])
        .args(args)
        .output()
        .unwrap()
}
fn payload(root: &Path, args: &[&str]) -> Value {
    let out = run(root, args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn exact_identifiers_are_filtered_before_collection_limits() {
    let dir = tempdir().unwrap();
    let mut source = (0..40)
        .map(|i| format!("fn login_user_{i}() {{}}\n"))
        .collect::<String>();
    source.push_str("fn login_user() {}\nfn usage() { login_user(); }\n");
    fs::write(dir.path().join("main.rs"), source).unwrap();
    let out = payload(
        dir.path(),
        &[
            "search",
            "login_user",
            ".",
            "--exact-symbol",
            "--json-only",
            "--concise",
            "--disable-fd",
            "--disable-ast-grep",
        ],
    );
    let hits = out["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 2, "{out}");
    assert!(hits.iter().all(|hit| hit["line"].as_u64().unwrap() >= 41));
    assert!(
        !run(
            dir.path(),
            &["search", "login_user", ".", "--exact-symbol", "--text"]
        )
        .status
        .success()
    );
}

#[test]
fn context_distinguishes_not_requested_missing_and_analyzed_relationships() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "def helper():\n    pass\ndef target():\n    helper()\ndef caller():\n    target()\n",
    )
    .unwrap();
    let basic = payload(dir.path(), &["context", "target", ".", "--concise"]);
    assert_eq!(basic["resolution"]["status"], "resolved");
    assert_eq!(basic["analysis"]["callers"], "not_requested");
    assert_eq!(basic["analysis"]["callees"], "not_requested");
    assert_eq!(basic["callers"], json!([]));
    let rich = payload(
        dir.path(),
        &[
            "context",
            "target",
            ".",
            "--concise",
            "--callers",
            "--callees",
        ],
    );
    assert_eq!(rich["analysis"]["callers"], "complete");
    assert_eq!(rich["analysis"]["callees"], "complete");
    assert!(!rich["callers"].as_array().unwrap().is_empty());
    assert!(
        rich["callees"]
            .as_array()
            .unwrap()
            .contains(&json!("helper"))
    );
    let missing = payload(
        dir.path(),
        &["context", "missing", ".", "--concise", "--callers"],
    );
    assert_eq!(missing["resolution"]["status"], "not_found");
    assert_eq!(missing["analysis"]["callers"], "incomplete");
}

#[test]
fn search_hit_selects_duplicate_definition_without_merging_callees() {
    let dir = tempdir().unwrap();
    for suffix in ["a", "b"] {
        fs::write(dir.path().join(format!("{suffix}.py")), format!("def helper_{suffix}():\n    pass\ndef target():\n    helper_{suffix}()\ndef caller_{suffix}():\n    target()\n")).unwrap();
    }
    let ambiguous = payload(dir.path(), &["context", "target", ".", "--concise"]);
    assert_eq!(ambiguous["resolution"]["status"], "ambiguous");
    assert_eq!(
        ambiguous["resolution"]["candidates"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(ambiguous.get("def").is_none());
    let selected = payload(
        dir.path(),
        &[
            "context",
            "target",
            ".",
            "--target-file=b.py",
            "--target-line=3",
            "--concise",
            "--callers",
            "--callees",
        ],
    );
    assert_eq!(selected["def_location"], "b.py:3");
    assert_eq!(selected["callees"], json!(["helper_b"]));
    assert_eq!(selected["analysis"]["callers"], "incomplete");
    let refs = payload(
        dir.path(),
        &[
            "refs",
            "target",
            ".",
            "--target-file=b.py",
            "--target-line=3",
            "--callees",
        ],
    );
    assert_eq!(refs["analysis"]["callers"], "not_requested");
    assert_eq!(refs["callees"].as_array().unwrap().len(), 1);
    assert_eq!(refs["callees"][0]["symbol"], "helper_b");
    assert_eq!(refs["callees"][0]["file"], "b.py");
    let invalid_line = payload(
        dir.path(),
        &[
            "context",
            "target",
            ".",
            "--target-file=b.py",
            "--target-line=4",
            "--concise",
        ],
    );
    assert_eq!(invalid_line["resolution"]["status"], "not_found");
}

#[test]
fn selectors_preserve_scope_and_validate_lines() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("main.py"), "def target():\n    pass\n").unwrap();
    assert!(
        !run(dir.path(), &["context", "target", ".", "--target-line=1"])
            .status
            .success()
    );
    assert!(
        !run(
            dir.path(),
            &[
                "context",
                "target",
                ".",
                "--target-file=main.py",
                "--target-line=0"
            ]
        )
        .status
        .success()
    );
    assert!(
        !run(
            dir.path(),
            &["context", "target", ".", "--target-file=../main.py"]
        )
        .status
        .success()
    );
    assert!(
        !run(
            dir.path(),
            &[
                "refs",
                "target",
                ".",
                "--target-file=main.py",
                "--exclude=main.py"
            ]
        )
        .status
        .success()
    );
    #[cfg(unix)]
    {
        let outside = tempdir().unwrap();
        fs::write(
            outside.path().join("outside.py"),
            "def target():\n    pass\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("outside.py"),
            dir.path().join("linked.py"),
        )
        .unwrap();
        assert!(
            !run(
                dir.path(),
                &["context", "target", ".", "--target-file=linked.py"]
            )
            .status
            .success()
        );
    }
}

#[test]
fn language_and_truncation_status_are_preserved() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("main.rs"),
        "fn target() {}\nfn a() { target(); }\nfn b() { target(); }\nfn c() { target(); }\n",
    )
    .unwrap();
    fs::write(dir.path().join("main.py"), "def target():\n    pass\n").unwrap();
    let out = payload(
        dir.path(),
        &[
            "context",
            "target",
            ".",
            "--language=rust",
            "--callers",
            "--max-lines=3",
            "--concise",
        ],
    );
    assert_eq!(out["def_location"], "main.rs:1");
    assert_eq!(out["truncated"], true);
    assert_eq!(out["analysis"]["callers"], "incomplete");
}

#[test]
fn context_preserves_member_call_expressions_for_followup_navigation() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("main.js"),
        "export function target() { return Object.entries({}); }\n",
    )
    .unwrap();
    let result = payload(
        dir.path(),
        &["context", "target", ".", "--callees", "--concise"],
    );
    assert!(
        result["callees"]
            .as_array()
            .unwrap()
            .contains(&json!("Object.entries")),
        "{result}"
    );
    let refs = payload(dir.path(), &["refs", "target", ".", "--callees"]);
    assert!(
        refs["callees"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["symbol"] == "Object.entries")
    );
}
