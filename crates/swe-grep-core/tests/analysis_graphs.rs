use std::path::PathBuf;

use swe_grep::analysis::{
    CallGraph, ExtractFactsOptions, TypeDirection, TypeGraph, extract_repo_facts,
};
use tempfile::tempdir;

fn fixture_root() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // swe-grep-core
    path.pop(); // crates
    path
}

#[test]
fn callgraph_finds_rust_callers() {
    let repo_root = fixture_root().join("fixtures/multi_lang");
    let temp = tempdir().expect("tempdir");
    let cache_dir = temp.path().join("cache");

    let options = ExtractFactsOptions::for_search(None, None);
    let (_stats, file_facts) =
        extract_repo_facts(&repo_root, &cache_dir, &options).expect("extract facts");

    let graph = CallGraph::from_facts(&file_facts);
    let edges = graph.caller_edges_for("login_user");

    assert!(
        edges
            .iter()
            .any(|edge| edge.caller.contains("login_user_allows_admin")),
        "expected login_user_allows_admin to call login_user, got: {edges:?}"
    );
}

#[test]
fn typegraph_finds_swift_conformance() {
    let repo_root = fixture_root().join("fixtures/multi_lang");
    let temp = tempdir().expect("tempdir");
    let cache_dir = temp.path().join("cache");

    let options = ExtractFactsOptions::for_search(None, None);
    let (_stats, file_facts) =
        extract_repo_facts(&repo_root, &cache_dir, &options).expect("extract facts");

    let graph = TypeGraph::from_facts(&file_facts);
    let view = graph.view("UserService", TypeDirection::Down, 3, 50);
    assert!(
        view.nodes.iter().any(|node| node.symbol == "UserAPI"),
        "expected UserAPI to appear under UserService, got: {view:?}"
    );
}

#[test]
fn typegraph_finds_typescript_extends_and_implements() {
    let repo_root = fixture_root().join("fixtures/multi_lang");
    let temp = tempdir().expect("tempdir");
    let cache_dir = temp.path().join("cache");

    let options = ExtractFactsOptions::for_search(None, None);
    let (_stats, file_facts) =
        extract_repo_facts(&repo_root, &cache_dir, &options).expect("extract facts");

    let graph = TypeGraph::from_facts(&file_facts);

    let animal_down = graph.view("Animal", TypeDirection::Down, 3, 50);
    let animal_children: Vec<&str> = animal_down
        .nodes
        .iter()
        .map(|n| n.symbol.as_str())
        .collect();
    assert!(
        animal_children.contains(&"Dog"),
        "expected interface Dog extends Animal, got: {animal_children:?}"
    );
    assert!(
        animal_children.contains(&"Cat"),
        "expected class Cat implements Animal, got: {animal_children:?}"
    );

    let dog_down = graph.view("Dog", TypeDirection::Down, 3, 50);
    assert!(
        dog_down.nodes.iter().any(|node| node.symbol == "Poodle"),
        "expected class Poodle implements Dog, got: {dog_down:?}"
    );
}

#[test]
fn go_facts_extract_imports_defs_calls() {
    let repo_root = fixture_root().join("fixtures/multi_lang");
    let temp = tempdir().expect("tempdir");
    let cache_dir = temp.path().join("cache");

    let options = ExtractFactsOptions::for_search(None, None);
    let (_stats, file_facts) =
        extract_repo_facts(&repo_root, &cache_dir, &options).expect("extract facts");

    let go = file_facts
        .iter()
        .find(|f| f.language == "go" && f.file.ends_with("src/main.go"))
        .expect("expected go file facts for src/main.go");

    assert!(
        go.facts.imports.iter().any(|i| i.source == "fmt"),
        "expected fmt import, got: {:?}",
        go.facts.imports
    );
    assert!(
        go.facts.imports.iter().any(|i| i.source == "io"),
        "expected io import, got: {:?}",
        go.facts.imports
    );

    assert!(
        go.facts
            .definitions
            .iter()
            .any(|d| d.kind == "interface" && d.symbol == "Reader"),
        "expected interface Reader, got: {:?}",
        go.facts.definitions
    );
    assert!(
        go.facts
            .definitions
            .iter()
            .any(|d| d.kind == "struct" && d.symbol == "File"),
        "expected struct File, got: {:?}",
        go.facts.definitions
    );
    assert!(
        go.facts.definitions.iter().any(|d| {
            d.kind == "method"
                && d.symbol == "File::Read"
                && d.signature
                    .as_deref()
                    .unwrap_or_default()
                    .contains("func (f *File) Read")
        }),
        "expected method File::Read with signature, got: {:?}",
        go.facts.definitions
    );

    assert!(
        go.facts
            .calls
            .iter()
            .any(|c| c.caller == "File::Use" && c.callee == "helper"),
        "expected File::Use to call helper, got: {:?}",
        go.facts.calls
    );
}

