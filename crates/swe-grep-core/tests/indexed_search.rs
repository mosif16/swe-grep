#![cfg(feature = "indexing")]

use std::path::Path;

use clap::Parser;
use swe_grep::cli::SearchArgs;
use swe_grep::search;

#[derive(Parser)]
struct SearchOptions {
    #[command(flatten)]
    args: SearchArgs,
}

fn arguments(root: &Path, cache: &Path, query: &str) -> SearchArgs {
    SearchOptions::parse_from([
        "test-search",
        "--symbol",
        query,
        "--path",
        root.to_str().unwrap(),
        "--cache-dir",
        cache.to_str().unwrap(),
        "--index-dir",
        cache.join("index").to_str().unwrap(),
        "--enable-index",
        "--exact-symbol",
        "--disable-fd",
        "--disable-ast-grep",
    ])
    .args
}

#[tokio::test]
async fn exact_search_uses_index_before_scanning_and_refreshes_edits_and_deletions() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("changed.rs"), "fn target() {}\n").unwrap();
    std::fs::write(root.path().join("deleted.rs"), "fn target() {}\n").unwrap();
    std::fs::write(root.path().join("unrelated.rs"), "fn unrelated() {}\n").unwrap();
    let args = arguments(root.path(), cache.path(), "target");
    let cold = search::execute(args.clone()).await.unwrap();
    assert_eq!(cold.stage_stats.index_candidates, 2);
    assert_eq!(cold.top_hits.len(), 2);
    assert!(cold.top_hits.iter().all(|hit| hit.origin == "rg-indexed"));

    std::fs::write(root.path().join("changed.rs"), "fn changed() {}\n").unwrap();
    std::fs::remove_file(root.path().join("deleted.rs")).unwrap();
    std::fs::write(root.path().join("added.rs"), "fn target() {}\n").unwrap();
    let refreshed = search::execute(args).await.unwrap();
    assert_eq!(refreshed.stage_stats.index_candidates, 1);
    assert_eq!(refreshed.top_hits.len(), 1);
    assert!(refreshed.top_hits[0].path.ends_with("added.rs"));
    assert_eq!(refreshed.top_hits[0].origin, "rg-indexed");
}

#[tokio::test]
async fn substring_and_regex_queries_preserve_scan_semantics() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("app.rs"),
        "fn target_suffix() {}\nfn other() {}\n",
    )
    .unwrap();
    let mut args = arguments(root.path(), cache.path(), "target");
    args.exact_symbol = false;
    let substring = search::execute(args.clone()).await.unwrap();
    assert_eq!(substring.top_hits.len(), 1);
    assert_ne!(substring.top_hits[0].origin, "rg-indexed");

    args.symbol = "target_suffix|other".into();
    args.regex = true;
    let regex = search::execute(args).await.unwrap();
    assert_eq!(regex.top_hits.len(), 2);
    assert!(regex.top_hits.iter().all(|hit| hit.origin != "rg-indexed"));
}

#[tokio::test]
async fn unsupported_long_identifier_falls_back_without_losing_a_match() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let query = "x".repeat(40);
    std::fs::write(root.path().join("app.rs"), format!("fn {query}() {{}}\n")).unwrap();
    let summary = search::execute(arguments(root.path(), cache.path(), &query))
        .await
        .unwrap();
    assert_eq!(summary.top_hits.len(), 1);
    assert_ne!(summary.top_hits[0].origin, "rg-indexed");
}
