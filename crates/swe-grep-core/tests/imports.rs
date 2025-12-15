use std::fs;

use swe_grep::analysis::{ExtractFactsOptions, ImportGraph, ImportKind, extract_repo_facts};
use tempfile::tempdir;

#[test]
fn extracts_rust_imports_use_mod_and_extern_crate() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("repo");
    fs::create_dir_all(root.join("src")).expect("mkdir");

    fs::write(
        root.join("src/lib.rs"),
        r#"extern crate serde;
mod foo;
use crate::foo::Bar;
use crate::foo::{Baz, Qux as Quux, *};
"#,
    )
    .expect("write lib.rs");
    fs::write(root.join("src/foo.rs"), "pub struct Bar;").expect("write foo.rs");

    let cache_dir = temp.path().join("cache");
    let options = ExtractFactsOptions::for_search(None, None);
    let (_stats, facts) = extract_repo_facts(&root, &cache_dir, &options).expect("extract");

    let lib = facts
        .iter()
        .find(|f| f.file == "src/lib.rs")
        .expect("src/lib.rs facts");

    assert!(
        lib.facts.imports.iter().any(|imp| {
            imp.kind == ImportKind::SideEffect && imp.source == "serde" && imp.line == 1
        }),
        "expected extern crate serde import fact, got: {:?}",
        lib.facts.imports
    );
    assert!(
        lib.facts.imports.iter().any(|imp| {
            imp.kind == ImportKind::SideEffect && imp.source == "foo" && imp.line == 2
        }),
        "expected mod foo import fact, got: {:?}",
        lib.facts.imports
    );
    assert!(
        lib.facts.imports.iter().any(|imp| {
            imp.kind == ImportKind::Named
                && imp.source == "crate::foo"
                && imp.symbols == vec!["Bar"]
                && imp.line == 3
        }),
        "expected use crate::foo::Bar import fact, got: {:?}",
        lib.facts.imports
    );
    assert!(
        lib.facts.imports.iter().any(|imp| {
            imp.kind == ImportKind::Named
                && imp.source == "crate::foo"
                && imp.symbols == vec!["Baz"]
                && imp.line == 4
        }),
        "expected Baz import fact, got: {:?}",
        lib.facts.imports
    );
    assert!(
        lib.facts.imports.iter().any(|imp| {
            imp.kind == ImportKind::Named
                && imp.source == "crate::foo"
                && imp.symbols == vec!["Qux"]
                && imp.alias.as_deref() == Some("Quux")
                && imp.line == 4
        }),
        "expected Qux as Quux import fact, got: {:?}",
        lib.facts.imports
    );
    assert!(
        lib.facts.imports.iter().any(|imp| {
            imp.kind == ImportKind::Namespace
                && imp.source == "crate::foo"
                && imp.symbols.is_empty()
                && imp.line == 4
        }),
        "expected glob import fact, got: {:?}",
        lib.facts.imports
    );
}

#[test]
fn extracts_ts_imports_requires_and_reexports_and_builds_graph() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("repo");
    fs::create_dir_all(root.join("src")).expect("mkdir");

    fs::write(
        root.join("src/auth.ts"),
        r#"export class AuthService {}
export const Token = 1;
"#,
    )
    .expect("write auth.ts");
    fs::write(
        root.join("src/api.ts"),
        r#"import { AuthService } from "./auth";
export { AuthService } from "./auth";
export * from "./auth";
const x = require("./auth");
"#,
    )
    .expect("write api.ts");

    let cache_dir = temp.path().join("cache");
    let options = ExtractFactsOptions::for_search(None, None);
    let (_stats, facts) = extract_repo_facts(&root, &cache_dir, &options).expect("extract");

    let api = facts
        .iter()
        .find(|f| f.file == "src/api.ts")
        .expect("src/api.ts facts");

    assert!(
        api.facts.imports.iter().any(|imp| {
            imp.kind == ImportKind::Named
                && imp.source == "./auth"
                && imp.symbols == vec!["AuthService"]
                && imp.line == 1
        }),
        "expected named import fact, got: {:?}",
        api.facts.imports
    );
    assert!(
        api.facts.imports.iter().any(|imp| {
            imp.kind == ImportKind::Reexport
                && imp.source == "./auth"
                && imp.symbols == vec!["AuthService"]
                && imp.line == 2
        }),
        "expected reexport import fact, got: {:?}",
        api.facts.imports
    );
    assert!(
        api.facts.imports.iter().any(|imp| {
            imp.kind == ImportKind::Reexport
                && imp.source == "./auth"
                && imp.symbols.is_empty()
                && imp.line == 3
        }),
        "expected wildcard reexport import fact, got: {:?}",
        api.facts.imports
    );
    assert!(
        api.facts.imports.iter().any(|imp| {
            imp.kind == ImportKind::SideEffect
                && imp.source == "./auth"
                && imp.symbols.is_empty()
                && imp.line == 4
        }),
        "expected require import fact, got: {:?}",
        api.facts.imports
    );

    let graph = ImportGraph::from_facts(&facts);
    let direct = graph.imports_of("src/api.ts");
    assert!(
        direct.iter().any(|edge| edge.imported == "src/auth.ts"),
        "expected ./auth to resolve to src/auth.ts, got: {direct:?}"
    );

    let importers = graph.importers_of("src/auth.ts");
    assert!(
        importers.iter().any(|edge| edge.importer == "src/api.ts"),
        "expected src/api.ts to import src/auth.ts, got: {importers:?}"
    );

    let exporters = graph.find_export("AuthService");
    assert!(
        exporters.contains(&"src/auth.ts"),
        "expected src/auth.ts to export AuthService, got: {exporters:?}"
    );
    assert!(
        exporters.contains(&"src/api.ts"),
        "expected src/api.ts to export AuthService via re-export, got: {exporters:?}"
    );
}

#[test]
fn extracts_swift_imports() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).expect("mkdir");

    fs::write(
        root.join("App.swift"),
        r#"import Foundation
@testable import MyLib
"#,
    )
    .expect("write App.swift");

    let cache_dir = temp.path().join("cache");
    let options = ExtractFactsOptions::for_search(None, None);
    let (_stats, facts) = extract_repo_facts(&root, &cache_dir, &options).expect("extract");

    let file = facts
        .iter()
        .find(|f| f.file == "App.swift")
        .expect("App.swift facts");

    let mut modules: Vec<String> = file
        .facts
        .imports
        .iter()
        .map(|i| i.source.clone())
        .collect();
    modules.sort();
    assert_eq!(modules, vec!["Foundation".to_string(), "MyLib".to_string()]);
}
