use std::path::PathBuf;

use swe_grep::cli::SearchArgs;
use swe_grep::search;
use tempfile::tempdir;

fn fixture_root() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // swe-grep-core
    path.pop(); // crates
    path
}

#[tokio::test]
async fn finds_rust_symbol() {
    let repo_root = fixture_root().join("fixtures/multi_lang");
    let args = SearchArgs {
        symbol: "login_user".to_string(),
        path: Some(repo_root),
        language: Some("rust".to_string()),
        timeout_secs: 3,
        max_matches: 20,
        concurrency: 8,
        enable_index: false,
        index_dir: None,
        enable_rga: false,
        cache_dir: None,
        log_dir: None,
    };

    let summary = search::execute(args).await.expect("search should succeed");
    assert!(
        summary
            .top_hits
            .iter()
            .any(|hit| hit.path.ends_with("src/lib.rs")),
        "expected rust lib.rs to appear in top hits"
    );
}

#[tokio::test]
async fn writes_log_when_requested() {
    let repo_root = fixture_root().join("fixtures/multi_lang");
    let temp = tempdir().expect("failed to create tempdir");
    let log_dir = temp.path().join("logs");

    let args = SearchArgs {
        symbol: "getUser".to_string(),
        path: Some(repo_root),
        language: Some("ts".to_string()),
        timeout_secs: 3,
        max_matches: 20,
        concurrency: 8,
        enable_index: false,
        index_dir: None,
        enable_rga: false,
        cache_dir: None,
        log_dir: Some(log_dir.clone()),
    };

    let _summary = search::execute(args).await.expect("search should succeed");

    let log_path = log_dir.join("search.log.jsonl");
    assert!(log_path.exists(), "expected search log to be created");

    let contents = tokio::fs::read_to_string(&log_path)
        .await
        .expect("failed to read search log");
    assert!(contents.contains("\"symbol\":\"getUser\""));
}
