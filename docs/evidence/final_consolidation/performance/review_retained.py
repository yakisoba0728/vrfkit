#!/usr/bin/env python3
"""Recompute retained Phase D arithmetic; never launches a replay or benchmark."""

from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import statistics
import sys


ROOT = Path(__file__).resolve().parents[4]
PHASE_D = ROOT / "docs/evidence/phase_d"
METRICS = (
    "wall_seconds", "cpu_user_seconds", "cpu_system_seconds", "cpu_total_seconds",
    "cpu_utilization_percent", "peak_rss_bytes", "input_mib_per_second",
    "io_input_blocks", "io_output_blocks", "output_bytes", "parquet_bytes",
)
PAIRED = ("wall_seconds", "cpu_total_seconds", "peak_rss_bytes")
MANDATORY = (
    "movement_component_streams", "movement_component_complete",
    "movement_component_errors", "movement_component_empty",
    "movement_component_envelopes", "movement_component_payload_bytes",
    "movement_component_unused_bits", "movement_component_input_headers",
    "movement_component_discrepancy_headers", "route_children_focus_projectiles",
    "focus_projectiles_empty_trailers",
)


def require(condition, detail):
    if not condition:
        raise ValueError(detail)


def equal(actual, expected, detail):
    if isinstance(actual, (int, float)) and isinstance(expected, (int, float)):
        require(math.isfinite(actual) and math.isfinite(expected), detail)
        require(math.isclose(actual, expected, rel_tol=1e-12, abs_tol=1e-12), detail)
    else:
        require(actual == expected, detail)


def read(path):
    def unique(items):
        result = {}
        for key, value in items:
            require(key not in result, f"duplicate JSON key in {path.name}")
            result[key] = value
        return result

    def nonfinite(_):
        raise ValueError(f"nonfinite JSON in {path.name}")

    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique,
                      parse_constant=nonfinite)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def series(path, is_control):
    report = read(path)
    require(report["status"] == "completed" and report["equivalent"] is True,
            f"unqualified series: {path.name}")
    require(report["failures"] == [], f"failed series: {path.name}")
    require(report["provenance_before"] == report["provenance_after"],
            f"changed provenance: {path.name}")
    if is_control:
        require(report["provenance_before"]["baseline"] ==
                report["provenance_before"]["candidate"], f"not same binary: {path.name}")
    require(report["configuration"]["warmup_pairs"] == 1 and
            report["configuration"]["measured_pairs"] == 6,
            f"protocol mismatch: {path.name}")
    expected_order = [("warmup", 0, "baseline"), ("warmup", 0, "candidate")]
    for index in range(6):
        order = ("baseline", "candidate") if index % 2 == 0 else ("candidate", "baseline")
        expected_order.extend(("measured", index, variant) for variant in order)
    require([(s["phase"], s["round"], s["variant"]) for s in report["samples"]]
            == expected_order, f"sample count or order mismatch: {path.name}")
    reference = report["reference_output"]
    fingerprint = hashlib.sha256(json.dumps(
        {key: reference[key] for key in ("tables", "manifest_sha256", "counters")},
        sort_keys=True, separators=(",", ":"), allow_nan=False).encode()).hexdigest()
    for prefix in (("", "cp_") if report["configuration"]["checkpoints"] else ("",)):
        for key in MANDATORY:
            value = reference["counters"][prefix + key]
            require(type(value) is int and value >= 0, f"missing/invalid counter: {path.name}")
    for sample in report["samples"]:
        require(sample["valid"] is True and sample["status"] == "completed" and
                sample["exit_code"] == 0, f"invalid sample: {path.name}")
        require(sample["output_fingerprint"] == fingerprint,
                f"changed output fingerprint: {path.name}")
        values = sample["measurements"]
        equal(values["cpu_total_seconds"], values["cpu_user_seconds"] +
              values["cpu_system_seconds"], f"CPU sum: {path.name}")
        equal(values["cpu_utilization_percent"],
              100 * values["cpu_total_seconds"] / values["wall_seconds"],
              f"CPU utilization: {path.name}")
        equal(values["input_mib_per_second"], report["provenance_before"]["input"]["bytes"] /
              (1024 ** 2) / values["wall_seconds"], f"throughput: {path.name}")
    measured = [sample for sample in report["samples"] if sample["phase"] == "measured"]
    for variant in ("baseline", "candidate"):
        for metric in METRICS:
            values = [sample["measurements"][metric] for sample in measured
                      if sample["variant"] == variant]
            median = statistics.median(values)
            recalculated = {
                "samples": len(values), "median": median, "min": min(values), "max": max(values),
                "sample_stdev": statistics.stdev(values),
                "median_absolute_deviation": statistics.median(abs(v - median) for v in values),
            }
            for statistic, value in recalculated.items():
                equal(report["summary"][variant][metric][statistic], value,
                      f"{path.name}: {variant}/{metric}/{statistic}")
    by_pair = {(sample["round"], sample["variant"]): sample["measurements"] for sample in measured}
    ratios = {}
    for metric in PAIRED:
        ratios[metric] = [by_pair[index, "candidate"][metric] /
                          by_pair[index, "baseline"][metric] for index in range(6)]
        for actual, expected in zip(report["comparison"][metric]["candidate_over_baseline"], ratios[metric], strict=True):
            equal(actual, expected, f"pair ratio: {path.name}")
        equal(report["comparison"][metric]["median_candidate_over_baseline"],
              statistics.median(ratios[metric]), f"paired median: {path.name}")
    return report, ratios, fingerprint


