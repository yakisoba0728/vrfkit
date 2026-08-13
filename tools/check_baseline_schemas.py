#!/usr/bin/env python3
"""Validate every committed baseline's schema and cross-file identities."""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path, PureWindowsPath

try:
    from .check_export_baseline import CHECKPOINT_COUNTERS, COUNTERS, PARQUET_FILES
except ImportError:  # direct script execution
    from check_export_baseline import CHECKPOINT_COUNTERS, COUNTERS, PARQUET_FILES

REPO = Path(__file__).resolve().parent.parent
BASELINES = REPO / "tools" / "baselines"
MAIN_COUNTERS = frozenset(COUNTERS)
CHECKPOINT_ONLY_COUNTERS = frozenset(CHECKPOINT_COUNTERS)
MAIN_PARQUET = tuple(PARQUET_FILES)
CORPUS_TOTALS = ("blocks", "fields", "rpcs", "malformed", "skipped")
BUILDS = ("12.10", "12.11", "13.00", "13.01", "13.02")
SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")


def _keys(path: Path, value: dict, expected: set[str], problems: list[str]) -> None:
    got = set(value) if isinstance(value, dict) else set()
    if got != expected:
        problems.append(
            f"{path.name}: keys are {sorted(got)}, expected {sorted(expected)}"
        )


def _nonnegative_int(value) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def validate_export_baseline(
    path: Path, data: dict, *, require_hashes: bool = True
) -> list[str]:
    problems: list[str] = []
    _keys(path, data, {"replay", "counters", "parquet"}, problems)
    if not isinstance(data.get("replay"), str) or not data.get("replay", "").endswith(".vrf"):
        problems.append(f"{path.name}: replay must name a .vrf file")

    checkpoint = path.name.startswith("checkpoint_")
    expected_counters = MAIN_COUNTERS | (CHECKPOINT_ONLY_COUNTERS if checkpoint else set())
    counters = data.get("counters")
    if not isinstance(counters, dict):
        problems.append(f"{path.name}: counters must be an object")
        counters = {}
    _keys(path, counters, set(expected_counters), problems)
    for key, value in counters.items():
        if not _nonnegative_int(value):
            problems.append(f"{path.name}: counters.{key} must be a non-negative integer")

    expected_tables = set(MAIN_PARQUET) | ({"checkpoint_fields"} if checkpoint else set())
    parquet = data.get("parquet")
    if not isinstance(parquet, dict):
        problems.append(f"{path.name}: parquet must be an object")
        parquet = {}
    _keys(path, parquet, expected_tables, problems)
    for name, record in parquet.items():
        if not isinstance(record, dict):
            problems.append(f"{path.name}: parquet.{name} must be an object")
            continue
        expected_record_keys = {"rows", "bytes", "sha256"}
        if not require_hashes and "sha256" not in record:
            expected_record_keys.remove("sha256")
        _keys(path, record, expected_record_keys, problems)
        for field in ("rows", "bytes"):
            if not _nonnegative_int(record.get(field)):
                problems.append(
                    f"{path.name}: parquet.{name}.{field} must be a non-negative integer"
                )
        digest = record.get("sha256")
        if (require_hashes or digest is not None) and (
            not isinstance(digest, str) or SHA256_RE.fullmatch(digest) is None
        ):
            problems.append(f"{path.name}: parquet.{name}.sha256 is not a measured SHA-256")
    return problems


def validate_corpus_baseline(path: Path, data: dict) -> list[str]:
    problems: list[str] = []
    _keys(path, data, {"corpus", "branches", "totals", "per_file"}, problems)
    if not isinstance(data.get("corpus"), str) or not data.get("corpus"):
        problems.append(f"{path.name}: corpus must be a non-empty string")
    branches = data.get("branches") if isinstance(data.get("branches"), dict) else {}
    totals = data.get("totals") if isinstance(data.get("totals"), dict) else {}
    per_file = data.get("per_file") if isinstance(data.get("per_file"), dict) else {}
    _keys(path, totals, set(CORPUS_TOTALS), problems)
    summed = Counter({key: 0 for key in CORPUS_TOTALS})
    seen_branches: Counter[str] = Counter()
    if not per_file:
        problems.append(f"{path.name}: per_file must contain at least one replay")
    for name, entry in per_file.items():
        if not isinstance(name, str) or not name.endswith(".vrf") or not isinstance(entry, dict):
            problems.append(f"{path.name}: invalid per_file entry {name!r}")
            continue
        _keys(path, entry, {"branch", "rate", *CORPUS_TOTALS}, problems)
        branch = entry.get("branch")
        if isinstance(branch, str) and branch:
            seen_branches[branch] += 1
        else:
            problems.append(f"{path.name}: {name}.branch must be a non-empty string")
        try:
            rate = float(entry.get("rate"))
        except (TypeError, ValueError):
            rate = -1
        if not 0 <= rate <= 100:
            problems.append(f"{path.name}: {name}.rate is not a percentage")
        for key in CORPUS_TOTALS:
            value = entry.get(key)
            if not _nonnegative_int(value):
                problems.append(f"{path.name}: {name}.{key} must be a non-negative integer")
            else:
                summed[key] += value
    if dict(seen_branches) != branches:
        problems.append(f"{path.name}: branches do not count per_file entries")
    if dict(summed) != totals:
        problems.append(f"{path.name}: totals do not sum per_file entries")
    return problems


