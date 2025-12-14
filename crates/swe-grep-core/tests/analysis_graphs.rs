use std::path::PathBuf;

use swe_grep::analysis::{extract_repo_facts, CallGraph, ExtractFactsOptions, TypeDirection, TypeGraph};
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
        edges.iter().any(|edge| edge.caller.contains("login_user_allows_admin")),
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
    let animal_children: Vec<&str> = animal_down.nodes.iter().map(|n| n.symbol.as_str()).collect();
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

