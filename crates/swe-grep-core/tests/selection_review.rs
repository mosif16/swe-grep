use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn context(root: &Path, query: &str, options: &[&str]) -> Value {
    let result = Command::new(env!("CARGO_BIN_EXE_swe-grep"))
        .args(["--no-config", "--disable-telemetry", "context", query])
        .arg(root)
        .args(["--concise", "--quiet-hint"])
        .args(options)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

#[test]
fn same_file_method_selection_does_not_merge_qualified_callers() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("app.rs"),
        "struct A;\nimpl A { fn run() { alpha(); } }\nstruct B;\nimpl B { fn run() { beta(); } }\nfn alpha() {}\nfn beta() {}\n",
    ).unwrap();
    for (line, expected) in [("2", "alpha"), ("4", "beta")] {
        let result = context(
            root.path(),
            "run",
            &[
                "--target-file",
                "app.rs",
                "--target-line",
                line,
                "--callees",
            ],
        );
        assert_eq!(result["resolution"]["status"], "resolved");
        assert_eq!(result["callees"], serde_json::json!([expected]), "{result}");
    }
}

#[test]
fn nested_same_name_function_selection_uses_the_closest_declaration_scope() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("app.py"),
        "def target():\n    def target():\n        inner()\n    outer()\n",
    )
    .unwrap();
    for (line, expected) in [("1", "outer"), ("2", "inner")] {
        let result = context(
            root.path(),
            "target",
            &[
                "--target-file",
                "app.py",
                "--target-line",
                line,
                "--callees",
            ],
        );
        assert_eq!(result["resolution"]["status"], "resolved");
        assert_eq!(result["callees"], serde_json::json!([expected]), "{result}");
    }
}

#[test]
fn related_test_cap_cannot_report_complete_results() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("app.rs"), "pub fn target() {}\n").unwrap();
    std::fs::create_dir(root.path().join("tests")).unwrap();
    for index in 0..55 {
        std::fs::write(
            root.path().join("tests").join(format!("file_{index}.rs")),
            format!("#[test]\nfn test_{index}() {{ target(); }}\n"),
        )
        .unwrap();
    }
    let result = context(root.path(), "target", &["--tests"]);
    let returned = result["tests"].as_array().unwrap().len();
    if returned < 55 {
        assert_eq!(result["analysis"]["tests"], "incomplete", "{result}");
        assert_eq!(result["truncated"], true, "{result}");
    } else {
        assert_eq!(returned, 55);
    }
}

#[test]
fn bounded_transitive_relationships_mark_analysis_incomplete() {
    let root = tempfile::tempdir().unwrap();
    let mut source = String::from("fn target() { step0(); }\n");
    for index in 0..8 {
        source.push_str(&format!("fn step{index}() {{ step{}(); }}\n", index + 1));
    }
    std::fs::write(root.path().join("app.rs"), source).unwrap();
    let result = context(
        root.path(),
        "target",
        &["--callees", "--depth", "10", "--max-lines", "2"],
    );
    assert_eq!(result["analysis"]["callees"], "incomplete", "{result}");
    assert_eq!(result["truncated"], true, "{result}");
}

#[test]
fn missing_selector_does_not_fall_back_to_another_declaration() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("app.rs"),
        "fn target() { alpha(); }\nfn alpha() {}\n",
    )
    .unwrap();
    let result = context(
        root.path(),
        "target",
        &["--target-file", "app.rs", "--target-line", "2", "--callees"],
    );
    assert_eq!(result["resolution"]["status"], "not_found");
    assert!(result.get("def").is_none());
    assert_eq!(result["callees"], serde_json::json!([]));
    assert_eq!(result["analysis"]["callees"], "incomplete");
}
