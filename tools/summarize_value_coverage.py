"""Count physical rows with typed values, separately from parser/block coverage.

Inputs are export directories or parents whose direct children are exports.
Read-only: the JSON report goes to stdout, and no export is modified. A typed
row has at least one non-null value_i64/f64/bool/str, including 0, False and
empty strings. Multiple populated columns still count as one typed row and
are reported separately. This is not a percentage of game facts understood.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import json
from pathlib import Path
import sys

import pyarrow as pa
import pyarrow.compute as pc
import pyarrow.parquet as pq

VALUE_COLUMNS = ("value_i64", "value_f64", "value_bool", "value_str")
TABLES = ("fields", "checkpoint_fields")


def count_table(path: Path) -> dict[str, int]:
    # Own the handle even if Arrow raises during footer parsing; otherwise an
    # exception traceback retained by a future can keep a Windows file locked.
    with path.open("rb") as source:
        return _count_parquet(pq.ParquetFile(source), path.name)


def _count_parquet(parquet: pq.ParquetFile, filename: str) -> dict[str, int]:
    missing = set(VALUE_COLUMNS) - set(parquet.schema_arrow.names)
    if missing:
        raise ValueError(f"{filename}: missing value columns {sorted(missing)}")
    rows = typed = multi = 0
    for batch in parquet.iter_batches(batch_size=65536, columns=list(VALUE_COLUMNS),
                                     use_threads=False):
        populated = pc.cast(pc.is_valid(batch.column(0)), pa.uint8())
        for index in range(1, len(VALUE_COLUMNS)):
            populated = pc.add(populated, pc.cast(pc.is_valid(batch.column(index)), pa.uint8()))
        rows += batch.num_rows
        typed += pc.sum(pc.greater(populated, 0)).as_py() or 0
        multi += pc.sum(pc.greater(populated, 1)).as_py() or 0
    if rows != parquet.metadata.num_rows:
        raise ValueError(f"{filename}: scanned rows disagree with footer")
    return {"rows": rows, "typed_rows": typed, "untyped_rows": rows - typed,
            "multi_value_rows": multi}


def discover(inputs: list[Path]) -> list[Path]:
    exports: set[Path] = set()
    for root in inputs:
        if not root.is_dir():
            raise ValueError(f"not an export directory or parent: {root}")
        if (root / "fields.parquet").is_file():
            exports.add(root.resolve())
            continue
        children = [p.parent.resolve() for p in root.glob("*/fields.parquet")]
        if not children:
            raise ValueError(f"no direct child exports in {root}")
        exports.update(children)
    return sorted(exports)


def count_export(directory: Path) -> dict[str, dict[str, int]]:
    result = {"fields": count_table(directory / "fields.parquet")}
    checkpoint = directory / "checkpoint_fields.parquet"
    if checkpoint.exists():
        result["checkpoint_fields"] = count_table(checkpoint)
    return result


def summarize(exports: list[Path], jobs: int) -> dict:
    if jobs < 1:
        raise ValueError("jobs must be positive")
    totals = {name: {"exports_with_table": 0, "rows": 0, "typed_rows": 0,
                     "untyped_rows": 0, "multi_value_rows": 0} for name in TABLES}
    errors = []
    successful = 0
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        pending = {pool.submit(count_export, p): p for p in exports}
        for future in as_completed(pending):
            try:
                measured = future.result()
            except (OSError, ValueError, pa.ArrowException) as exc:
                errors.append({"export": str(pending[future]), "error": str(exc)})
                continue
            successful += 1
            for name, counts in measured.items():
                totals[name]["exports_with_table"] += 1
                for key, value in counts.items():
                    totals[name][key] += value
    for counts in totals.values():
        counts["typed_fraction"] = (counts["typed_rows"] / counts["rows"]
                                    if counts["rows"] else None)
    return {
        "schema_version": 1,
        "complete": not errors,
        "export_count": len(exports),
        "successful_exports": successful,
        "denominator": "physical field rows in successfully read export directories",
        "interpretation": "non-null typed values, not semantic or block-loss coverage",
        "tables": totals,
        "errors": sorted(errors, key=lambda e: e["export"]),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inputs", type=Path, nargs="+")
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args(argv)
    try:
        report = summarize(discover(args.inputs), args.jobs)
    except ValueError as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(report, indent=2, allow_nan=False))
    return 0 if report["complete"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
