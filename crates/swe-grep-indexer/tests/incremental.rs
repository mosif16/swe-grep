use std::fs;
use std::path::{Path, PathBuf};
use swe_grep_indexer::{IndexConfig, TantivyIndex};

fn config(root: &Path) -> IndexConfig {
    IndexConfig {
        root: root.to_owned(),
        index_dir: root.join("index"),
        extensions: Some(vec!["rs".into()]),
    }
}

#[cfg(unix)]
#[test]
fn warm_open_has_no_content_reads_commits_or_writer_lock() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        for i in 0..12 {
            fs::write(root.path().join(format!("file{i}.rs")), "warmmarker").unwrap();
        }
        let cold = TantivyIndex::open_or_build(config.clone()).await.unwrap();
        assert_eq!(cold.stats().source_reads, 12);
        assert_eq!(cold.stats().commits, 1);
        let index = tantivy::Index::open_in_dir(config.index_dir.join(".swe-grep-v2-0")).unwrap();
        let _held_writer = index.writer(50_000_000).unwrap();
        let warm = TantivyIndex::open_or_build(config).await.unwrap();
        assert_eq!(warm.stats().source_reads, 0);
        assert_eq!(warm.stats().hashed_files, 0);
        assert_eq!(warm.stats().indexed_documents, 0);
        assert_eq!(warm.stats().writer_acquisitions, 0);
        assert_eq!(warm.stats().commits, 0);
        assert_eq!(warm.search("warmmarker", 20).await.unwrap().len(), 12);
    });
}

#[test]
fn refresh_updates_only_changed_added_or_deleted_documents() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        for name in ["changed.rs", "deleted.rs", "stable.rs"] {
            fs::write(root.path().join(name), "oldmarker").unwrap();
        }
        drop(TantivyIndex::open_or_build(config.clone()).await.unwrap());
        fs::write(root.path().join("changed.rs"), "newmarker").unwrap();
        fs::remove_file(root.path().join("deleted.rs")).unwrap();
        fs::write(root.path().join("added.rs"), "addmarker").unwrap();
        let refreshed = TantivyIndex::open_or_build(config).await.unwrap();
        assert_eq!(
            refreshed.stats().source_reads,
            if cfg!(unix) { 2 } else { 3 }
        );
        assert_eq!(refreshed.stats().indexed_documents, 2);
        assert_eq!(refreshed.stats().deleted_documents, 2);
        assert_eq!(refreshed.stats().commits, 1);
        assert_eq!(
            refreshed.search("oldmarker", 10).await.unwrap(),
            vec![PathBuf::from("stable.rs")]
        );
        assert_eq!(
            refreshed.search("newmarker", 10).await.unwrap(),
            vec![PathBuf::from("changed.rs")]
        );
    });
}

#[cfg(unix)]
#[test]
fn restored_mtime_edits_and_metadata_only_changes_are_handled() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let path = root.path().join("source.rs");
        fs::write(&path, "oldmarker").unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        drop(TantivyIndex::open_or_build(config.clone()).await.unwrap());
        fs::write(&path, "newmarker").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        let changed = TantivyIndex::open_or_build(config.clone()).await.unwrap();
        assert!(changed.search("oldmarker", 10).await.unwrap().is_empty());
        assert_eq!(changed.search("newmarker", 10).await.unwrap().len(), 1);
        drop(changed);
        fs::write(&path, "newmarker").unwrap();
        let touched = TantivyIndex::open_or_build(config.clone()).await.unwrap();
        assert_eq!(touched.stats().source_reads, 1);
        assert_eq!(touched.stats().indexed_documents, 0);
        assert_eq!(touched.stats().deleted_documents, 0);
        drop(touched);
        let warm = TantivyIndex::open_or_build(config).await.unwrap();
        assert_eq!(warm.stats().commits, 0);
    });
}

#[test]
fn corrupt_manifest_is_rebuilt_without_duplicate_documents() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        fs::write(root.path().join("source.rs"), "recovermarker").unwrap();
        drop(TantivyIndex::open_or_build(config.clone()).await.unwrap());
        let index = tantivy::Index::open_in_dir(config.index_dir.join(".swe-grep-v2-0")).unwrap();
        let mut writer = index.writer(50_000_000).unwrap();
        let mut prepared = writer.prepare_commit().unwrap();
        prepared.set_payload("malformed manifest");
        prepared.commit().unwrap();
        drop(writer);
        drop(index);
        let rebuilt = TantivyIndex::open_or_build(config).await.unwrap();
        assert_eq!(rebuilt.stats().commits, 1);
        assert_eq!(
            rebuilt.search("recovermarker", 10).await.unwrap(),
            vec![PathBuf::from("source.rs")]
        );
    });
}

#[cfg(unix)]
#[test]
fn concurrent_warm_readers_never_rewrite_the_index() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        fs::write(root.path().join("source.rs"), "parallelmarker").unwrap();
        drop(TantivyIndex::open_or_build(config.clone()).await.unwrap());
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..3)
                .map(|_| {
                    let config = config.clone();
                    scope.spawn(move || {
                        tokio::runtime::Runtime::new().unwrap().block_on(async {
                            let index = TantivyIndex::open_or_build(config).await.unwrap();
                            assert_eq!(index.stats().commits, 0);
                            assert_eq!(index.stats().writer_acquisitions, 0);
                            assert_eq!(index.search("parallelmarker", 10).await.unwrap().len(), 1);
                        });
                    })
                })
                .collect();
            for handle in handles {
                handle.join().unwrap();
            }
        });
    });
}
