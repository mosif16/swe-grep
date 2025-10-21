#!/usr/bin/env python3
"""Simple regression guard for swe-grep benchmark summaries.

Reads the latest entry from docs/benchmark-summary.jsonl (or a custom path)
and verifies success rates and latency budgets stay within configured bounds.
Exits with status 1 when a guard fails so CI can catch regressions.
"""

import argparse
import json
import sys
from pathlib import Path


def load_latest(summary_path: Path):
    if not summary_path.exists():
        raise FileNotFoundError(f"summary file not found: {summary_path}")
    lines = summary_path.read_text().strip().splitlines()
    if not lines:
        raise ValueError(f"summary file {summary_path} is empty")
    return json.loads(lines[-1])


def check_scenarios(data, max_latency_ms, min_success):
    failures = []
    for scenario in data.get("scenarios", []):
        name = scenario.get("name", "unknown")
        mean_latency = scenario.get("mean_latency_ms", 0.0)
        success_rate = scenario.get("success_rate", 0.0)
        if success_rate < min_success:
            failures.append(
                f"scenario {name}: success_rate {success_rate:.2f} < {min_success:.2f}"
            )
        if mean_latency > max_latency_ms:
            failures.append(
                f"scenario {name}: mean_latency {mean_latency:.2f} ms > {max_latency_ms:.2f} ms"
            )
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description="Check swe-grep benchmark regressions")
    parser.add_argument(
        "--summary",
        default="docs/benchmark-summary.jsonl",
        help="Path to benchmark JSONL summary (default: docs/benchmark-summary.jsonl)",
    )
    parser.add_argument(
        "--max-latency-ms",
        type=float,
        default=20.0,
        help="Maximum allowed mean latency per scenario (default: 20 ms)",
    )
    parser.add_argument(
        "--min-success",
        type=float,
        default=0.99,
        help="Minimum required success rate per scenario (default: 0.99)",
    )

    args = parser.parse_args()
    summary_path = Path(args.summary)

    try:
        latest = load_latest(summary_path)
    except (FileNotFoundError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    failures = check_scenarios(latest, args.max_latency_ms, args.min_success)
    if failures:
        print("Benchmark regression detected:")
        for failure in failures:
            print(f"  - {failure}")
        return 1

    print(
        f"Benchmarks OK (<= {args.max_latency_ms:.1f} ms mean latency, >= {args.min_success:.2f} success)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
