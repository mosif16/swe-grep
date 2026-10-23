use std::fs;
use std::path::PathBuf;

use swe_grep_indexer::{IndexConfig, TantivyIndex};

#[test]
fn relative_root_returns_paths_relative_to_root() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let cwd = std::env::current_dir().unwrap();
        let dir = tempfile::tempdir_in(&cwd).unwrap();
        let root = dir.path().strip_prefix(&cwd).unwrap().to_path_buf();
        fs::write(root.join("hello.rs"), "relativemarker").unwrap();
        let index = TantivyIndex::open_or_build(IndexConfig {
            index_dir: root.join("search-index"),
            root,
            extensions: None,
        })
        .await
        .unwrap();
        assert_eq!(
            index.search("relativemarker", 10).await.unwrap(),
            vec![PathBuf::from("hello.rs")]
        );
    });
}
