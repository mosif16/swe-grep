use std::process::Command;

#[test]
fn harness_calls_disable_ambient_configuration_and_keep_stdout_json_only() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("app.rs"),
        "pub fn target() {}\nfn caller() { target(); }\n",
    )
    .unwrap();
    for args in [
        vec![
            "search",
            "--symbol",
            "target",
            "--json-only",
            "--concise",
            "--disable-fd",
            "--disable-ast-grep",
        ],
        vec!["context", "--symbol", "target", "--concise"],
        vec!["refs", "--symbol", "target", "--callers"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_swe-grep"))
            .args(args)
            .args([
                "--no-config",
                "--disable-telemetry",
                "--quiet-hint",
                "--path",
            ])
            .arg(root.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(result.is_object());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("swe-grep-system"));
    }
}
