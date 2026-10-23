use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::tempdir;

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_swe-grep"))
        .current_dir(root)
        .env("XDG_CACHE_HOME", root.join(".cache"))
        .arg("--disable-telemetry")
        .args(args)
        .output()
        .expect("run swe-grep")
}

fn json(root: &Path, args: &[&str]) -> Value {
    let output = run(root, args);
    assert!(
        output.status.success(),
        "command {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

#[test]
fn context_reports_unrelated_invalid_utf8_before_emitting_any_output() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("lib.rs"), "pub fn target() {}\n").unwrap();
    fs::write(temp.path().join("broken.rs"), [0xff, 0xfe]).unwrap();

    for mode in [None, Some("--concise"), Some("--grouped"), Some("--stream")] {
        let mut args = vec!["context", "target", "."];
        if let Some(mode) = mode {
            args.push(mode);
        }
        let output = run(temp.path(), &args);
        assert!(!output.status.success(), "mode {mode:?} silently succeeded");
        assert!(output.stdout.is_empty(), "must not emit a success payload");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("failed to extract repository facts"),
            "{stderr}"
        );
        assert!(
            stderr.contains("broken.rs"),
            "missing failing path: {stderr}"
        );
    }
}

#[test]
fn context_cache_io_failure_is_not_reported_as_absence() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("lib.rs"), "pub fn target() {}\n").unwrap();
    fs::write(temp.path().join(".cache"), "not a directory").unwrap();
    let output = run(temp.path(), &["context", "target", ".", "--concise"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to extract repository facts"));
}

#[test]
fn context_genuine_absence_remains_successful() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("lib.rs"), "pub fn other() {}\n").unwrap();
    let payload = json(temp.path(), &["context", "missing", ".", "--concise"]);
    assert_eq!(payload["symbol"], "missing");
    assert!(payload["def"].is_null());
    assert_eq!(payload["callers"], serde_json::json!([]));
}

fn caller_fixture(root: &Path) {
    fs::write(root.join("lib.rs"), "pub fn target() {}\n").unwrap();
    fs::write(
        root.join("allowed.rs"),
        "pub fn allowed() {\n    target();\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("excluded.rs"),
        "pub fn excluded() {\n    target();\n}\n",
    )
    .unwrap();
}

fn assert_filtered_callers(payload: &Value) {
    let callers = payload["callers"].as_array().unwrap();
    assert!(
        !callers.is_empty(),
        "filtered semantic graph lost allowed callers: {payload}"
    );
    for caller in callers {
        assert_eq!(
            caller["file"], "allowed.rs",
            "leaked filtered caller: {payload}"
        );
        assert_eq!(caller["origin"], "callgraph");
    }
    assert!(
        !payload["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| { note.as_str().unwrap().contains("graph enrichment disabled") })
    );
}

#[test]
fn refs_exclude_cannot_leak_warm_graph_callers() {
    let temp = tempdir().unwrap();
    caller_fixture(temp.path());
    let warm = json(temp.path(), &["refs", "target", ".", "--callers"]);
    assert!(
        warm["callers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| { hit["file"] == "excluded.rs" && hit["origin"] == "callgraph" }),
        "fixture must populate the unfiltered graph: {warm}"
    );
    let payload = json(
        temp.path(),
        &[
            "refs",
            "target",
            ".",
            "--callers",
            "--exclude",
            "excluded.rs",
        ],
    );
    assert_filtered_callers(&payload);
}

#[test]
fn refs_scope_file_and_path_glob_cannot_leak_graph_callers() {
    let temp = tempdir().unwrap();
    caller_fixture(temp.path());
    for flag in ["--scope-file", "--path-glob"] {
        let payload = json(
            temp.path(),
            &[
                "refs",
                "target",
                "--path",
                ".",
                "--callers",
                flag,
                "allowed.rs",
            ],
        );
        assert_filtered_callers(&payload);
    }
}

#[test]
fn refs_exclude_tests_cannot_leak_graph_callers() {
    let temp = tempdir().unwrap();
    fs::write(
        temp.path().join("allowed.rs"),
        "pub fn allowed() {\n    target();\n}\n",
    )
    .unwrap();
    fs::create_dir(temp.path().join("tests")).unwrap();
    fs::write(
        temp.path().join("tests/test.rs"),
        "pub fn test_call() {\n    target();\n}\n",
    )
    .unwrap();
    let payload = json(
        temp.path(),
        &["refs", "target", ".", "--callers", "--exclude-tests"],
    );
    assert_filtered_callers(&payload);
}

#[test]
fn refs_filters_retain_allowed_callee_and_import_graph_data() {
    let temp = tempdir().unwrap();
    fs::write(
        temp.path().join("allowed.ts"),
        "import { helper } from './helper';\nexport function target() { helper(); }\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("excluded.ts"),
        "import { target } from './allowed';\nexport function target() { secret(); }\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("helper.ts"),
        "export function helper() {}\n",
    )
    .unwrap();
    let payload = json(
        temp.path(),
        &[
            "refs",
            "target",
            ".",
            "--callees",
            "--imports",
            "--importers",
            "--exclude",
            "excluded.ts",
        ],
    );
    let callees = payload["callees"].as_array().unwrap();
    assert_eq!(callees.len(), 1, "{payload}");
    assert_eq!(callees[0]["symbol"], "helper", "{payload}");
    assert_eq!(callees[0]["file"], "allowed.ts", "{payload}");
    assert_eq!(payload["imports"][0]["file"], "helper.ts", "{payload}");
    assert!(payload.get("importers").is_none(), "{payload}");
}

#[test]
fn refs_tracked_only_cannot_leak_untracked_graph_callers() {
    let temp = tempdir().unwrap();
    caller_fixture(temp.path());
    for args in [
        ["init", "--quiet"].as_slice(),
        ["add", "allowed.rs"].as_slice(),
    ] {
        let output = Command::new("git")
            .current_dir(temp.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let payload = json(
        temp.path(),
        &["refs", "target", ".", "--callers", "--tracked-only"],
    );
    assert_filtered_callers(&payload);
}

#[test]
fn refs_does_not_treat_a_sole_definition_as_a_caller() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("lib.rs"), "pub fn target() {}\n").unwrap();
    let payload = json(temp.path(), &["refs", "target", ".", "--callers"]);
    assert_eq!(payload["callers"], serde_json::json!([]), "{payload}");
}
