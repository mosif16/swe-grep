#!/usr/bin/env python3
"""Opt-in, repository-local Cargo artifact maintenance (Python 3.11+)."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shlex
import signal
import stat
import subprocess
import sys
import time
import tomllib


REPOSITORY = Path(__file__).resolve().parent.parent
PACKAGE_NAMES = ("swe-grep", "swe-grep-indexer", "swe-grep-desktop")
PROFILE_DIRECTORIES = {"dev": "debug", "release": "release"}


class CleanupError(Exception):
    pass


def arguments(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Preview Cargo cleanup for this repository's workspace packages. "
        "Defaults to the dev profile and never removes anything without --apply. "
        "Unselected dependency artifacts and the other profile are retained."
    )
    parser.add_argument("--apply", action="store_true", help="Actually clean the selected package artifacts")
    parser.add_argument("--profile", choices=PROFILE_DIRECTORIES, default="dev", help="Profile to clean (default: dev; preserves release)")
    parser.add_argument("--package", choices=PACKAGE_NAMES, action="append", help="Workspace member to clean; repeatable (default: all three members)")
    parser.add_argument("--timeout-secs", type=float, default=30.0, help="Maximum Cargo runtime, including waiting for build locks (default: 30)")
    parser.add_argument("--verbose", action="store_true", help="Include Cargo's affected-file listing")
    parser.add_argument("--json", action="store_true", help="Return inventory, command and results as JSON")
    parsed = parser.parse_args(argv)
    if not 0 < parsed.timeout_secs <= 3600:
        parser.error("--timeout-secs must be greater than zero and at most 3600")
    return parsed


def repository_paths(root: Path, profile: str) -> tuple[Path, Path]:
    root = root.resolve(strict=True)
    manifest = root / "Cargo.toml"
    if manifest.is_symlink() or not manifest.is_file():
        raise CleanupError("Repository Cargo.toml must be a regular, non-symlink file")
    target = root / "target"
    selected = target / PROFILE_DIRECTORIES[profile]
    # Refuse redirection before invoking Cargo or traversing the directory.
    for location in (target, selected):
        if location.is_symlink():
            raise CleanupError(f"Refusing symlinked build directory: {location}")
        if location.exists() and not location.is_dir():
            raise CleanupError(f"Expected a build directory: {location}")
        if location.resolve() != location:
            raise CleanupError(f"Build directory escapes the repository: {location}")
    for name in (".cargo-lock", ".cargo-build-lock"):
        lock = selected / name
        if lock.is_symlink() or (lock.exists() and not lock.is_file()):
            raise CleanupError(f"Refusing unsafe Cargo lock path: {lock}")
    return target, selected


def workspace_packages(root: Path) -> set[str]:
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    result = set()
    for member in manifest.get("workspace", {}).get("members", []):
        # This repository has explicit member paths; reject globs and redirection.
        relative = Path(member)
        if relative.is_absolute() or ".." in relative.parts or any(char in member for char in "*?["):
            raise CleanupError("Workspace members must be explicit repository-relative paths")
        location = root / relative / "Cargo.toml"
        if location.is_symlink() or not location.resolve().is_relative_to(root):
            raise CleanupError(f"Workspace member escapes the repository: {location}")
        result.add(tomllib.loads(location.read_text())["package"]["name"])
    return result


def inventory(directory: Path) -> dict:
    """Estimate local file bytes without following symlinks or counting hardlinks twice."""
    totals = {"files": 0, "logical_bytes": 0, "allocated_file_bytes": 0, "symlinks_skipped": 0}
    if not directory.exists():
        return totals
    seen = set()
    for parent, directories, files in os.walk(directory, followlinks=False):
        for name in list(directories):
            if (Path(parent) / name).is_symlink():
                directories.remove(name)
                totals["symlinks_skipped"] += 1
        for name in files:
            try:
                info = (Path(parent) / name).lstat()
            except FileNotFoundError:
                continue  # A concurrent build may replace an artifact during inventory.
            if stat.S_ISLNK(info.st_mode):
                totals["symlinks_skipped"] += 1
                continue
            if not stat.S_ISREG(info.st_mode) or (info.st_dev, info.st_ino) in seen:
                continue
            seen.add((info.st_dev, info.st_ino))
            totals["files"] += 1
            totals["logical_bytes"] += info.st_size
            totals["allocated_file_bytes"] += getattr(info, "st_blocks", (info.st_size + 511) // 512) * 512
    return totals


def cargo_command(root: Path, options: argparse.Namespace) -> tuple[list[str], dict[str, str]]:
    target, _ = repository_paths(root, options.profile)
    requested = list(dict.fromkeys(options.package or PACKAGE_NAMES))
    members = workspace_packages(root)
    if not requested or any(name not in PACKAGE_NAMES or name not in members for name in requested):
        raise CleanupError("Only this repository's known workspace members can be cleaned")
    command = [
        "cargo", "clean", "--manifest-path", str(root / "Cargo.toml"),
        "--target-dir", str(target), "--profile", options.profile, "--frozen",
        # CLI overrides and environment cleanup keep ambient routing local. The
        # build-dir setting is honored by Cargo versions supporting split dirs;
        # older Cargo uses target-dir for intermediate artifacts too.
        "--config", "build.target-dir=" + json.dumps(str(target)),
        "--config", "build.build-dir=" + json.dumps(str(target)),
        "--config", "build.target=[]",
    ]
    # Always select packages. Cargo 1.91 whole-profile cleanup does not acquire
    # the build lock, whereas package cleanup does and retains the lock inode.
    for name in requested:
        command.extend(("--package", name))
    if not options.apply:
        command.append("--dry-run")
    if options.verbose:
        command.append("--verbose")
    environment = dict(os.environ)
    for name in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "CARGO_BUILD_BUILD_DIR", "CARGO_BUILD_TARGET"):
        environment.pop(name, None)
    environment["CARGO_TERM_COLOR"] = "never"
    return command, environment


def run_cargo(command: list[str], environment: dict[str, str], root: Path, timeout: float) -> dict:
    if os.name != "posix":
        raise CleanupError("This maintenance helper currently supports macOS/Linux only")
    start = time.monotonic()
    process = subprocess.Popen(command, cwd=root, env=environment, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True, start_new_session=True)

    def stop():
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            return process.communicate(timeout=2)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            return process.communicate()

    timed_out = False
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        stdout, stderr = stop()
    except BaseException:
        stop()  # Reap the separate process group on cancellation too.
        raise
    return {"exit_code": process.returncode, "timed_out": timed_out,
            "elapsed_seconds": time.monotonic() - start, "stdout": stdout, "stderr": stderr}


def clean(root: Path, options: argparse.Namespace) -> tuple[dict, int]:
    root = root.resolve(strict=True)
    target, selected = repository_paths(root, options.profile)
    command, environment = cargo_command(root, options)
    report = {
        "mode": "apply" if options.apply else "dry_run",
        "repository": str(root), "target": str(target), "profile": options.profile,
        "packages": list(dict.fromkeys(options.package or PACKAGE_NAMES)),
        "command": command,
        "target_before": inventory(target), "profile_before": inventory(selected),
        "retained": "Unselected dependency artifacts, the other profile, Cargo home and other repositories are retained.",
    }
    if not selected.exists():
        report["status"] = "nothing_to_clean"
        return report, 0
    # Revalidate after inventory in case a directory was replaced while scanning.
    repository_paths(root, options.profile)
    result = run_cargo(command, environment, root, options.timeout_secs)
    report["cargo"] = result
    report["status"] = "timed_out" if result["timed_out"] else "complete" if result["exit_code"] == 0 else "failed"
    if options.apply:
        report["target_after"] = inventory(target)
        report["profile_after"] = inventory(selected)
        report["allocated_file_bytes_reduction"] = report["target_before"]["allocated_file_bytes"] - report["target_after"]["allocated_file_bytes"]
    return report, 1 if result["timed_out"] or result["exit_code"] != 0 else 0


def main(argv: list[str] | None = None) -> int:
    options = arguments(argv)
    try:
        report, code = clean(REPOSITORY, options)
    except (CleanupError, OSError, ValueError, KeyError, TypeError) as error:
        if options.json:
            print(json.dumps({"status": "refused", "error": str(error)}))
        else:
            print(f"Build-cache cleanup refused: {error}", file=sys.stderr)
        return 1
    if options.json:
        print(json.dumps(report, indent=2))
    else:
        print(f"{report['mode']}: {report['profile']} artifacts for {', '.join(report['packages'])}")
        print(f"Target: {report['target']}")
        print(f"Target allocated file bytes: {report['target_before']['allocated_file_bytes']:,}")
        print("Command: " + shlex.join(report["command"]))
        print(report["retained"])
        if "cargo" in report:
            print(report["cargo"]["stdout"], end="")
            print(report["cargo"]["stderr"], end="", file=sys.stderr)
        print("Status: " + report["status"])
        if options.apply:
            print(f"Allocated file bytes reduction: {report.get('allocated_file_bytes_reduction', 0):,}")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
