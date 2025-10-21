#!/usr/bin/env python3
"""Measure swe-grep cold start and per-stage latency characteristics.

This benchmark script captures process startup time, time-to-first-output,
and the structured metrics exposed in the swe-grep JSON summary. Use it to
track cold-start regressions alongside the existing throughput/latency suites.
"""

import argparse
import json
import statistics
import subprocess
import sys
import time
from collections import defaultdict
from pathlib import Path
from typing import Dict, List, Optional


def _aggregate(values: List[float]) -> Dict[str, float]:
    if not values:
        return {"runs": 0, "mean_ms": 0.0, "min_ms": 0.0, "max_ms": 0.0, "p95_ms": 0.0}
    summary = {
        "runs": len(values),
        "mean_ms": statistics.mean(values),
        "min_ms": min(values),
        "max_ms": max(values),
    }
    if len(values) >= 20:
        summary["p95_ms"] = statistics.quantiles(values, n=20)[18]
    else:
        summary["p95_ms"] = max(values)
    return summary


def _run_once(cmd: List[str], cwd: Path) -> Dict[str, object]:
    start = time.perf_counter()
    proc = subprocess.Popen(
        cmd,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    stdout_lines: List[str] = []
    first_output: Optional[float] = None

    assert proc.stdout is not None  # for type checkers
    while True:
        line = proc.stdout.readline()
        if line:
            if first_output is None:
                first_output = time.perf_counter()
            stdout_lines.append(line)
        else:
            break

    raw_stdout = "".join(stdout_lines)
    stderr = "" if proc.stderr is None else proc.stderr.read()
    rc = proc.wait()
    duration_ms = (time.perf_counter() - start) * 1000.0
    first_output_ms = (
        (first_output - start) * 1000.0 if first_output is not None else duration_ms
    )

    if rc != 0:
        raise RuntimeError(
            f"Command {' '.join(cmd)} failed with code {rc}:\nSTDERR:\n{stderr}\nSTDOUT:\n{raw_stdout}"
        )

    summary_start: Optional[int] = None
    for idx, line in enumerate(stdout_lines):
        if '"cycle"' in line:
            summary_start = max(0, idx - 1)
            break

    if summary_start is None:
        raise RuntimeError(
            f"No JSON output captured from swe-grep. STDERR:\n{stderr}\nSTDOUT:\n{raw_stdout}"
        )

    try:
        summary_blob = "".join(stdout_lines[summary_start:])
        summary = json.loads(summary_blob)
    except json.JSONDecodeError as err:
        raise RuntimeError(
            f"Failed to parse swe-grep output as JSON: {err}\nOutput:\n{raw_stdout}\nSTDERR:\n{stderr}"
        ) from err

    stage_stats = summary.get("stage_stats", {})
    startup_stats = summary.get("startup_stats", {}) or {}

    return {
        "duration_ms": duration_ms,
        "time_to_first_output_ms": first_output_ms,
        "stage_stats": stage_stats,
        "startup_stats": startup_stats,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Benchmark swe-grep cold start behaviour")
    parser.add_argument("--repo", required=True, help="Repository root to search")
    parser.add_argument("--symbol", required=True, help="Symbol for the benchmark query")
    parser.add_argument("--runs", type=int, default=5, help="Number of iterations to run")
    parser.add_argument(
        "--language",
        help="Optional language hint to pass to swe-grep",
    )
    parser.add_argument(
        "--swegrep-bin",
        default="target/debug/swe-grep",
        help="Path to the swe-grep binary (default: target/debug/swe-grep)",
    )
    parser.add_argument(
        "--timeout-secs",
        type=int,
        default=3,
        help="Per-run timeout passed through to swe-grep",
    )
    parser.add_argument(
        "--output",
        help="Optional file to write JSON results to instead of stdout",
    )
    args = parser.parse_args()

    repo_path = Path(args.repo).resolve()
    swegrep_bin = Path(args.swegrep_bin).resolve()
    if not swegrep_bin.exists():
        parser.error(f"swe-grep binary not found at {swegrep_bin}")

    run_cmd = [
        str(swegrep_bin),
        "search",
        "--symbol",
        args.symbol,
        "--path",
        str(repo_path),
        "--timeout-secs",
        str(args.timeout_secs),
    ]
    if args.language:
        run_cmd.extend(["--language", args.language])

    runs: List[Dict[str, object]] = []
    for _ in range(max(1, args.runs)):
        runs.append(_run_once(run_cmd, repo_path))

    duration_stats = _aggregate([run["duration_ms"] for run in runs])
    first_output_stats = _aggregate([run["time_to_first_output_ms"] for run in runs])

    stage_totals: Dict[str, List[float]] = defaultdict(list)
    startup_totals: Dict[str, List[float]] = defaultdict(list)

    for run in runs:
        for key, value in run["stage_stats"].items():
            if isinstance(value, (int, float)):
                stage_totals[key].append(float(value))
        for key, value in run["startup_stats"].items():
            if isinstance(value, (int, float)):
                startup_totals[key].append(float(value))

    stage_summary = {key: _aggregate(values) for key, values in stage_totals.items()}
    startup_summary = {key: _aggregate(values) for key, values in startup_totals.items()}

    result = {
        "symbol": args.symbol,
        "repository": str(repo_path),
        "runs": len(runs),
        "command": run_cmd,
        "process_duration_ms": duration_stats,
        "time_to_first_output_ms": first_output_stats,
        "stage_stats": stage_summary,
        "startup_stats": startup_summary,
    }

    payload = json.dumps(result, indent=2)
    if args.output:
        Path(args.output).write_text(payload)
    else:
        sys.stdout.write(payload + "\n")


if __name__ == "__main__":
    main()
