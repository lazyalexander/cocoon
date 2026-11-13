#!/usr/bin/env python3
"""Aggregate real benchmark JSONL samples; no VM execution or simulated samples."""
import argparse
import json
import math
import pathlib
import statistics
import sys
from collections import defaultdict


def percentile(values, percent):
    ordered = sorted(values)
    return ordered[math.ceil(len(ordered) * percent / 100) - 1]


def analyze(rows, minimum):
    groups = defaultdict(list)
    seen = set()
    for row in rows:
        if set(row) != {"run_id", "host", "scenario", "metric", "unit", "value", "result"}:
            raise ValueError("sample must contain exactly the benchmark schema fields")
        key = (row["run_id"], row["scenario"], row["metric"])
        if key in seen:
            raise ValueError("duplicate sample identity")
        seen.add(key)
        if not all(isinstance(row[field], str) and row[field] for field in ["run_id", "host", "scenario", "metric", "unit"]):
            raise ValueError("empty benchmark identity")
        if row["result"] not in {"passed", "failed", "unknown"}:
            raise ValueError("invalid result")
        if not isinstance(row["value"], (int, float)) or isinstance(row["value"], bool) or not math.isfinite(row["value"]) or row["value"] < 0:
            raise ValueError("invalid measured value")
        groups[(row["host"], row["scenario"], row["metric"], row["unit"])].append(row)
    report = []
    for (host, scenario, metric, unit), samples in sorted(groups.items()):
        passed = [row["value"] for row in samples if row["result"] == "passed"]
        failures = sum(row["result"] == "failed" for row in samples)
        unknown = sum(row["result"] == "unknown" for row in samples)
        complete = len(samples) >= minimum and failures == 0 and unknown == 0
        report.append({"host": host, "scenario": scenario, "metric": metric, "unit": unit,
                       "samples": len(samples), "failed": failures, "unknown": unknown,
                       "complete": complete,
                       "median": statistics.median(passed) if complete else None,
                       "p95": percentile(passed, 95) if complete and len(passed) >= 100 else None,
                       "p99": percentile(passed, 99) if complete and len(passed) >= 300 else None})
    return {"schema": "cocoon-benchmark-summary-v1", "groups": report,
            "complete": bool(report) and all(group["complete"] for group in report)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("samples", type=pathlib.Path, help="Private JSONL measurements from real runs")
    parser.add_argument("--minimum", type=int, default=100)
    args = parser.parse_args()
    if args.minimum < 1:
        parser.error("--minimum must be positive")
    rows = [json.loads(line) for line in args.samples.read_text().splitlines() if line.strip()]
    report = analyze(rows, args.minimum)
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["complete"] else 2


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError) as error:
        print(f"benchmark input: {error}", file=sys.stderr)
        raise SystemExit(2)
