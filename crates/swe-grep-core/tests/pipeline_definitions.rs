use std::process::Command;
use tempfile::tempdir;

#[test]
fn definitions_and_calls_are_distinct_across_languages() {
    for (extension, source) in [
        (
            "rs",
            "pub fn target() {}\nfn caller() {\n    target();\n}\nfn target_extra() {}\n",
        ),
        (
            "swift",
            "func target() {}\nfunc caller() {\n    target()\n}\nfunc target_extra() {}\n",
        ),
        (
            "py",
            "def target(): pass\ndef caller():\n    target()\n\ndef target_extra(): pass\n",
        ),
        (
            "go",
            "func target() {}\nfunc caller() {\n    target()\n}\nfunc target_extra() {}\n",
        ),
    ] {
        let root = tempdir().unwrap();
        std::fs::write(root.path().join(format!("app.{extension}")), source).unwrap();
        for (flag, expected) in [
            ("--definitions-only", vec![1, 5]),
            ("--usages-only", vec![3]),
        ] {
            let cache = tempdir().unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_swe-grep"))
                .args([
                    "search",
                    "target",
                    "--json-only",
                    "--disable-fd",
                    "--disable-ast-grep",
                    "--disable-telemetry",
                    "--path",
                ])
                .arg(root.path())
                .arg("--cache-dir")
                .arg(cache.path())
                .arg(flag)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            let mut lines: Vec<_> = value["hits"]
                .as_array()
                .unwrap()
                .iter()
                .map(|hit| hit["line"].as_u64().unwrap())
                .collect();
            lines.sort_unstable();
            assert_eq!(lines, expected, "{extension} {flag}: {value}");
        }
    }
}

