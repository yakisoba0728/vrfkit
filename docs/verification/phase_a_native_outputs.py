#!/usr/bin/env python3
"""Independent, read-only Phase A table/source/manifest acceptance.

Unlike the production comparison helper, this script reads the two actual
export trees, parses every Parquet footer and hashes every byte itself.
Detailed input paths and replay values are never written to the receipt.
"""

import argparse
import copy
import hashlib
import json
import math
import re
from pathlib import Path

import pyarrow.parquet as pq


OLD = {
    "movement_sized_section_tails", "movement_sized_section_tail_bits",
    "movement_open_section_tails", "movement_open_section_tail_bits",
    "movement_sized_section_trailers", "movement_sized_section_trailer_bits",
    "movement_envelope_trailers", "movement_envelope_trailer_bits",
}
NEW = {"movement_component_" + suffix for suffix in (
    "streams", "complete", "errors", "empty", "envelopes", "payload_bytes",
    "unused_bits", "input_headers", "discrepancy_headers",
)}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def exact(left, right):
    assert type(left) is type(right), "changed JSON type"
    if isinstance(left, dict):
        assert left.keys() == right.keys(), "unexplained JSON key change"
        for key in left:
            exact(left[key], right[key])
    elif isinstance(left, list):
        assert len(left) == len(right), "changed JSON array size"
        for a, b in zip(left, right):
            exact(a, b)
    else:
        assert left == right, "changed JSON value"


def compare_manifests(before, after):
    old, new = copy.deepcopy(before), copy.deepcopy(after)
    for doc in (old, new):
        elapsed = doc.pop("elapsed_ms")
        assert type(elapsed) in (int, float) and math.isfinite(elapsed) and elapsed >= 0
    for a, b in ((old["quality"]["sink"], new["quality"]["sink"]),
                 (old["quality"]["checkpoints"]["sink"],
                  new["quality"]["checkpoints"]["sink"])):
        assert OLD <= a.keys() and not NEW & a.keys()
        assert NEW <= b.keys() and not OLD & b.keys()
        for key in OLD:
            assert type(a.pop(key)) is int
        counters = {k.removeprefix("movement_component_"): b.pop(k) for k in NEW}
        assert all(type(value) is int and value >= 0 for value in counters.values())
        assert counters["streams"] == counters["complete"] + counters["errors"]
        assert counters["empty"] <= counters["complete"] <= counters["envelopes"] <= counters["streams"]
        assert counters["complete"] - counters["empty"] <= counters["input_headers"] <= counters["envelopes"] - counters["empty"]
        assert counters["discrepancy_headers"] <= counters["input_headers"]
        assert counters["unused_bits"] <= 7 * (counters["envelopes"] - counters["empty"])
        assert counters["payload_bytes"] >= counters["envelopes"] - counters["empty"]
    exact(old, new)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    names = lambda root: {x.name for x in root.iterdir() if x.is_dir() and re.fullmatch("[a-f0-9]{64}", x.name)}
    inputs = names(args.before)
    assert len(inputs) == 6 and inputs == names(args.after), "missing or changed input population"
    tables = []
    rows = {}
    manifest_controls = 0
    for replay in sorted(inputs):
        left, right = args.before / replay / "export", args.after / replay / "export"
        files = {p.name for p in left.glob("*.parquet")}
        assert len(files) == 13 and files == {p.name for p in right.glob("*.parquet")}
        for name in sorted(files):
            a, b = left / name, right / name
            assert a.is_file() and b.is_file() and not a.is_symlink() and not b.is_symlink()
            ah, bh = digest(a), digest(b)
            assert ah == bh, "Parquet bytes changed"
            am, bm = pq.read_metadata(a), pq.read_metadata(b)
            assert am.num_rows == bm.num_rows and am.schema == bm.schema
            rows[name] = rows.get(name, 0) + bm.num_rows
            tables.append({"input_sha256": replay, "table": name, "sha256": bh,
                           "bytes": b.stat().st_size, "rows": bm.num_rows})
        a, b = (json.loads((root / "manifest.json").read_text()) for root in (left, right))
        compare_manifests(a, b)
        # Negative controls change data outside the allowed migration, its type,
        # and a supposedly harmless elapsed value. Each must be rejected.
        for change in ("value", "type", "counter", "elapsed"):
            mutant = copy.deepcopy(b)
            if change == "value":
                mutant["source_size_bytes"] += 1
            elif change == "type":
                mutant["quality"]["unknown_chunks"] = False
            elif change == "counter":
                mutant["quality"]["sink"]["movement_component_streams"] += 1
            else:
                mutant["elapsed_ms"] = True
            try:
                compare_manifests(a, mutant)
            except AssertionError:
                manifest_controls += 1
            else:
                raise AssertionError("manifest negative control escaped")
    build = json.loads((args.source / "docs/evidence/native-movement-build.json").read_text())
    sources = build["parser_sources"]
    mismatches = [name for name, expected in sources.items() if digest(args.source / name) != expected]
    assert not mismatches, "published source differs from frozen binary receipt"
    receipt = {
        "schema_version": 1, "status": "passed", "inputs": len(inputs),
        "tables_hashed_and_footer_parsed": len(tables), "row_totals": rows,
        "unexplained_table_or_manifest_differences": 0,
        "manifest_negative_controls_rejected": manifest_controls,
        "source_files_hashed": len(sources), "source_mismatches": mismatches,
        "source_receipt_sha256": digest(args.source / "docs/evidence/native-movement-build.json"),
        "script_sha256": digest(Path(__file__)), "table_receipts": tables,
        "scope": "Independent read of existing baseline and freshly generated Phase A exports; no new replay decoding or independent game semantics claimed.",
    }
    args.output.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({k: v for k, v in receipt.items() if k != "table_receipts"}, indent=2))


if __name__ == "__main__":
    main()
