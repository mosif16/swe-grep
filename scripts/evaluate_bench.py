#!/usr/bin/env python3
import argparse
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description="Check swe-grep vs rg benchmark gap")
    parser.add_argument("--input", required=True, help="JSON benchmark file")
    parser.add_argument("--max-gap-ms", type=float, default=6.0, help="Allowed mean latency gap")
    args = parser.parse_args()

    payload = json.loads(Path(args.input).read_text())
    rg_mean = payload["rg"]["mean_ms"]
    sweg_mean = payload["swe_grep"]["mean_ms"]
    gap = sweg_mean - rg_mean

    print(f"rg_mean_ms={rg_mean:.3f}")
    print(f"swe_grep_mean_ms={sweg_mean:.3f}")
    print(f"gap_ms={gap:.3f}")

    if gap > args.max_gap_ms:
        raise SystemExit(f"FAIL: swe-grep exceeds allowed gap ({gap:.3f} > {args.max_gap_ms})")

if __name__ == "main":
    main()
