use std::fs;
use std::path::PathBuf;

use swe_grep_indexer::{IndexConfig, TantivyIndex};

fn config(root: &std::path::Path) -> IndexConfig {
    IndexConfig {
        root: root.to_owned(),
        index_dir: root.join("index"),
        extensions: Some(vec!["rs".into()]),
    }
}

#[test]
fn candidates_are_exhaustive_or_require_a_scan() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("one.rs"), "fn login_user() {}\n").unwrap();
        fs::write(
            root.path().join("two.rs"),
            "fn caller() { login_user(); }\n",
        )
        .unwrap();
        let index = TantivyIndex::open_or_build(config(root.path()))
            .await
            .unwrap();
        let mut paths = index
            .exact_identifier_candidates("login_user", 10)
            .await
            .unwrap()
            .unwrap();
        paths.sort();
        assert_eq!(
            paths,
            vec![PathBuf::from("one.rs"), PathBuf::from("two.rs")]
        );
        // Never mistake a bounded candidate list for complete coverage.
        assert!(
            index
                .exact_identifier_candidates("login_user", 1)
                .await
                .unwrap()
                .is_none()
        );
        for unsupported in ["___".to_owned(), "a.b".into(), "x".repeat(40)] {
            assert!(
                index
                    .exact_identifier_candidates(&unsupported, 10)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    });
}

#[test]
fn candidates_refresh_changed_and_deleted_sources() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        fs::write(root.path().join("changed.rs"), "fn old_symbol() {}\n").unwrap();
        fs::write(root.path().join("deleted.rs"), "fn old_symbol() {}\n").unwrap();
        drop(TantivyIndex::open_or_build(config.clone()).await.unwrap());
        fs::write(root.path().join("changed.rs"), "fn new_symbol() {}\n").unwrap();
        fs::remove_file(root.path().join("deleted.rs")).unwrap();
        let index = TantivyIndex::open_or_build(config).await.unwrap();
        assert_eq!(
            index
                .exact_identifier_candidates("new_symbol", 10)
                .await
                .unwrap(),
            Some(vec![PathBuf::from("changed.rs")])
        );
        assert_eq!(
            index
                .exact_identifier_candidates("old_symbol", 10)
                .await
                .unwrap(),
            Some(vec![])
        );
    });
}

#[test]
fn incomplete_utf8_coverage_requires_scanning_including_on_warm_open() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("valid.rs"), "fn target() {}\n").unwrap();
        fs::write(root.path().join("invalid.rs"), b"target\xff").unwrap();
        let config = config(root.path());
        for _ in 0..2 {
            let index = TantivyIndex::open_or_build(config.clone()).await.unwrap();
            assert!(!index.stats().coverage_complete);
            assert!(
                index
                    .exact_identifier_candidates("target", 10)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    });
}
