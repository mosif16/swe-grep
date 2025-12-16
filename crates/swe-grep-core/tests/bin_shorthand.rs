use std::process::Command;

use serde_json::Value;
use tempfile::tempdir;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_swe-grep"))
}

#[test]
fn simple_mode_accepts_scope_file() {
    let temp = tempdir().expect("failed to create tempdir");
    let file_path = temp.path().join("A.rs");
    std::fs::write(&file_path, "pub fn Foo() {}\n").expect("failed to write A.rs");

    let output = bin()
        .current_dir(temp.path())
        .arg("Foo")
        .arg("A.rs")
        .output()
        .expect("failed to run swe-grep");

    assert!(output.status.success(), "expected success");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("A.rs:1 |"),
        "expected output to include A.rs hit, got: {stdout}"
    );
}

#[test]
fn refs_callees_filters_rust_constructors_by_default() {
    let temp = tempdir().expect("failed to create tempdir");
    let file_path = temp.path().join("lib.rs");
    std::fs::write(
        &file_path,
        r#"
pub fn foo() {
    bar();
    let _x = Ok::<(), ()>(());
    let _y = Some(1);
}

pub fn bar() {}
"#,
    )
    .expect("failed to write lib.rs");

    let output = bin()
        .current_dir(temp.path())
        .args(["refs", "--language", "rust", "foo", ".", "--callees"])
        .output()
        .expect("failed to run swe-grep refs");

    assert!(output.status.success(), "expected success");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let payload: Value = serde_json::from_str(&stdout).expect("expected JSON output");

    let callees = payload
        .get("callees")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut saw_bar = false;
    for entry in callees {
        let symbol = entry
            .get("symbol")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if symbol == "bar" {
            saw_bar = true;
        }
        assert_ne!(symbol, "Ok", "expected Ok to be filtered out");
        assert_ne!(symbol, "Some", "expected Some to be filtered out");
    }

    assert!(saw_bar, "expected to see bar() as a callee");
}
