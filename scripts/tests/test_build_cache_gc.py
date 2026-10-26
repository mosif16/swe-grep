"""Safe cleanup regressions using disposable, dependency-free Cargo projects."""

import fcntl
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
SCRIPT = Path(__file__).resolve().parents[1] / "build-cache-gc.py"
SPEC = importlib.util.spec_from_file_location("build_cache_gc", SCRIPT)
GC = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GC)


@unittest.skipUnless(shutil.which("cargo"), "Cargo is required for real cleanup/lock tests")
class CargoCleanupTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="swe-build-cache-gc-test-")
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name).resolve() / "workspace"
        (self.root / "core/src").mkdir(parents=True)
        (self.root / "dependency/src").mkdir(parents=True)
        (self.root / "Cargo.toml").write_text('[workspace]\nmembers=["core", "dependency"]\nresolver="2"\n')
        (self.root / "core/Cargo.toml").write_text('[package]\nname="swe-grep"\nversion="0.1.0"\nedition="2021"\n[dependencies]\nfixture-dependency={path="../dependency"}\n')
        (self.root / "core/src/main.rs").write_text('fn main() { println!("{}", fixture_dependency::value()); }\n')
        (self.root / "dependency/Cargo.toml").write_text('[package]\nname="fixture-dependency"\nversion="0.1.0"\nedition="2021"\n')
        (self.root / "dependency/src/lib.rs").write_text('pub fn value() -> u32 { 42 }\n')
        for profile in ([], ["--release"]):
            subprocess.run(["cargo", "build", "--offline", "--target-dir", str(self.root / "target"), *profile],
                           cwd=self.root, capture_output=True, text=True, check=True)
        self.binary = self.root / "target/debug/swe-grep"
        self.release = self.root / "target/release/swe-grep"
        self.dependencies = list((self.root / "target/debug/deps").glob("libfixture_dependency*.rlib"))
        self.assertTrue(self.binary.is_file())
        self.assertTrue(self.dependencies)

    def options(self, *extra):
        return GC.arguments(["--package", "swe-grep", "--timeout-secs", "5", *extra])

    def test_default_dry_run_preserves_binaries_and_dependencies(self):
        report, code = GC.clean(self.root, self.options())
        self.assertEqual(code, 0, report)
        self.assertEqual(report["mode"], "dry_run")
        self.assertIn("--dry-run", report["command"])
        self.assertTrue(self.binary.exists())
        self.assertTrue(self.release.exists())
        self.assertTrue(all(location.exists() for location in self.dependencies))

    def test_apply_removes_selected_package_and_preserves_dependency_and_release(self):
        report, code = GC.clean(self.root, self.options("--apply"))
        self.assertEqual(code, 0, report)
        self.assertFalse(self.binary.exists())
        self.assertTrue(self.release.exists())
        self.assertTrue(all(location.exists() for location in self.dependencies))
        self.assertGreater(report["allocated_file_bytes_reduction"], 0)
        repeat, code = GC.clean(self.root, self.options("--apply"))
        self.assertEqual(code, 0, repeat)

    def test_busy_cargo_lock_times_out_before_cleaning(self):
        with (self.root / "target/debug/.cargo-lock").open("a+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            report, code = GC.clean(self.root, self.options("--apply", "--timeout-secs", "0.2"))
            self.assertEqual(code, 1, report)
            self.assertEqual(report["status"], "timed_out")
            self.assertIn("file lock", report["cargo"]["stderr"])
            self.assertTrue(self.binary.exists())
            self.assertTrue(all(location.exists() for location in self.dependencies))

    def test_environment_and_config_cannot_redirect_cleanup(self):
        outside = Path(self.scratch.name) / "unrelated"
        outside.mkdir()
        sentinel = outside / "keep-me"
        sentinel.write_text("other project")
        (self.root / ".cargo").mkdir()
        (self.root / ".cargo/config.toml").write_text(f'[build]\ntarget-dir="{outside}"\nbuild-dir="{outside}"\n')
        with patch.dict(os.environ, {"CARGO_TARGET_DIR": str(outside), "CARGO_BUILD_BUILD_DIR": str(outside)}):
            report, code = GC.clean(self.root, self.options("--apply"))
        self.assertEqual(code, 0, report)
        self.assertFalse(self.binary.exists())
        self.assertEqual(sentinel.read_text(), "other project")
        self.assertTrue(all(location.exists() for location in self.dependencies))

    def test_symlinked_profile_is_refused(self):
        selected = self.root / "target/debug"
        outside = Path(self.scratch.name) / "moved-debug"
        selected.rename(outside)
        selected.symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(GC.CleanupError, "symlink"):
            GC.clean(self.root, self.options("--apply"))
        self.assertTrue((outside / "swe-grep").exists())

    def test_target_and_lock_symlinks_are_refused(self):
        target = self.root / "target"
        outside = Path(self.scratch.name) / "moved-target"
        target.rename(outside)
        target.symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(GC.CleanupError, "symlink"):
            GC.clean(self.root, self.options("--apply"))
        target.unlink()
        outside.rename(target)
        lock = target / "debug/.cargo-lock"
        lock.unlink()
        lock.symlink_to(self.root / "Cargo.toml")
        with self.assertRaisesRegex(GC.CleanupError, "lock"):
            GC.clean(self.root, self.options("--apply"))

    def test_non_workspace_package_is_refused(self):
        options = self.options()
        options.package = ["fixture-dependency"]
        with self.assertRaisesRegex(GC.CleanupError, "workspace members"):
            GC.cargo_command(self.root, options)


if __name__ == "__main__":
    unittest.main()