def _basename(raw: str) -> str:
    return PureWindowsPath(raw).name


def validate_repository(
    root: Path = BASELINES, *, require_hashes: bool = True
) -> list[str]:
    problems: list[str] = []
    loaded: dict[str, dict] = {}
    for path in sorted(root.glob("*.json")):
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError) as exc:
            problems.append(f"{path.name}: invalid JSON: {exc}")
            continue
        if not isinstance(value, dict):
            problems.append(f"{path.name}: top level must be an object")
            continue
        loaded[path.name] = value
        if path.name.startswith(("export_", "checkpoint_")):
            problems.extend(
                validate_export_baseline(path, value, require_hashes=require_hashes)
            )
        elif path.name.startswith("build_"):
            problems.extend(validate_corpus_baseline(path, value))

    required = {
        "bench.json", "metrics_builds.json", "export_02d4d478.json",
        "checkpoint_02d4d478.json", "build_1210.json", "build_1211.json",
        "build_1300.json", "build_1302.json",
    }
    missing = sorted(required - set(loaded))
    if missing:
        problems.append("missing committed baselines: " + ", ".join(missing))
        return problems

    export = loaded["export_02d4d478.json"]
    checkpoint = loaded["checkpoint_02d4d478.json"]
    if export.get("replay") != checkpoint.get("replay"):
        problems.append("export/checkpoint baselines name different replays")
    for key in MAIN_COUNTERS:
        if export.get("counters", {}).get(key) != checkpoint.get("counters", {}).get(key):
            problems.append(f"export/checkpoint counter {key} disagrees")
    for name in MAIN_PARQUET:
        if export.get("parquet", {}).get(name) != checkpoint.get("parquet", {}).get(name):
            problems.append(f"export/checkpoint {name}.parquet disagrees")

    bench = loaded["bench.json"]
    _keys(root / "bench.json", bench, {"export", "replay"}, problems)
    if bench.get("replay") != export.get("replay"):
        problems.append("bench/export baselines name different replays")
    if not isinstance(bench.get("export"), (int, float)) or bench.get("export", 0) <= 0:
        problems.append("bench.json: export must be positive")

    metrics = loaded["metrics_builds.json"]
    _keys(root / "metrics_builds.json", metrics, {"note", "replays", "metrics"}, problems)
    replays = metrics.get("replays") if isinstance(metrics.get("replays"), dict) else {}
    values = metrics.get("metrics") if isinstance(metrics.get("metrics"), dict) else {}
    if set(replays) != set(BUILDS) or set(values) != set(BUILDS):
        problems.append("metrics_builds.json: replay/metric build keys must be the five supported builds")
    metric_keys = {frozenset(v) for v in values.values() if isinstance(v, dict)}
    if len(metric_keys) != 1:
        problems.append("metrics_builds.json: per-build metric schemas disagree")

    corpus_files = {
        "12.10": "build_1210.json", "12.11": "build_1211.json",
        "13.00": "build_1300.json", "13.02": "build_1302.json",
    }
    for build, filename in corpus_files.items():
        corpus = loaded[filename]
        expected_branch = f"++Ares-Core+release-{build}"
        if corpus.get("branches") != {expected_branch: len(corpus.get("per_file", {}))}:
            problems.append(f"{filename}: branch does not identify build {build}")
        corpus_names = set(corpus.get("per_file", {}))
        if _basename(replays.get(build, "")) not in corpus_names:
            problems.append(f"metrics/build baseline replay disagrees for {build}")
    if _basename(replays.get("13.01", "")) != export.get("replay"):
        problems.append("metrics/export baseline replay disagrees for 13.01")
    return problems


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--allow-missing-hashes",
        action="store_true",
        help="migration aid: validate legacy structure/cross-file identities "
             "without accepting malformed hashes that are present",
    )
    args = ap.parse_args()
    problems = validate_repository(require_hashes=not args.allow_missing_hashes)
    if problems:
        print(f"FAILED: {len(problems)} baseline schema/cross-file problem(s)", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1
    qualifier = " (legacy hashes allowed)" if args.allow_missing_hashes else ""
    print(
        f"OK: {len(list(BASELINES.glob('*.json')))} committed baselines are "
        f"consistent{qualifier}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
