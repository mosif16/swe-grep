use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn run(cwd: &Path, cache: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_swe-grep"))
        .current_dir(cwd)
        .env("XDG_CACHE_HOME", cache)
        .args(["--no-config", "--disable-telemetry", "--quiet-hint"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn garbage_collection_previews_then_applies_and_search_rebuilds() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("source");
    let cache = scratch.path().join("cache");
    fs::create_dir(&root).unwrap();
    let source = "pub fn target() {}\n";
    fs::write(root.join("lib.rs"), source).unwrap();
    let warm = || run(&root, &cache, &["context", "target", ".", "--concise"]);
    assert_eq!(warm()["resolution"]["status"], "resolved");
    let preview = run(&root, &cache, &["cache", "gc", "--max-bytes=0"]);
    assert_eq!(preview["applied"], false);
    assert!(preview["eligible_files"].as_u64().unwrap() > 0, "{preview}");
    assert_eq!(preview["removed_files"], 0);
    let applied = run(&root, &cache, &["cache", "gc", "--max-bytes=0", "--apply"]);
    assert!(applied["removed_files"].as_u64().unwrap() > 0, "{applied}");
    assert_eq!(fs::read_to_string(root.join("lib.rs")).unwrap(), source);
    assert_eq!(warm()["resolution"]["status"], "resolved");
}

#[test]
fn clear_cache_uses_positional_root_and_preserves_unknown_files() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("source");
    let cache = scratch.path().join("cache");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("lib.rs"), "pub fn target() {}\n").unwrap();
    run(&root, &cache, &["context", "target", ".", "--concise"]);
    let base = cache.join("swe-grep/managed-v1");
    let namespace = fs::read_dir(&base)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.is_dir())
        .unwrap();
    fs::write(namespace.join("notes.txt"), "keep").unwrap();
    fs::write(namespace.join("state.json"), "{\"temporary\":true}").unwrap();
    let payload = run(
        scratch.path(),
        &cache,
        &[
            "--clear-cache",
            "context",
            "target",
            root.to_str().unwrap(),
            "--concise",
        ],
    );
    assert_eq!(payload["resolution"]["status"], "resolved");
    assert!(
        !namespace.join("state.json").exists(),
        "clear must use the selected root"
    );
    assert_eq!(
        fs::read_to_string(namespace.join("notes.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn clear_cache_reports_busy_collector_instead_of_succeeding_without_a_clear() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("source");
    let cache = scratch.path().join("cache");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("lib.rs"), "pub fn target() {}\n").unwrap();
    run(&root, &cache, &["context", "target", ".", "--concise"]);
    let base = cache.join("swe-grep/managed-v1");
    let namespace = fs::read_dir(&base)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.is_dir())
        .unwrap();
    let state = namespace.join("state.json");
    fs::write(&state, "{\"sentinel\":true}").unwrap();
    let collector = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(base.join(".swe-grep-collector"))
        .unwrap();
    collector.try_lock().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_swe-grep"))
        .current_dir(&root)
        .env("XDG_CACHE_HOME", &cache)
        .args([
            "--no-config",
            "--disable-telemetry",
            "--quiet-hint",
            "--clear-cache",
            "context",
            "target",
            ".",
            "--concise",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .to_ascii_lowercase()
            .contains("collector"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(state).unwrap(), "{\"sentinel\":true}");
    collector.unlock().unwrap();
}

#[test]
fn repeated_bounded_gc_reaches_payloads_after_preserved_unknown_entries() {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("source");
    let cache = scratch.path().join("cache");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("lib.rs"), "pub fn target() {}\n").unwrap();
    run(&root, &cache, &["context", "target", ".", "--concise"]);
    let base = cache.join("swe-grep/managed-v1");
    let namespace = fs::read_dir(&base)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.is_dir())
        .unwrap();
    let facts = namespace.join("facts");
    let key = |kind: &str, number: usize| {
        let mut hash = DefaultHasher::new();
        (kind, number).hash(&mut hash);
        format!("{:016x}", hash.finish())
    };
    let unknown: Vec<_> = (0..4500)
        .map(|number| facts.join(format!("{}.keep", key("unknown", number))))
        .collect();
    for path in &unknown {
        fs::write(path, "preserved unknown payload").unwrap();
    }
    let payloads: Vec<_> = (0..100)
        .map(|number| facts.join(format!("{}.json", key("managed", number))))
        .collect();
    for path in &payloads {
        fs::write(path, "{\"synthetic_cache_payload\":true}").unwrap();
    }
    let mut observed_bound = false;
    for _ in 0..5 {
        let report = run(&root, &cache, &["cache", "gc", "--max-bytes=0", "--apply"]);
        observed_bound |= report["scan_limited"] == true;
        if payloads.iter().all(|path| !path.exists()) {
            break;
        }
    }
    assert!(observed_bound, "fixture must exercise a bounded scan");
    assert!(
        payloads.iter().all(|path| !path.exists()),
        "repeated bounded sweeps must eventually reach every owned payload"
    );
    assert!(unknown.iter().all(|path| path.exists()));
    for path in [unknown.first().unwrap(), unknown.last().unwrap()] {
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "preserved unknown payload"
        );
    }
}
