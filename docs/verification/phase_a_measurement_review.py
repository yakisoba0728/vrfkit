#!/usr/bin/env python3
"""Independently recompute Phase A paired measurements from recorded samples.

This reviews existing receipts. It does not execute a replay or measure speed.
"""

import argparse
import hashlib
import json
import math
import statistics
from pathlib import Path


METRICS = ("wall_seconds", "cpu_total_seconds", "peak_rss_bytes")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def equal(left, right):
    assert math.isclose(left, right, rel_tol=1e-12, abs_tol=1e-12), (left, right)


def review(path, reader=False):
    data = json.loads(path.read_text())
    assert data["status"] == "completed" and data["equivalent"] is True
    samples = data["samples"]
    for sample in samples:
        assert sample["status"] == "completed" and sample["exit_code"] == 0
        if not reader:
            assert sample["valid"] is True
        measurements = sample["measurements"]
        equal(measurements["cpu_total_seconds"],
              measurements["cpu_user_seconds"] + measurements["cpu_system_seconds"])
        equal(measurements["cpu_utilization_percent"],
              100 * measurements["cpu_total_seconds"] / measurements["wall_seconds"])
    measured = [sample for sample in samples if sample["phase"] == "measured"]
    rounds = sorted({sample["round"] for sample in measured})
    recomputed = {}
    for metric in METRICS:
        ratios = []
        for round_number in rounds:
            pair = [sample for sample in measured if sample["round"] == round_number]
            assert len(pair) == 2
            values = {sample["variant"]: sample["measurements"][metric] for sample in pair}
            assert set(values) == {"baseline", "candidate"}
            ratios.append(values["candidate"] / values["baseline"])
        reported = data["paired_ratios" if reader else "comparison"][metric]
        assert len(ratios) == len(reported["candidate_over_baseline"])
        for actual, expected in zip(ratios, reported["candidate_over_baseline"]):
            equal(actual, expected)
        median = statistics.median(ratios)
        equal(median, reported["median_candidate_over_baseline"])
        recomputed[metric] = median
        for variant in ("baseline", "candidate"):
            values = [sample["measurements"][metric] for sample in measured
                      if sample["variant"] == variant]
            equal(statistics.median(values), data["summary"][variant][metric]["median"])
    fingerprint = "output_sha256" if reader else "output_fingerprint"
    assert len({sample[fingerprint] for sample in samples}) == 1
    if not reader:
        assert data["provenance_before"] == data["provenance_after"]
    return {"report_sha256": digest(path), "exports_or_reads": len(samples),
            "measured_pairs": len(rounds), "recomputed_paired_ratios": recomputed}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reader_evidence", type=Path)
    parser.add_argument("performance_evidence", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    reader = {}
    for name in ("reader-input-1209-primary.json", "reader-input-1209-prefetch.json",
                 "reader-input-1205-prefetch.json", "reader-input-1209-control.json"):
        reader[name] = review(args.reader_evidence / name, reader=True)
    parity_path = args.reader_evidence / "reader-public-parity-prefetch.json"
    parity = json.loads(parity_path.read_text())
    assert len(parity["samples"]) == 6
    for sample in parity["samples"]:
        assert sample["equal"] is True and sample["input_unchanged"] is True
        variants = sample["variants"]
        assert variants["buffered"]["returncode"] == variants["file"]["returncode"] == 0
        assert variants["buffered"]["stdout"] == variants["file"]["stdout"]
    performance = {}
    names = {"1209-checkpoints": "phase-a-inline-1209-checkpoints.json",
             "1205-main": "phase-a-inline-1205-main.json",
             "same-binary": "phase-a-inline-same-binary.json",
             "1209-reversed": "phase-a-inline-1209-reversed.json"}
    decision = json.loads((args.performance_evidence / "phase-a-inline-decision.json").read_text())
    assert decision["decision"] == "withdrawn"
    assert decision["accepted_runtime_optimizations"] == 0
    exports, tables = 0, 0
    for key, name in names.items():
        path = args.performance_evidence / name
        result = review(path)
        source = json.loads(path.read_text())
        decision_series = decision["series"][key]
        assert result["report_sha256"] == decision_series["report_sha256"]
        assert result["exports_or_reads"] == decision_series["exports"]
        assert len(source["reference_output"]["tables"]) == decision_series["tables_per_export"]
        exports += result["exports_or_reads"]
        tables += result["exports_or_reads"] * len(source["reference_output"]["tables"])
        performance[key] = result
    assert exports == decision["total_exports"] == decision["equivalent_exports"] == 60
    assert tables == decision["parquet_files_fingerprinted"] == 668
    control = json.loads((args.performance_evidence / names["same-binary"]).read_text())
    assert control["provenance_before"]["baseline"] == control["provenance_before"]["candidate"]
    reverse = json.loads((args.performance_evidence / names["1209-reversed"]).read_text())
    primary = json.loads((args.performance_evidence / names["1209-checkpoints"]).read_text())
    assert reverse["provenance_before"]["baseline"] == primary["provenance_before"]["candidate"]
    assert reverse["provenance_before"]["candidate"] == primary["provenance_before"]["baseline"]
    assert performance["1209-checkpoints"]["recomputed_paired_ratios"]["cpu_total_seconds"] < 1
    assert performance["1209-reversed"]["recomputed_paired_ratios"]["cpu_total_seconds"] < 1
    report = {"schema_version": 1, "status": "passed", "reader": reader,
              "reader_public_parity_receipt_sha256": digest(parity_path),
              "reader_public_parity_rows_reviewed": 6, "performance": performance,
              "performance_exports_reviewed": exports, "performance_table_fingerprints_reviewed": tables,
              "decision": "withdrawal supported; reverse-label CPU direction changes and same-binary variation preclude a stable improvement claim",
              "scope": "Independent arithmetic and receipt consistency review, not a new timing run or replay decode; fingerprints are recorded author evidence."}
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print("passed: Reader receipts, six public parity entries, 60 performance exports and 668 recorded table fingerprints")


if __name__ == "__main__":
    main()