#[test]
fn python_context_body_includes_whole_function() {
    let root = tempdir().unwrap();
    std::fs::write(
        root.path().join("app.py"),
        "def hello():\n    print('body one')\n    print('body two')\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_swe-grep"))
        .current_dir(root.path())
        .args(["context", "hello", ".", "--body", "--disable-telemetry"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let body = value["definition"]["body"]
        .as_str()
        .expect("context body text");
    assert!(
        body.contains("body one") && body.contains("body two"),
        "{value}"
    );
}

#[test]
fn rust_modifiers_attributes_and_js_generators_remain_definitions() {
    for (extension, source, symbol) in [
        (
            "rs",
            "pub const fn target() -> usize { 1 }\n#[inline] pub fn target() {}\npub fn target() {}\n",
            "target",
        ),
        ("js", "export function* target() { yield 1; }\n", "target"),
    ] {
        let root = tempdir().unwrap();
        let cache = tempdir().unwrap();
        std::fs::write(root.path().join(format!("app.{extension}")), source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_swe-grep"))
            .args([
                "search",
                symbol,
                "--json-only",
                "--definitions-only",
                "--disable-fd",
                "--disable-ast-grep",
                "--disable-telemetry",
                "--path",
            ])
            .arg(root.path())
            .arg("--cache-dir")
            .arg(cache.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            value["hits"].as_array().unwrap().len(),
            source.lines().count(),
            "{value}"
        );
    }
}

fn filtered_lines(
    root: &std::path::Path,
    cache: &std::path::Path,
    query: &str,
    flag: &str,
) -> Vec<u64> {
    filtered_lines_mode(root, cache, query, flag, None)
}

fn filtered_lines_mode(
    root: &std::path::Path,
    cache: &std::path::Path,
    query: &str,
    flag: &str,
    mode: Option<&str>,
) -> Vec<u64> {
    let output = Command::new(env!("CARGO_BIN_EXE_swe-grep"))
        .args([
            "search",
            query,
            "--json-only",
            "--disable-fd",
            "--disable-ast-grep",
            "--disable-telemetry",
            "--path",
        ])
        .arg(root)
        .arg("--cache-dir")
        .arg(cache)
        .arg(flag)
        .args(mode)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut lines: Vec<_> = value["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["line"].as_u64().unwrap())
        .collect();
    lines.sort_unstable();
    lines
}

#[test]
fn language_aware_declarations_keep_calls_and_type_mentions_as_usages() {
    for (extension, source, query, definitions, usages) in [
        (
            "rs",
            "struct Target;\nimpl Target {\n #[inline]\n pub const fn target(\n value: Target\n ) -> Target {\n target();\n }\n}\n",
            "Target",
            vec![1, 2],
            vec![5, 6],
        ),
        (
            "rs",
            "#[inline]\npub const fn\n target() {\n target();\n}\n",
            "target",
            vec![3],
            vec![4],
        ),
        (
            "ts",
            "const target = () => {};\nclass C {\n target() {\n target();\n }\n}\n",
            "target",
            vec![1, 3],
            vec![4],
        ),
        (
            "js",
            "const target = () => {};\nclass C {\n target() {\n target();\n }\n}\n",
            "target",
            vec![1, 3],
            vec![4],
        ),
        (
            "swift",
            "struct C {\n func target() {\n target()\n }\n}\n",
            "target",
            vec![2],
            vec![3],
        ),
        (
            "go",
            "package app\ntype C struct {}\nfunc (c C) target() {\n target()\n}\n",
            "target",
            vec![3],
            vec![4],
        ),
        (
            "py",
            "class Target:\n def method(self):\n  return Target()\n",
            "Target",
            vec![1],
            vec![3],
        ),
    ] {
        let root = tempdir().unwrap();
        let cache = tempdir().unwrap();
        std::fs::write(root.path().join(format!("app.{extension}")), source).unwrap();
        assert_eq!(
            filtered_lines(root.path(), cache.path(), query, "--definitions-only"),
            definitions,
            "{extension} definitions"
        );
        assert_eq!(
            filtered_lines(root.path(), cache.path(), query, "--usages-only"),
            usages,
            "{extension} usages"
        );
    }
}

#[test]
fn declaration_filters_handle_regex_alternation_and_wildcards() {
    let root = tempdir().unwrap();
    let cache = tempdir().unwrap();
    std::fs::write(
        root.path().join("app.rs"),
        "fn alpha() {}\nfn beta() {}\nfn caller() {\n alpha();\n beta();\n}\n",
    )
    .unwrap();
    assert_eq!(
        filtered_lines_mode(
            root.path(),
            cache.path(),
            "a*",
            "--definitions-only",
            Some("--wildcard")
        ),
        vec![1, 2, 3]
    );
    for query in ["alpha|beta", "(alpha|beta).*", "[ab](lpha|eta)"] {
        assert_eq!(
            filtered_lines_mode(
                root.path(),
                cache.path(),
                query,
                "--definitions-only",
                Some("--regex")
            ),
            vec![1, 2]
        );
        assert_eq!(
            filtered_lines_mode(
                root.path(),
                cache.path(),
                query,
                "--usages-only",
                Some("--regex")
            ),
            vec![4, 5]
        );
    }
}

#[test]
fn declaration_filters_refresh_after_same_size_source_edit() {
    let root = tempdir().unwrap();
    let cache = tempdir().unwrap();
    let file = root.path().join("app.rs");
    std::fs::write(&file, "fn target() {}\nfn caller() {\n target();\n}\n").unwrap();
    assert_eq!(
        filtered_lines(root.path(), cache.path(), "target", "--definitions-only"),
        vec![1]
    );
    // Same byte count and no timestamp delay: declaration and usage rows swap.
    std::fs::write(&file, "fn caller() {\n target();\n}\nfn target() {}\n").unwrap();
    assert_eq!(
        filtered_lines(root.path(), cache.path(), "target", "--definitions-only"),
        vec![4]
    );
    assert_eq!(
        filtered_lines(root.path(), cache.path(), "target", "--usages-only"),
        vec![2]
    );
}

#[test]
fn rust_constants_statics_aliases_and_modules_are_declarations_not_usages() {
    let root = tempdir().unwrap();
    let cache = tempdir().unwrap();
    std::fs::write(root.path().join("app.rs"), "pub const TARGET: usize = 1;\npub static TARGET_STATIC: usize = 2;\npub type TargetAlias = usize;\npub mod target_module {}\nfn use_target(_: TargetAlias) { let _ = TARGET + TARGET_STATIC; }\n").unwrap();
    for (symbol, row) in [
        ("TARGET", 1),
        ("TARGET_STATIC", 2),
        ("TargetAlias", 3),
        ("target_module", 4),
    ] {
        let definitions = filtered_lines(root.path(), cache.path(), symbol, "--definitions-only");
        assert!(definitions.contains(&row), "{symbol}: {definitions:?}");
        let usages = filtered_lines(root.path(), cache.path(), symbol, "--usages-only");
        assert!(!usages.contains(&row), "{symbol}: {usages:?}");
    }
}

#[test]
fn js_regex_literals_and_escaped_strings_do_not_hide_later_declarations() {
    for extension in ["ts", "js"] {
        let root = tempdir().unwrap();
        let cache = tempdir().unwrap();
        std::fs::write(
            root.path().join(format!("app.{extension}")),
            "const quote = /\"/;\nexport function target() {}\nfunction caller() { target(); }\n",
        )
        .unwrap();
        assert_eq!(
            filtered_lines(root.path(), cache.path(), "target", "--definitions-only"),
            vec![2]
        );
        assert_eq!(
            filtered_lines(root.path(), cache.path(), "target", "--usages-only"),
            vec![3]
        );
    }
}

#[test]
fn local_bindings_and_properties_keep_their_declaration_rows() {
    for (extension, source) in [
        (
            "rs",
            "fn caller() {\n let target = 1;\n consume(target);\n}\n",
        ),
        (
            "swift",
            "struct Holder {\n var target = 1\n func caller() { consume(target) }\n}\n",
        ),
        (
            "ts",
            "class Holder {\n target = 1;\n caller() { consume(this.target); }\n}\n",
        ),
    ] {
        let root = tempdir().unwrap();
        let cache = tempdir().unwrap();
        std::fs::write(root.path().join(format!("app.{extension}")), source).unwrap();
        assert_eq!(
            filtered_lines(root.path(), cache.path(), "target", "--definitions-only"),
            vec![2],
            "{extension}"
        );
        assert_eq!(
            filtered_lines(root.path(), cache.path(), "target", "--usages-only"),
            vec![3],
            "{extension}"
        );
    }
}
