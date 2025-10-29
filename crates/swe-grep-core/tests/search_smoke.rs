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
        context_before: 0,
        context_after: 0,
        body: false,
        enable_index: false,
        index_dir: None,
        enable_rga: false,
        cache_dir: None,
        log_dir: None,
        use_fd: true,
        use_ast_grep: true,
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
        context_before: 0,
        context_after: 0,
        body: false,
        enable_index: false,
        index_dir: None,
        enable_rga: false,
        cache_dir: None,
        log_dir: Some(log_dir.clone()),
        use_fd: true,
        use_ast_grep: true,
    };

    let _summary = search::execute(args).await.expect("search should succeed");

    let log_path = log_dir.join("search.log.jsonl");
    assert!(log_path.exists(), "expected search log to be created");

    let contents = tokio::fs::read_to_string(&log_path)
        .await
        .expect("failed to read search log");
    assert!(contents.contains("\"symbol\":\"getUser\""));
}

#[tokio::test]
async fn surfaces_expanded_snippet_metadata() {
    let repo_root = fixture_root().join("fixtures/multi_lang");

    let args = SearchArgs {
        symbol: "login_user_allows_admin".to_string(),
        path: Some(repo_root),
        language: Some("rust".to_string()),
        timeout_secs: 3,
        max_matches: 20,
        concurrency: 8,
        context_before: 1,
        context_after: 1,
        body: false,
        enable_index: false,
        index_dir: None,
        enable_rga: false,
        cache_dir: None,
        log_dir: None,
        use_fd: true,
        use_ast_grep: true,
    };

    let summary = search::execute(args).await.expect("search should succeed");
    let hit = summary
        .top_hits
        .iter()
        .find(|hit| hit.path.ends_with("src/lib.rs"))
        .expect("expected rust lib.rs hit with expanded context");

    assert!(
        hit.raw_snippet.is_some(),
        "expected raw ripgrep payload to be preserved"
    );
    assert!(
        hit.body.is_none(),
        "body should remain absent when the flag is not enabled"
    );
    assert!(
        hit.snippet_length.unwrap_or_default() > 0,
        "snippet_length should record the original ripgrep text length"
    );
    assert!(
        !hit.raw_snippet_truncated,
        "fixture snippet should not be truncated at default column limits"
    );

    let expanded = hit
        .expanded_snippet
        .as_ref()
        .expect("expected expanded snippet to be populated");
    assert!(
        expanded.contains("login_user_allows_admin"),
        "expanded snippet should contain the target symbol"
    );
    assert!(
        !hit.auto_expanded_context,
        "explicit context window should not be marked as auto expanded"
    );

    let start = hit
        .context_start
        .expect("expected context_start to accompany expanded snippet");
    let end = hit
        .context_end
        .expect("expected context_end to accompany expanded snippet");
    assert!(
        start <= hit.line && hit.line <= end,
        "line {} should be within the expanded context window {}-{}",
        hit.line,
        start,
        end
    );
    assert_eq!(
        start,
        hit.line.saturating_sub(1),
        "expected one line of leading context"
    );
    assert_eq!(end, hit.line + 1, "expected one line of trailing context");
    assert_eq!(
        expanded.lines().count(),
        3,
        "window should include neighbours"
    );
}

#[tokio::test]
async fn auto_expands_context_when_flags_omitted() {
    let repo_root = fixture_root().join("fixtures/multi_lang");

    let args = SearchArgs {
        symbol: "login_user_allows_admin".to_string(),
        path: Some(repo_root),
        language: Some("rust".to_string()),
        timeout_secs: 3,
        max_matches: 20,
        concurrency: 8,
        context_before: 0,
        context_after: 0,
        body: false,
        enable_index: false,
        index_dir: None,
        enable_rga: false,
        cache_dir: None,
        log_dir: None,
        use_fd: true,
        use_ast_grep: true,
    };

    let summary = search::execute(args).await.expect("search should succeed");
    let hit = summary
        .top_hits
        .iter()
        .find(|hit| hit.path.ends_with("src/lib.rs"))
        .expect("expected rust lib.rs hit when relying on default context");

    let expanded = hit
        .expanded_snippet
        .as_ref()
        .expect("expanded_snippet should be populated by default");
    let start = hit
        .context_start
        .expect("context_start should accompany expanded snippet");
    let end = hit
        .context_end
        .expect("context_end should accompany expanded snippet");

    assert!(
        start < hit.line,
        "default context should include at least one leading line"
    );
    assert!(
        end > hit.line,
        "default context should include at least one trailing line"
    );
    assert!(
        expanded.lines().count() >= 3,
        "expanded snippet should include surrounding lines when context flags are omitted"
    );
    assert!(
        hit.auto_expanded_context,
        "auto_expanded_context should be true when default padding is applied"
    );
}

#[tokio::test]
async fn retrieves_body_when_requested() {
    let repo_root = fixture_root().join("fixtures/multi_lang");

    let args = SearchArgs {
        symbol: "login_user_allows_admin".to_string(),
        path: Some(repo_root),
        language: Some("rust".to_string()),
        timeout_secs: 3,
        max_matches: 20,
        concurrency: 8,
        context_before: 0,
        context_after: 0,
        body: true,
        enable_index: false,
        index_dir: None,
        enable_rga: false,
        cache_dir: None,
        log_dir: None,
        use_fd: true,
        use_ast_grep: true,
    };

    let summary = search::execute(args).await.expect("search should succeed");
    let hit = summary
        .top_hits
        .iter()
        .find(|hit| hit.path.ends_with("src/lib.rs"))
        .expect("expected rust lib.rs hit when requesting body");

    assert!(
        hit.body_retrieved,
        "body_retrieved should be true on success"
    );
    let body = hit
        .body
        .as_ref()
        .expect("body payload should be populated when retrieved");
    assert!(
        body.contains("login_user_allows_admin"),
        "body should include the target symbol"
    );
    assert!(
        body.len() < 10_000,
        "fixture body should comfortably sit below guardrail limits"
    );
}