def review():
    result = {"schema_version": 1, "status": "PASS",
              "reviewed_utc": datetime.now(timezone.utc).isoformat(),
              "scope": "New arithmetic review of retained Phase D records; zero new performance executions.",
              "new_replay_executions": 0, "new_performance_measurements": 0,
              "retained_series_checked": 0, "retained_records_checked": 0,
              "retained_warmups_checked": 0, "retained_measured_records_checked": 0,
              "summary_statistic_cells_checked": 0, "series_sha256": {}, "conditions": []}
    for directory, summary_name in (("benchmark", "primary-summary.json"),
                                    ("optimization-measurement", "summary.json")):
        summary = read(PHASE_D / directory / summary_name)
        for row in summary["rows"]:
            name = f"{row['build']}-{row['mode']}"
            compared = {}
            fingerprints = []
            for kind in ("control", "comparison"):
                path = PHASE_D / directory / f"{kind}-{name}.json"
                require(sha(path) == row["series_sha256"][kind], f"retained hash mismatch: {path.name}")
                report, ratios, fingerprint = series(path, kind == "control")
                fingerprints.append(fingerprint)
                compared[kind] = ratios
                result["series_sha256"][str(path.relative_to(ROOT))] = sha(path)
                result["retained_series_checked"] += 1
                result["retained_records_checked"] += len(report["samples"])
                result["retained_warmups_checked"] += 2
                result["retained_measured_records_checked"] += 12
                result["summary_statistic_cells_checked"] += 2 * len(METRICS) * 6
                for metric in PAIRED:
                    stored = row["control" if kind == "control" else "paired_changes"][metric]
                    equal(stored["median"], statistics.median(ratios[metric]), "summary paired median")
                    equal(stored["range"], [min(ratios[metric]), max(ratios[metric])], "summary paired range")
                    equal(stored["ratios"], ratios[metric], "summary ratios")
            require(len(set(fingerprints)) == 1, f"A/A vs A/B output difference: {name}")
            wall, control = compared["comparison"]["wall_seconds"], compared["control"]["wall_seconds"]
            crosses_one = min(wall) < 1 < max(wall)
            overlaps_control = max(min(wall), min(control)) <= min(max(wall), max(control))
            require(crosses_one and overlaps_control, f"inconclusive reason changed: {name}")
            rss = compared["comparison"]["peak_rss_bytes"]
            result["conditions"].append({
                "phase_d_directory": directory, "build": row["build"], "mode": row["mode"],
                "historical_wall_classification": "INCONCLUSIVE",
                "wall_pairs_on_both_sides_of_one": crosses_one,
                "wall_ranges_overlap_same_binary_control": overlaps_control,
                "paired_wall_percent_change": (statistics.median(wall) - 1) * 100,
                "same_binary_wall_range": [min(control), max(control)],
                "paired_rss_percent_change": (statistics.median(rss) - 1) * 100,
                "rss_increases": sum(value > 1 for value in rss),
                "paired_rss_percent_range": [(min(rss) - 1) * 100, (max(rss) - 1) * 100],
            })
    allocator = read(PHASE_D / "optimization/allocator.json")
    baseline, experiment = allocator["samples"]
    require(baseline["output_equivalent"] and experiment["output_equivalent"], "allocator output gate")
    result["retained_allocator_deltas"] = {
        "realloc_calls": experiment["functions"]["realloc"]["calls"] - baseline["functions"]["realloc"]["calls"],
        "heap_total_reported": experiment["heap_total_reported"] - baseline["heap_total_reported"],
        "heap_peak_reported": experiment["heap_peak_reported"] - baseline["heap_peak_reported"],
        "scope": "One historical interposed pair; not process RSS or ordinary throughput.",
    }
    require(result["retained_allocator_deltas"]["realloc_calls"] == -1571, "allocator calls")
    require(result["retained_allocator_deltas"]["heap_total_reported"] == 27201974, "allocator total")
    require(result["retained_allocator_deltas"]["heap_peak_reported"] == 3464100, "allocator peak")
    result["limitations"] = [
        "Retained fingerprints are recomputed from retained hash/counter metadata; no historical replay or Parquet file was reopened.",
        "No exclusive host, new A/A control, new binary comparison, stage profile or allocation probe was performed.",
        "Overlapping ranges are descriptive evidence, not a confidence interval, equivalence test or proof of no regression.",
    ]
    return result


if __name__ == "__main__":
    try:
        json.dump(review(), sys.stdout, indent=2, allow_nan=False)
        print()
    except (ValueError, KeyError, TypeError, OSError) as error:
        print(f"FAILED: {error}", file=sys.stderr)
        raise SystemExit(1) from error
