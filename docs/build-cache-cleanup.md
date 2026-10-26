# Build-artifact maintenance

[Documentation home](README.md) · [Performance and runtime caches](performance-tuning.md)

The Rust workspace's `target/` directory contains compiled binaries, test programs, dependencies and incremental compiler data. These are separate from SWE-Grep's runtime analysis/index caches. Repeated profiles and feature combinations can accumulate gigabytes of artifacts.

Preview this repository's development-package cleanup:

```sh
python3 scripts/build-cache-gc.py
python3 scripts/build-cache-gc.py --json
```

The helper requires Python 3.11+ and Cargo, and currently supports macOS/Linux. It always uses the checkout containing the script, regardless of the current working directory. Default behavior invokes Cargo's dry-run and reports an inventory; no artifacts are removed. The inventory distinguishes logical bytes from allocated file bytes and does not follow symlinks. Estimates are not a guarantee of disk space reclaimed on filesystems with snapshots, compression or shared extents.

To actually remove development artifacts for all three workspace members:

```sh
python3 scripts/build-cache-gc.py --apply
```

This retains release artifacts and compiled third-party dependencies. Development binaries, test programs and incremental data belonging to the selected workspace packages must be rebuilt when needed. There is no automatic cleanup schedule and no target-directory hard size cap.

For a narrower package or an explicitly selected release profile:

```sh
python3 scripts/build-cache-gc.py --package swe-grep
python3 scripts/build-cache-gc.py --package swe-grep --apply
python3 scripts/build-cache-gc.py --profile release --package swe-grep --apply
```

Only `swe-grep`, `swe-grep-indexer` and `swe-grep-desktop` are accepted, and their workspace membership is checked. `--package` can be repeated. No registry, Git dependency checkout, Cargo home, external target directory or other project's cache is cleaned.

The helper pins Cargo's target and intermediate build directories to this checkout's `target/`, overrides ambient target routing, and refuses symlinked target/profile directories or lock paths. Cleanup always uses Cargo's **package-specific** mode. On tested Cargo 1.91.1, that mode acquires the build-directory lock; whole-profile cleanup without `--package` does not provide the same protection. The helper deliberately offers no full-target deletion mode.

Cargo waits for an active build's lock. The helper bounds the whole Cargo operation with `--timeout-secs` (default 30) and terminates its process group on timeout. A timeout while waiting leaves artifacts untouched; a timeout after cleanup begins may leave a partial cleanup, which is reported as a failure and can be safely retried. This does not stop another running build.

There is no incremental-only age/budget pruning: Cargo's supported clean interface does not expose those selectors, and the internal build-directory layout can change. Package-specific cleanup preserves useful dependency and release caches instead of guessing which compiler internals are stale. Cargo's automatic global cache cleaning applies to Cargo home, not to this build-artifact policy. See the official [clean command](https://doc.rust-lang.org/cargo/commands/cargo-clean.html), [build-cache layout](https://doc.rust-lang.org/cargo/reference/build-cache.html), and [automatic cache policy](https://doc.rust-lang.org/cargo/reference/config.html#cache).

Run the disposable-fixture regressions:

```sh
python3 -B -m unittest discover -s scripts/tests -p test_build_cache_gc.py -v
```

The tests compile tiny local projects, then verify dry-run preservation, selective apply, dependency/release retention, build-lock timeout, redirected configuration rejection and symlink boundaries. They do not clean this checkout's live artifacts.
