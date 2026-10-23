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
        .unwrap()
}

fn json(root: &Path, args: &[&str]) -> Value {
    let output = run(root, args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

const LANGUAGES: [(&str, &str, &str, &str); 3] = [
    (
        "rust",
        "rs",
        "pub fn target() {}
pub fn unrelated() { decoy(); }
pub fn actual() { target(); }
pub fn reference() { let value = target; }
",
        "pub fn decoy() {}
",
    ),
    (
        "python",
        "py",
        "def target():
    pass
def unrelated():
    decoy()
def actual():
    target()
def reference():
    value = target
",
        "def decoy():
    pass
",
    ),
    (
        "typescript",
        "ts",
        "export function target() {}
function unrelated() { decoy(); }
function actual() { target(); }
const reference = target;
",
        "export function decoy() {}
",
    ),
];

#[test]
fn language_filtered_graphs_do_not_guess_calls_from_other_functions_or_references() {
    for (language, ext, source, other) in LANGUAGES {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join(format!("main.{ext}")), source).unwrap();
        fs::write(temp.path().join(format!("other.{ext}")), other).unwrap();
        for filters in [
            vec![],
            vec!["--language", language],
            vec!["--language", language, "--exclude-tests"],
        ] {
            let mut args = vec!["refs", "target", ".", "--callers", "--callees"];
            args.extend(filters);
            let payload = json(temp.path(), &args);
            assert_eq!(
                payload["callees"],
                serde_json::json!([]),
                "{language}: {payload}"
            );
            let callers = payload["callers"].as_array().unwrap();
            assert_eq!(callers.len(), 1, "{language}: {payload}");
            assert_eq!(callers[0]["origin"], "callgraph", "{payload}");
            assert!(
                callers[0]["snippet"].as_str().unwrap().contains("target()"),
                "{payload}"
            );
        }
    }
}

#[test]
fn scoped_multilanguage_callees_use_call_origin_not_callee_definition_file() {
    for (language, ext, _, helper) in LANGUAGES {
        let temp = tempdir().unwrap();
        let source = match language {
            "rust" => {
                "pub fn target() { decoy(); }
pub fn unrelated() { forbidden(); }
"
            }
            "python" => {
                "def target():
    decoy()
def unrelated():
    forbidden()
"
            }
            _ => {
                "export function target() { decoy(); }
function unrelated() { forbidden(); }
"
            }
        };
        let allowed = format!("allowed.{ext}");
        let excluded = format!("excluded.{ext}");
        fs::write(temp.path().join(&allowed), source).unwrap();
        fs::write(temp.path().join(&excluded), helper).unwrap();
        for flag in ["--scope-file", "--path-glob"] {
            let payload = json(
                temp.path(),
                &[
                    "refs",
                    "target",
                    "--path",
                    ".",
                    "--callees",
                    "--language",
                    language,
                    flag,
                    &allowed,
                ],
            );
            let callees = payload["callees"].as_array().unwrap();
            assert_eq!(callees.len(), 1, "{language} {flag}: {payload}");
            assert_eq!(callees[0]["symbol"], "decoy", "{payload}");
            assert_eq!(callees[0]["file"], allowed, "{payload}");
        }
        let payload = json(temp.path(), &["refs", "target", &allowed, "--callees"]);
        assert_eq!(payload["callees"][0]["symbol"], "decoy", "{payload}");
    }
}

#[test]
fn excluding_one_import_endpoint_preserves_other_resolved_imports_and_callees() {
    let temp = tempdir().unwrap();
    fs::write(
        temp.path().join("entry.ts"),
        "import { helper } from './helper';
import { secret } from './excluded';
export function target() { helper(); }
function unrelated() { secret(); }
",
    )
    .unwrap();
    fs::write(
        temp.path().join("helper.ts"),
        "export function helper() {}
",
    )
    .unwrap();
    fs::write(
        temp.path().join("excluded.ts"),
        "export function secret() {}
",
    )
    .unwrap();
    fs::write(
        temp.path().join("consumer.ts"),
        "import { target } from './entry';
export function caller() { target(); }
",
    )
    .unwrap();
    let payload = json(
        temp.path(),
        &[
            "refs",
            "target",
            ".",
            "--callers",
            "--callees",
            "--imports",
            "--importers",
            "--language",
            "typescript",
            "--exclude",
            "excluded.ts",
        ],
    );
    assert_eq!(payload["callees"].as_array().unwrap().len(), 1, "{payload}");
    assert_eq!(payload["callees"][0]["symbol"], "helper", "{payload}");
    assert_eq!(payload["imports"].as_array().unwrap().len(), 1, "{payload}");
    assert_eq!(payload["imports"][0]["file"], "helper.ts", "{payload}");
    assert_eq!(payload["importers"][0]["file"], "consumer.ts", "{payload}");
    assert_eq!(payload["callers"].as_array().unwrap().len(), 1, "{payload}");
    let hidden_target = json(
        temp.path(),
        &[
            "refs",
            "target",
            ".",
            "--imports",
            "--importers",
            "--exclude",
            "entry.ts",
        ],
    );
    assert!(hidden_target.get("imports").is_none(), "{hidden_target}");
    assert!(hidden_target.get("importers").is_none(), "{hidden_target}");
}

#[test]
fn mixed_language_graphs_filter_origins_without_disabling_semantics() {
    let temp = tempdir().unwrap();
    for (_, ext, source, _) in LANGUAGES {
        fs::write(temp.path().join(format!("main.{ext}")), source).unwrap();
    }
    for (language, ext, _, _) in LANGUAGES {
        let payload = json(
            temp.path(),
            &[
                "refs",
                "target",
                ".",
                "--callers",
                "--callees",
                "--language",
                language,
            ],
        );
        let callers = payload["callers"].as_array().unwrap();
        assert_eq!(callers.len(), 1, "{language}: {payload}");
        assert_eq!(callers[0]["file"], format!("main.{ext}"), "{payload}");
        assert_eq!(callers[0]["origin"], "callgraph", "{payload}");
        assert_eq!(payload["callees"], serde_json::json!([]), "{payload}");
    }
}

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

#[test]
fn modified_and_since_scopes_filter_warm_graph_origins() {
    let temp = tempdir().unwrap();
    fs::write(
        temp.path().join("allowed.rs"),
        "pub fn allowed() { target(); }\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("unchanged.rs"),
        "pub fn unchanged() { target(); }\n",
    )
    .unwrap();
    git(temp.path(), &["init", "--quiet"]);
    git(temp.path(), &["add", "."]);
    git(
        temp.path(),
        &[
            "-c",
            "user.name=Refs Test",
            "-c",
            "user.email=refs@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "initial",
        ],
    );
    let warm = json(temp.path(), &["refs", "target", ".", "--callers"]);
    assert_eq!(warm["callers"].as_array().unwrap().len(), 2, "{warm}");
    fs::write(
        temp.path().join("allowed.rs"),
        "pub fn allowed() { target(); target(); }\n",
    )
    .unwrap();
    for filter in [
        vec!["--git-modified"],
        vec!["--git-since", "HEAD"],
        vec!["--git-modified", "--tracked-only"],
    ] {
        let mut args = vec!["refs", "target", ".", "--callers"];
        args.extend(filter);
        let payload = json(temp.path(), &args);
        let callers = payload["callers"].as_array().unwrap();
        assert_eq!(callers.len(), 1, "{payload}");
        assert_eq!(callers[0]["file"], "allowed.rs", "{payload}");
        assert_eq!(callers[0]["origin"], "callgraph", "{payload}");
    }
}

#[test]
fn scoped_rust_constructor_filtering_is_preserved() {
    let temp = tempdir().unwrap();
    fs::write(
        temp.path().join("lib.rs"),
        "pub fn target() { helper(); Some(1); Ok(2); }\npub fn helper() {}\n",
    )
    .unwrap();
    let default = json(
        temp.path(),
        &["refs", "target", ".", "--callees", "--language", "rust"],
    );
    assert_eq!(default["callees"].as_array().unwrap().len(), 1, "{default}");
    assert_eq!(default["callees"][0]["symbol"], "helper", "{default}");
    let included = json(
        temp.path(),
        &[
            "refs",
            "target",
            ".",
            "--callees",
            "--language",
            "rust",
            "--include-constructors",
        ],
    );
    let symbols: Vec<_> = included["callees"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["symbol"].as_str().unwrap())
        .collect();
    assert!(
        symbols.contains(&"Some") && symbols.contains(&"Ok") && symbols.contains(&"helper"),
        "{included}"
    );
}

#[test]
fn partial_semantic_graph_warns_and_does_not_fabricate_edges() {
    let temp = tempdir().unwrap();
    fs::write(
        temp.path().join("lib.rs"),
        "pub fn target() {}\npub fn unrelated() { decoy(); }\n",
    )
    .unwrap();
    fs::write(temp.path().join("broken.rs"), [0xff, 0xfe]).unwrap();
    let payload = json(
        temp.path(),
        &["refs", "target", ".", "--callers", "--callees"],
    );
    assert_eq!(payload["callers"], serde_json::json!([]), "{payload}");
    assert_eq!(payload["callees"], serde_json::json!([]), "{payload}");
    assert!(
        payload["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning
                .as_str()
                .unwrap()
                .contains("incomplete refs: call graph analysis reported warnings")),
        "{payload}"
    );
}

#[test]
fn declaration_filters_are_rejected_instead_of_silently_bypassed() {
    let temp = tempdir().unwrap();
    fs::write(
        temp.path().join("lib.rs"),
        "pub fn target() {}
pub fn caller() { target(); }
",
    )
    .unwrap();
    for flags in [vec!["--definitions-only"], vec!["--kind", "func"]] {
        let mut args = vec!["refs", "target", ".", "--callers"];
        args.extend(flags);
        let output = run(temp.path(), &args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("refs reports usage edges"));
    }
    let payload = json(
        temp.path(),
        &["refs", "target", ".", "--callers", "--usages-only"],
    );
    assert_eq!(payload["callers"].as_array().unwrap().len(), 1, "{payload}");
}
