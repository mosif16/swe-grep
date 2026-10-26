use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant, SystemTime};

use swe_grep::cache_gc::{CacheLease, GcPolicy, GcReport, collect};

#[test]
fn lease_child() {
    let Ok(mode) = std::env::var("SWE_GC_CHILD") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("SWE_GC_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    let namespace = swe_grep::search::default_cache_dir_for_root(&root);
    // Registration must not require an exclusive activity lock. A reader may
    // already hold the activity inode when a stale initializer reaches it.
    let _existing_reader = if mode == "shared_before_registration" {
        fs::create_dir_all(&namespace).unwrap();
        let reader = fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(namespace.join(".swe-grep-lease"))
            .unwrap();
        reader.lock_shared().unwrap();
        Some(reader)
    } else {
        None
    };
    if mode == "concurrent" {
        for round in 0..16 {
            let round_root = root.join(format!("round-{round}"));
            fs::create_dir(&round_root).unwrap();
            let round_namespace = swe_grep::search::default_cache_dir_for_root(&round_root);
            let start = Arc::new(Barrier::new(8));
            let acquired = Arc::new(Barrier::new(8));
            std::thread::scope(|scope| {
                for _ in 0..8 {
                    let (start, acquired, root, namespace) = (
                        start.clone(),
                        acquired.clone(),
                        &round_root,
                        &round_namespace,
                    );
                    scope.spawn(move || {
                        start.wait();
                        let lease = CacheLease::acquire(root, namespace);
                        acquired.wait();
                        let _lease = lease.unwrap();
                    });
                }
            });
        }
    }
    let _lease = CacheLease::acquire(&root, &namespace).unwrap();
    println!("GC_READY={}", serde_json::to_string(&namespace).unwrap());
    std::io::stdout().flush().unwrap();
    if mode == "hold" {
        let mut byte = [0];
        std::io::stdin().read_exact(&mut byte).unwrap();
    }
}

fn spawn(root: &Path, home: &Path, mode: &str) -> (Child, PathBuf) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lease_child", "--nocapture"])
        .env("SWE_GC_CHILD", mode)
        .env("SWE_GC_ROOT", root)
        .env("XDG_CACHE_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = sender.send(None);
                    return;
                }
                Ok(_) => {
                    if let Some((_, json)) = line.split_once("GC_READY=") {
                        let path = serde_json::from_str::<PathBuf>(json.trim()).unwrap();
                        let _ = sender.send(Some((path, reader.into_inner())));
                        return;
                    }
                }
            }
        }
    });
    let ready = receiver.recv_timeout(Duration::from_secs(10));
    let (path, stdout) = match ready {
        Ok(Some(ready)) => ready,
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("lease child failed or did not register within 10 seconds");
        }
    };
    // Keep the pipe open until the child harness has printed its final result.
    child.stdout = Some(stdout);
    (child, path)
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    namespace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        let home = temp.path().join("cache");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&home).unwrap();
        fs::write(root.join("app.rs"), "fn target() {}\n").unwrap();
        let (mut child, namespace) = spawn(&root, &home, "once");
        assert!(child.wait().unwrap().success());
        Self {
            _temp: temp,
            root,
            home,
            namespace,
        }
    }

    fn base(&self) -> &Path {
        self.namespace.parent().unwrap()
    }
    fn state(&self) -> PathBuf {
        self.namespace.join("state.json")
    }
}

fn all() -> GcPolicy {
    GcPolicy {
        max_age: Duration::ZERO,
        max_bytes: 0,
        ..GcPolicy::default()
    }
}

fn snapshot(path: &Path) -> Vec<(PathBuf, Vec<u8>, SystemTime)> {
    let mut files = vec![];
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(snapshot(&path));
        } else {
            files.push((
                path.clone(),
                fs::read(&path).unwrap(),
                fs::metadata(&path).unwrap().modified().unwrap(),
            ));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

#[test]
fn dry_run_preserves_data_and_controls_apply_removes_only_owned_payloads() {
    let fixture = Fixture::new();
    fs::write(fixture.state(), "1234").unwrap();
    fs::create_dir(fixture.namespace.join("facts")).unwrap();
    fs::write(
        fixture.namespace.join("facts/0123456789abcdef.json"),
        "facts",
    )
    .unwrap();
    fs::write(
        fixture.namespace.join("facts/notes.json"),
        "private user data",
    )
    .unwrap();
    fs::create_dir(fixture.namespace.join("graphs")).unwrap();
    fs::write(
        fixture.namespace.join("graphs/analysis_graphs_v3.bin"),
        "graphs",
    )
    .unwrap();
    fs::write(fixture.namespace.join("keep.txt"), "keep").unwrap();
    let before = snapshot(fixture.base());
    let preview = collect(fixture.base(), &all(), false).unwrap();
    assert_eq!(preview.eligible_files, 3);
    assert_eq!(preview.removed_files, 0);
    assert_eq!(snapshot(fixture.base()), before);
    let report = collect(fixture.base(), &all(), true).unwrap();
    assert_eq!(report.removed_files, 3);
    assert!(fixture.namespace.join("facts/notes.json").exists());
    assert!(fixture.namespace.join("keep.txt").exists());
    for name in [
        ".swe-grep-owner.json",
        ".swe-grep-lease",
        ".swe-grep-last-used",
    ] {
        assert!(fixture.namespace.join(name).exists());
    }
    assert!(fixture.root.join("app.rs").exists());
}

#[test]
fn process_shared_lease_prevents_eviction_without_consuming_file_scan_budget() {
    let fixture = Fixture::new();
    fs::write(fixture.state(), "active").unwrap();
    let (mut child, _) = spawn(&fixture.root, &fixture.home, "hold");
    let report = collect(fixture.base(), &all(), true).unwrap();
    assert_eq!(report.skipped_active, 1);
    assert!(report.active_bytes_unmeasured);
    assert_eq!(report.scanned_files, 0);
    assert!(fixture.state().exists());
    child.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(
        collect(fixture.base(), &all(), true).unwrap().removed_files,
        1
    );
}

#[test]
fn age_and_size_policies_use_last_use_not_payload_write_time() {
    let fixture = Fixture::new();
    fs::write(fixture.state(), "1234").unwrap();
    let mut policy = GcPolicy {
        max_bytes: u64::MAX,
        ..GcPolicy::default()
    };
    let fresh = preview_quiescent_cache(fixture.base(), &policy);
    assert_eq!(fresh.eligible_files, 0, "fresh collection: {fresh:#?}");
    let used = fixture.namespace.join(".swe-grep-last-used");
    fs::File::options()
        .write(true)
        .open(&used)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(31 * 86400))
        .unwrap();
    let aged = preview_quiescent_cache(fixture.base(), &policy);
    assert_eq!(aged.eligible_files, 1, "aged collection: {aged:#?}");
    fs::File::options()
        .write(true)
        .open(&used)
        .unwrap()
        .set_modified(SystemTime::now())
        .unwrap();
    policy.max_bytes = 1;
    let report = preview_quiescent_cache(fixture.base(), &policy);
    assert_eq!(
        report.entries.len(),
        1,
        "size collection report: {report:#?}"
    );
    assert_eq!(report.entries.first().unwrap().reason, "size");
}

