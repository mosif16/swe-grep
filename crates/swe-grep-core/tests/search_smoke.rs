use std::path::PathBuf;

use swe_grep::cli::{SearchArgs, ToolArgs};
use swe_grep::search;
use tempfile::tempdir;

fn fixture_root() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // swe-grep-core
    path.pop(); // crates
    path
}

fn base_args(symbol: &str, path: PathBuf, language: Option<&str>) -> SearchArgs {
    SearchArgs {
        symbol: symbol.to_string(),
        text: false,
        concept: None,
        concept_expand: false,
        regex: false,
        literal_regex: false,
        wildcard: false,
        attribute: false,
        string: false,
        verbose_ast: false,
        include_strings: false,
        code_only: false,
        ignore_case: false,
        fuzzy_threshold: 0.7,
        definitions_only: false,
        usages_only: false,
        path: Some(path),
        scope_file: None,
        path_glob: Vec::new(),
        exclude_patterns: Vec::new(),
        exclude_tests: false,
        language: language.map(|lang| lang.to_string()),
        mode: None,
        kind: None,
        timeout_secs: 3,
        max_matches: 20,
        top_hits: 20,
        max_columns: 200,
        concurrency: 8,
        tools: ToolArgs {
            enable_index: false,
            enable_rga: false,
            use_fd: true,
            use_ast_grep: true,
        },
        context: None,
        context_before: 0,
        context_after: 0,
        body: false,
        body_limit: None,
        no_truncate: false,
        git_modified: false,
        git_since: None,
        git_info: false,
        index_dir: None,
        cache_dir: None,
        log_dir: None,
    }
}

