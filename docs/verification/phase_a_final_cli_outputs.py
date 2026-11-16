#!/usr/bin/env python3
"""Compare Native-accepted and combined Phase A exports under one contract."""

import argparse
import json
import math
import re
from pathlib import Path

import pyarrow.parquet as pq

from phase_a_native_outputs import digest, exact


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    roots = [p.resolve() for p in (args.before, args.after)]
    output = args.output.resolve()
    if output.exists() or any(output.is_relative_to(p) for p in roots):
        raise ValueError("use a new report outside the two export trees")
    names = lambda root: {p.name for p in root.iterdir()
                          if p.is_dir() and re.fullmatch(r"[a-f0-9]{64}", p.name)}
    inputs = names(roots[0])
    if len(inputs) != 6 or inputs != names(roots[1]):
        raise ValueError("expected the same six public inputs")
    tables, totals = [], {}
    for source in sorted(inputs):
        before, after = [p / source / "export" for p in roots]
        files = {p.name for p in before.glob("*.parquet")}
        if len(files) != 13 or files != {p.name for p in after.glob("*.parquet")}:
            raise ValueError("changed or missing table population")
        for name in sorted(files):
            left, right = before / name, after / name
            if digest(left) != digest(right):
                raise ValueError("Parquet bytes changed")
            metadata = pq.read_metadata(right)
            totals[name] = totals.get(name, 0) + metadata.num_rows
            tables.append({"input_sha256": source, "table": name,
                           "sha256": digest(right), "bytes": right.stat().st_size,
                           "rows": metadata.num_rows})
        manifests = [json.loads((p / "manifest.json").read_text())
                     for p in (before, after)]
        for manifest in manifests:
            elapsed = manifest.pop("elapsed_ms")
            if type(elapsed) not in (int, float) or not math.isfinite(elapsed) or elapsed < 0:
                raise ValueError("invalid elapsed time")
        exact(*manifests)
    result = {"schema_version": 1, "status": "passed", "inputs": len(inputs),
              "tables_compared": len(tables), "row_totals": totals,
              "unexplained_differences": 0,
              "allowed_manifest_difference": "finite nonnegative elapsed_ms only",
              "comparison_script_sha256": digest(Path(__file__)),
              "strict_comparison_helper_sha256": digest(Path(__file__).with_name("phase_a_native_outputs.py")),
              "tables": tables}
    with output.open("x") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print(json.dumps({k: v for k, v in result.items() if k != "tables"}))


if __name__ == "__main__":
    main()