fn preview_quiescent_cache(base: &Path, policy: &GcPolicy) -> GcReport {
    // Collection deliberately skips busy locks. Other tests spawn subprocesses
    // concurrently; a fork can briefly inherit a flock descriptor until exec
    // closes it, even after this process has finished its previous dry run.
    // Retry only explicit contention, never missing/unknown/policy results.
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let report = collect(base, policy, false).unwrap();
        if !report.collector_busy && report.skipped_active == 0 {
            return report;
        }
        assert!(
            Instant::now() < deadline,
            "cache remained busy: {report:#?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn unmarked_legacy_and_custom_directories_are_preserved() {
    let fixture = Fixture::new();
    let unknown = fixture.base().join("aaaaaaaaaaaaaaaa");
    fs::create_dir(&unknown).unwrap();
    fs::write(unknown.join("state.json"), "unowned").unwrap();
    let legacy = fixture.base().parent().unwrap().join("bbbbbbbbbbbbbbbb");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("state.json"), "legacy").unwrap();
    let custom = fixture.root.join("custom-cache");
    let _lease = CacheLease::acquire(&fixture.root, &custom).unwrap();
    assert!(
        !custom.exists(),
        "a custom no-op lease must not create files"
    );
    collect(fixture.base(), &all(), true).unwrap();
    assert!(unknown.join("state.json").exists());
    assert!(legacy.join("state.json").exists());
    assert!(
        CacheLease::acquire(&fixture.root, &fixture.namespace).is_err(),
        "explicit custom path into an owned namespace must not bypass leasing"
    );
    assert!(CacheLease::acquire(&fixture.root, &fixture.namespace.join("nested")).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_payloads_subdirectories_and_namespaces_never_touch_targets() {
    let fixture = Fixture::new();
    let outside = fixture.root.join("precious");
    fs::write(&outside, "source").unwrap();
    std::os::unix::fs::symlink(&outside, fixture.state()).unwrap();
    std::os::unix::fs::symlink(&fixture.root, fixture.namespace.join("facts")).unwrap();
    std::os::unix::fs::symlink(&fixture.root, fixture.base().join("cccccccccccccccc")).unwrap();
    let report = collect(fixture.base(), &all(), true).unwrap();
    assert_eq!(report.removed_files, 0);
    assert_eq!(report.skipped_symlinks, 3);
    assert_eq!(fs::read_to_string(outside).unwrap(), "source");
}

#[test]
fn simultaneous_first_acquires_share_one_valid_namespace() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    let home = temp.path().join("cache");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&home).unwrap();
    let (mut child, namespace) = spawn(&root, &home, "concurrent");
    assert!(child.wait().unwrap().success());
    assert!(namespace.join(".swe-grep-owner.json").exists());
}

#[test]
fn registration_does_not_request_exclusive_activity_access() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    let home = temp.path().join("cache");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&home).unwrap();
    let (mut child, namespace) = spawn(&root, &home, "shared_before_registration");
    assert!(child.wait().unwrap().success());
    assert!(namespace.join(".swe-grep-owner.json").exists());
    assert!(namespace.join(".swe-grep-init").exists());
}

#[test]
fn finishing_the_last_process_lease_reclaims_an_oversized_project_cache() {
    let fixture = Fixture::new();
    let (mut child, _) = spawn(&fixture.root, &fixture.home, "hold");
    let payload = fs::File::create(fixture.state()).unwrap();
    payload.set_len(513 * 1024 * 1024).unwrap();
    drop(payload);
    // Make the hourly collector due without reading or allocating the sparse payload.
    fs::File::options()
        .write(true)
        .open(fixture.base().join(".swe-grep-last-gc"))
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(7200))
        .unwrap();
    assert!(
        fixture.state().exists(),
        "an active process must retain its payload"
    );
    child.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(child.wait().unwrap().success());
    assert!(
        !fixture.state().exists(),
        "the idle namespace should participate in automatic size collection"
    );
    assert!(fixture.root.join("app.rs").exists());
}