#[test]
fn typegraph_finds_go_implements() {
    let repo_root = fixture_root().join("fixtures/multi_lang");
    let temp = tempdir().expect("tempdir");
    let cache_dir = temp.path().join("cache");

    let options = ExtractFactsOptions::for_search(None, None);
    let (_stats, file_facts) =
        extract_repo_facts(&repo_root, &cache_dir, &options).expect("extract facts");

    let graph = TypeGraph::from_facts(&file_facts);
    let view = graph.view("Reader", TypeDirection::Down, 2, 50);

    assert!(
        view.nodes.iter().any(|node| node.symbol == "File"),
        "expected File to appear under Reader, got: {view:?}"
    );
}

#[test]
fn python_facts_extract_imports_decorators_calls() {
    let repo_root = fixture_root().join("fixtures/multi_lang");
    let temp = tempdir().expect("tempdir");
    let cache_dir = temp.path().join("cache");

    let options = ExtractFactsOptions::for_search(None, None);
    let (_stats, file_facts) =
        extract_repo_facts(&repo_root, &cache_dir, &options).expect("extract facts");

    let py = file_facts
        .iter()
        .find(|f| f.language == "python" && f.file.ends_with("src/app.py"))
        .expect("expected python file facts for src/app.py");

    assert!(
        py.facts
            .imports
            .iter()
            .any(|i| i.source == "typing" && i.symbols == vec!["Protocol"]),
        "expected from typing import Protocol, got: {:?}",
        py.facts.imports
    );
    assert!(
        py.facts
            .imports
            .iter()
            .any(|i| i.source == "sys" && i.alias.as_deref() == Some("system")),
        "expected import sys as system, got: {:?}",
        py.facts.imports
    );

    assert!(
        py.facts
            .definitions
            .iter()
            .any(|d| d.kind == "decorator" && d.symbol == "deco"),
        "expected decorator deco, got: {:?}",
        py.facts.definitions
    );
    assert!(
        py.facts
            .definitions
            .iter()
            .any(|d| d.kind == "decorator" && d.symbol == "runtime_checkable"),
        "expected decorator runtime_checkable, got: {:?}",
        py.facts.definitions
    );

    assert!(
        py.facts.definitions.iter().any(|d| {
            d.kind == "async_func"
                && d.symbol == "main"
                && d.signature
                    .as_deref()
                    .unwrap_or_default()
                    .contains("async def main")
        }),
        "expected async def main with signature, got: {:?}",
        py.facts.definitions
    );
    assert!(
        py.facts.definitions.iter().any(|d| {
            d.kind == "method"
                && d.symbol == "Job::run"
                && d.signature
                    .as_deref()
                    .unwrap_or_default()
                    .contains("def run")
        }),
        "expected method Job::run with signature, got: {:?}",
        py.facts.definitions
    );

    assert!(
        py.facts
            .calls
            .iter()
            .any(|c| c.caller == "Job::run" && c.callee == "helper"),
        "expected Job::run to call helper, got: {:?}",
        py.facts.calls
    );
}

#[test]
fn typegraph_finds_python_protocol_and_inheritance() {
    let repo_root = fixture_root().join("fixtures/multi_lang");
    let temp = tempdir().expect("tempdir");
    let cache_dir = temp.path().join("cache");

    let options = ExtractFactsOptions::for_search(None, None);
    let (_stats, file_facts) =
        extract_repo_facts(&repo_root, &cache_dir, &options).expect("extract facts");

    let graph = TypeGraph::from_facts(&file_facts);

    let base_down = graph.view("Base", TypeDirection::Down, 2, 50);
    assert!(
        base_down.nodes.iter().any(|node| node.symbol == "Job"),
        "expected Job to appear under Base, got: {base_down:?}"
    );

    let runner_down = graph.view("Runner", TypeDirection::Down, 2, 50);
    assert!(
        runner_down.nodes.iter().any(|node| node.symbol == "Job"),
        "expected Job to appear under Runner (protocol), got: {runner_down:?}"
    );
}
