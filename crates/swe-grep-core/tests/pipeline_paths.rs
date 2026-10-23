use std::path::Path;
use std::process::{Command, Output};
use tempfile::tempdir;

fn search(root: &Path, path: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_swe-grep"))
        .current_dir(root)
        .args(["search", "Needle", "--path"])
        .arg(path)
        .args([
            "--disable-fd",
            "--disable-ast-grep",
            "--disable-telemetry",
            "--json-only",
            "--cache-dir",
        ])
        .arg(root.join("cache"))
        .args(extra)
        .output()
        .expect("run search")
}

#[test]
fn nested_relative_file_matches_absolute_file() {
    let temp = tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    let path = temp.path().join("src/lib.rs");
    std::fs::write(&path, "pub fn Needle() {}\n").unwrap();
    std::fs::write(temp.path().join("src/other.rs"), "pub fn Needle() {}\n").unwrap();
    for input in [Path::new("src/lib.rs"), path.as_path()] {
        let output = search(temp.path(), input, &[]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let hits = result["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1, "{result}");
        assert_eq!(hits[0]["file"], "lib.rs");
        assert_eq!(hits[0]["line"], 1);
    }
}

#[test]
fn existing_file_with_glob_characters_is_not_a_glob() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("lib[1].rs");
    std::fs::write(&path, "pub fn Needle() {}\n").unwrap();
    let output = search(temp.path(), &path, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["hits"][0]["file"], "lib[1].rs");
}

#[test]
fn invalid_explicit_scope_is_an_error_not_an_empty_success() {
    let temp = tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    for scope in ["missing.rs", "src"] {
        let output = search(temp.path(), temp.path(), &["--scope-file", scope]);
        assert!(!output.status.success(), "invalid scope {scope} must fail");
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            result["errors"].as_array().unwrap().iter().any(|error| {
                error
                    .as_str()
                    .is_some_and(|message| message.contains("scope file"))
            }),
            "{result}"
        );
    }
}

#[test]
fn explicit_scope_remains_relative_to_root() {
    let temp = tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/lib.rs"), "pub fn Needle() {}\n").unwrap();
    std::fs::write(temp.path().join("other.rs"), "pub fn Needle() {}\n").unwrap();
    let output = search(temp.path(), temp.path(), &["--scope-file", "src/lib.rs"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let hits = result["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["file"], "src/lib.rs");
}

#[test]
fn outside_scope_file_is_rejected() {
    let root = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let file = outside.path().join("secret.rs");
    std::fs::write(&file, "pub fn Needle() {}\n").unwrap();
    let output = search(
        root.path(),
        root.path(),
        &["--scope-file", file.to_str().unwrap()],
    );
    assert!(!output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        value["errors"][0]
            .as_str()
            .unwrap()
            .contains("outside search root")
    );
    assert!(value["hits"].as_array().unwrap().is_empty());
}

#[test]
fn malformed_regex_is_a_runtime_error_not_an_empty_match() {
    let root = tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_swe-grep"))
        .args(["search", "[", "--regex", "--json-only", "--path"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        value["errors"][0]
            .as_str()
            .unwrap()
            .contains("invalid regular expression")
    );
}
