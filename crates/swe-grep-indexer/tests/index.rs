use std::fs;
use std::path::PathBuf;

use swe_grep_indexer::{IndexConfig, TantivyIndex};
use tempfile::TempDir;

fn config(dir: &TempDir) -> IndexConfig {
    let root = dir.path().join("source");
    fs::create_dir_all(&root).unwrap();
    IndexConfig {
        index_dir: root.join("search-index"),
        root,
        extensions: None,
    }
}

#[test]
fn new_index_returns_source_hits() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let config = config(&dir);
        fs::write(config.root.join("hello.rs"), "freshmarker").unwrap();
        let index = TantivyIndex::open_or_build(config).await.unwrap();
        assert_eq!(
            index.search("freshmarker", 10).await.unwrap(),
            vec![PathBuf::from("hello.rs")]
        );
    });
}

#[test]
fn reopening_replaces_changed_deleted_and_added_sources_without_duplicates() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let config = config(&dir);
        fs::write(config.root.join("changed.rs"), "oldmarker").unwrap();
        fs::write(config.root.join("deleted.rs"), "deletedmarker").unwrap();
        let index = TantivyIndex::open_or_build(config.clone()).await.unwrap();
        assert_eq!(
            index.search("oldmarker", 10).await.unwrap(),
            vec![PathBuf::from("changed.rs")]
        );
        assert_eq!(
            index.search("deletedmarker", 10).await.unwrap(),
            vec![PathBuf::from("deleted.rs")]
        );
        drop(index);

        fs::write(config.root.join("changed.rs"), "newmarker").unwrap();
        fs::remove_file(config.root.join("deleted.rs")).unwrap();
        fs::write(config.root.join("added.rs"), "addedmarker").unwrap();
        let index = TantivyIndex::open_or_build(config.clone()).await.unwrap();
        assert!(
            index
                .search("oldmarker OR deletedmarker", 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            index.search("newmarker", 10).await.unwrap(),
            vec![PathBuf::from("changed.rs")]
        );
        assert_eq!(
            index.search("addedmarker", 10).await.unwrap(),
            vec![PathBuf::from("added.rs")]
        );
        drop(index);
        let index = TantivyIndex::open_or_build(config).await.unwrap();
        assert_eq!(
            index.search("newmarker", 10).await.unwrap(),
            vec![PathBuf::from("changed.rs")]
        );
    });
}

#[test]
fn empty_queries_and_zero_limit_return_no_hits() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let config = config(&dir);
        fs::write(config.root.join("hello.rs"), "freshmarker").unwrap();
        let index = TantivyIndex::open_or_build(config).await.unwrap();
        for (query, limit) in [
            ("", 10),
            ("  \n\t", 10),
            ("freshmarker", 0),
            ("\"", 0),
            ("", 0),
        ] {
            assert!(index.search(query, limit).await.unwrap().is_empty());
        }
    });
}

#[test]
fn index_directory_is_pruned_even_without_extension_filter() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let config = config(&dir);
        fs::create_dir_all(config.index_dir.join("nested")).unwrap();
        fs::write(
            config.index_dir.join("nested/artifact.rs"),
            "artifactmarker",
        )
        .unwrap();
        fs::write(config.root.join("real.rs"), "sourcemarker").unwrap();
        let index = TantivyIndex::open_or_build(config.clone()).await.unwrap();
        assert!(index.search("artifactmarker", 10).await.unwrap().is_empty());
        assert_eq!(
            index.search("sourcemarker", 10).await.unwrap(),
            vec![PathBuf::from("real.rs")]
        );
        assert_eq!(
            index.search("*", 100).await.unwrap(),
            vec![PathBuf::from("real.rs")]
        );
        drop(index);
        let index = TantivyIndex::open_or_build(config).await.unwrap();
        assert_eq!(
            index.search("*", 100).await.unwrap(),
            vec![PathBuf::from("real.rs")]
        );
    });
}

#[test]
fn reopening_applies_extension_changes_and_can_clear_all_documents() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config(&dir);
        fs::write(config.root.join("hello.rs"), "sharedmarker").unwrap();
        fs::write(config.root.join("hello.txt"), "sharedmarker").unwrap();
        config.extensions = Some(vec!["RS".into()]);
        let index = TantivyIndex::open_or_build(config.clone()).await.unwrap();
        assert_eq!(
            index.search("sharedmarker", 10).await.unwrap(),
            vec![PathBuf::from("hello.rs")]
        );
        drop(index);
        config.extensions = Some(vec!["txt".into()]);
        let index = TantivyIndex::open_or_build(config.clone()).await.unwrap();
        assert_eq!(
            index.search("sharedmarker", 10).await.unwrap(),
            vec![PathBuf::from("hello.txt")]
        );
        drop(index);
        fs::remove_file(config.root.join("hello.txt")).unwrap();
        let index = TantivyIndex::open_or_build(config).await.unwrap();
        assert!(index.search("*", 10).await.unwrap().is_empty());
    });
}

#[cfg(unix)]
#[test]
fn symlink_files_directories_and_source_roots_are_not_indexed() {
    use std::os::unix::fs::symlink;
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config(&dir);
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret.rs"), "outsidemarker").unwrap();
        symlink(outside.join("secret.rs"), config.root.join("linked.rs")).unwrap();
        symlink(&outside, config.root.join("linked-dir")).unwrap();
        symlink(&config.root, config.root.join("cycle")).unwrap();
        let index = TantivyIndex::open_or_build(config.clone()).await.unwrap();
        assert!(index.search("*", 10).await.unwrap().is_empty());
        drop(index);
        let alias = dir.path().join("alias");
        symlink(&config.root, &alias).unwrap();
        config.root = alias;
        assert!(TantivyIndex::open_or_build(config).await.is_err());
    });
}
