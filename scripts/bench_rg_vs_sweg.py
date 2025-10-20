#!/usr/bin/env python3
import argparse
import json
import statistics
import subprocess
import time
from pathlib import Path

DEFAULT_RUNS = 10


def run_once(cmd, cwd):
    start = time.perf_counter()
    completed = subprocess.run(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    duration_ms = (time.perf_counter() - start) * 1000
    return completed.returncode, duration_ms

def measure_tool(cmd, cwd, runs):
    times = []
    for _ in range(runs):
        code, duration = run_once(cmd, cwd)
        if code != 0:
            raise RuntimeError(f"Command {' '.join(cmd)} failed with code {code}")
        times.append(duration)
    return {
        "runs": runs,
        "times_ms": times,
        "mean_ms": statistics.mean(times),
        "p95_ms": statistics.quantiles(times, n=20)[18] if runs >= 20 else max(times),
        "min_ms": min(times),
        "max_ms": max(times),
    }

def run_benchmark(repo, symbol, swegrep_bin, runs):
    repo_path = Path(repo).resolve()
    sweg_cmd = [str(swegrep_bin), "search", "--symbol", symbol, "--path", str(repo_path)]
    rg_cmd = ["rg", symbol]

    return {
        "symbol": symbol,
        "repository": str(repo_path),
        "runs": runs,
        "rg": measure_tool(rg_cmd, repo_path, runs),
        "swe_grep": measure_tool(sweg_cmd, Path.cwd(), runs),
    }

def main():
    parser = argparse.ArgumentParser(description="Benchmark swe-grep vs rg")
    parser.add_argument("--repo", required=True, help="Repository root to search")
    parser.add_argument("--symbol", required=True, help="Symbol to search")
    parser.add_argument("--runs", type=int, default=DEFAULT_RUNS, help="Number of warm runs")
    parser.add_argument("--swegrep-bin", default="target/debug/swe-grep", help="Path to swe-grep binary")
    parser.add_argument("--output", help="Optional JSON output file")
    args = parser.parse_args()

    result = run_benchmark(args.repo, args.symbol, Path(args.swegrep_bin), args.runs)
    output = json.dumps(result, indent=2)
    if args.output:
        Path(args.output).write_text(output)
    else:
        print(output)

if __name__ == "__main__":
    main()