#[tokio::test]
async fn finds_rust_symbol() {
    let repo_root = fixture_root().join("fixtures/multi_lang");
    let args = base_args("login_user", repo_root, Some("rust"));

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
async fn finds_swift_definitions_when_filtered() {
    let repo_root = fixture_root().join("fixtures/multi_lang");
    let mut args = base_args("fetchUser", repo_root, Some("swift"));
    args.definitions_only = true;

    let summary = search::execute(args).await.expect("search should succeed");
    assert!(
        summary
            .top_hits
            .iter()
            .any(|hit| hit.path.ends_with("App.swift")),
        "expected App.swift to appear in top hits when filtering to definitions"
    );
}

#[tokio::test]
async fn writes_log_when_requested() {
    let repo_root = fixture_root().join("fixtures/multi_lang");
    let temp = tempdir().expect("failed to create tempdir");
    let log_dir = temp.path().join("logs");

    let mut args = base_args("getUser", repo_root, Some("ts"));
    args.log_dir = Some(log_dir.clone());

    let _summary = search::execute(args).await.expect("search should succeed");

    let log_path = log_dir.join("search.log.jsonl");
    assert!(log_path.exists(), "expected search log to be created");

    let contents = tokio::fs::read_to_string(&log_path)
        .await
        .expect("failed to read search log");
    assert!(contents.contains("\"symbol\":\"getUser\""));
}

#[tokio::test]
async fn scope_file_restricts_results_to_single_file() {
    let tmp = tempdir().expect("failed to create tempdir");
    let root = tmp.path().to_path_buf();

    let a = root.join("A.swift");
    let b = root.join("B.swift");
    tokio::fs::write(&a, "private func deckHeaderSection() -> Int { 1 }\n")
        .await
        .expect("failed to write A.swift");
    tokio::fs::write(&b, "private func deckHeaderSection() -> Int { 2 }\n")
        .await
        .expect("failed to write B.swift");

    let mut args = base_args("deckHeaderSection", root.clone(), Some("swift"));
    let summary = search::execute(args.clone())
        .await
        .expect("search should succeed");
    assert!(
        summary
            .top_hits
            .iter()
            .any(|hit| hit.path.ends_with("A.swift")),
        "expected unscoped search to match A.swift"
    );
    assert!(
        summary
            .top_hits
            .iter()
            .any(|hit| hit.path.ends_with("B.swift")),
        "expected unscoped search to match B.swift"
    );

    args.scope_file = Some(PathBuf::from("B.swift"));
    let scoped = search::execute(args).await.expect("search should succeed");
    assert!(
        !scoped.top_hits.is_empty(),
        "expected scoped search to return at least one hit"
    );
    assert!(
        scoped
            .top_hits
            .iter()
            .all(|hit| hit.path.ends_with("B.swift")),
        "expected scoped search to only return hits from B.swift"
    );
}

#[tokio::test]
async fn preserves_path_when_searching_single_file_root() {
    let temp = tempdir().expect("failed to create tempdir");
    let log_path = temp.path().join("search.log.jsonl");
    tokio::fs::write(
        &log_path,
        "{\"path\":\"src/lib.rs\",\"message\":\"something happened\"}\n",
    )
    .await
    .expect("failed to write temp log");

    let mut args = base_args("src/lib.rs", log_path.clone(), None);
    args.text = true;
    args.scope_file = Some(log_path.clone());

    let summary = search::execute(args).await.expect("search should succeed");
    let hit = summary
        .top_hits
        .first()
        .expect("expected at least one hit in log file");
    assert!(
        !hit.path.is_empty(),
        "expected hit path to be preserved when --path points to a file"
    );
    assert!(
        hit.path.ends_with("search.log.jsonl"),
        "expected hit path to name the scoped file (got {})",
        hit.path
    );
}

#[tokio::test]
async fn literal_regex_disables_query_expansion() {
    let temp = tempdir().expect("failed to create tempdir");
    let file_path = temp.path().join("sample.txt");
    tokio::fs::write(&file_path, "abc\n")
        .await
        .expect("failed to write temp file");

    let mut args = base_args("a.c", temp.path().to_path_buf(), None);
    args.regex = true;
    args.literal_regex = true;
    args.body = true;

    let summary = search::execute(args).await.expect("search should succeed");
    assert_eq!(
        summary.queries,
        vec!["a.c".to_string()],
        "expected literal-regex mode to keep a single raw query"
    );
    assert!(
        summary
            .top_hits
            .iter()
            .any(|hit| hit.path.ends_with("sample.txt")),
        "expected the temp file to be searched"
    );
}

#[tokio::test]
async fn body_limit_truncates_large_files_in_output() {
    let temp = tempdir().expect("failed to create tempdir");
    let file_path = temp.path().join("big.log");
    let mut content = String::new();
    content.push_str("hello\n");
    content.push_str(&"x".repeat(10_000));
    tokio::fs::write(&file_path, content)
        .await
        .expect("failed to write temp file");

    let mut args = base_args("hello", temp.path().to_path_buf(), None);
    args.text = true;
    args.body = true;
    args.scope_file = Some(file_path);
    args.body_limit = Some(128);

    let summary = search::execute(args).await.expect("search should succeed");
    let hit = summary.top_hits.first().expect("expected at least one hit");
    assert!(hit.body_retrieved, "expected body to be retrieved");
    assert!(hit.body_truncated, "expected body to be truncated");
    assert!(
        hit.body.as_deref().unwrap_or_default().len() <= 256,
        "expected truncated body to remain small"
    );
}

#[tokio::test]
async fn surfaces_expanded_snippet_metadata() {
    let repo_root = fixture_root().join("fixtures/multi_lang");

    let mut args = base_args("login_user_allows_admin", repo_root, Some("rust"));
    args.context_before = 1;
    args.context_after = 1;

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
        hit.body.is_some(),
        "rust hits should surface full file bodies even without --body"
    );
    assert!(
        hit.body_retrieved,
        "body_retrieved should be set when the file payload is loaded"
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
    assert!(
        !hit.hints.is_empty(),
        "context hints should be emitted for rust hits"
    );
    assert!(
        hit.hints.iter().any(|hint| hint.kind == "declaration"),
        "expected declaration hint to identify the matched function"
    );
}

#[tokio::test]
async fn auto_expands_context_when_flags_omitted() {
    let repo_root = fixture_root().join("fixtures/multi_lang");

    let args = base_args("login_user_allows_admin", repo_root, Some("rust"));

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

    let mut args = base_args("login_user_allows_admin", repo_root, Some("rust"));
    args.body = true;

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

#[tokio::test]
async fn surfaces_swift_context_hints() {
    let repo_root = fixture_root().join("fixtures/multi_lang");

    let args = base_args("hydrateAndNotify", repo_root, Some("swift"));

    let summary = search::execute(args).await.expect("search should succeed");
    let hit = summary
        .top_hits
        .iter()
        .find(|hit| hit.path.ends_with("App.swift"))
        .expect("expected swift App.swift hit when retrieving context");

    assert!(
        hit.body.is_some(),
        "swift hits should surface full file bodies even without --body"
    );
    assert!(
        hit.body_retrieved,
        "swift file body should be marked as retrieved"
    );
    assert!(
        !hit.hints.is_empty(),
        "swift hits should include contextual hints"
    );
    assert!(
        hit.hints.iter().any(|hint| hint.kind == "declaration"),
        "expected declaration hint to identify the matched member"
    );
    assert!(
        hit.hints.iter().any(|hint| {
            hint.kind == "extension"
                || (hint.kind == "type" && hint.label.to_ascii_lowercase().contains("userapi"))
        }),
        "expected hints to call out the surrounding type or extension"
    );
}
