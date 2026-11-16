"""Independent bounded full-corpus QA of the movement comparison artifacts.

This does not import the production comparator, use SQLite, modify replay
bytes, or retain player identities in its aggregate report. It parses both
exports into fixed-size NumPy arrays, sorts by the five independent wire keys,
and checks every latest value and every Rust component's move ordinal count.
"""

from __future__ import annotations

import argparse
from array import array
import gzip
import hashlib
import json
import math
from pathlib import Path
import resource
import sys
import time

import numpy as np


KEYS = ("packet", "actor", "object", "channel", "update_index")
SCALARS = ("character", "timestamp", "movement_state", "move_type", "move_index", "move_count")
FLOATS = ("pos_x", "pos_y", "pos_z", "yaw", "pitch", "vel_x", "vel_y", "vel_z")
BITS = tuple(name + "_bits" for name in FLOATS)
DTYPE = np.dtype([(name, "<u4") for name in (*KEYS, *SCALARS, *BITS)])
BLOCK = 65_536


def require(condition, message):
    if not condition:
        raise ValueError(message)


def uint(value, bits=32):
    require(type(value) is int and 0 <= value < 1 << bits, "invalid unsigned integer")
    return value


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON member")
        result[key] = value
    return result


def json_rows(path, receipt):
    before = path.stat()
    digest = hashlib.sha256()
    count = 0
    opener = gzip.open if path.suffix == ".gz" else open
    with opener(path, "rb") as stream:
        while line := stream.readline(1_048_577):
            require(len(line) <= 1_048_576, "oversized JSONL line")
            digest.update(line)
            row = json.loads(line, object_pairs_hook=unique_object)
            require(type(row) is dict, "JSON row must be an object")
            count += 1
            yield row
    after = path.stat()
    require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) ==
            (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns),
            "input changed while reading")
    receipt.update(logical_content_sha256=digest.hexdigest(), lines=count,
                   file_bytes=after.st_size)


def numeric_values(row, reference):
    if reference:
        values = [row["position"][axis] for axis in ("x", "y", "z")]
        values += [row["yaw"], row["pitch"]]
        values += [row["velocity"][axis] for axis in ("x", "y", "z")]
    else:
        values = [row[name] for name in FLOATS]
    require(all(type(value) in (float, int) and math.isfinite(value) for value in values),
            "movement contains a non-finite or nonnumeric value")
    # Python's C float array supplies an independent f64->f32 conversion path;
    # the production comparator uses struct.pack and a SQLite bits column.
    packed = array("f", values)
    require(all(math.isfinite(value) for value in packed), "non-finite f32 conversion")
    if not reference:
        require(all(value == rounded for value, rounded in zip(values, packed)),
                "Rust JSON value is not an exact expanded f32")
    words = array("I")
    words.frombytes(packed.tobytes())
    return tuple(words), values, packed


def scalar_values(row, reference):
    character = uint(row["shooter_character_net_guid" if reference else "character_net_guid"])
    timestamp = uint(row["timestamp"])
    state = uint(row["movement_state"], 8)
    move_type = uint(row["move_type"], 8)
    index = uint(row["move_index"])
    require(move_type == 1 and index < 16, "row is outside the explicit reference policy")
    return character, timestamp, state, move_type, index


def load_reference(path, capacity):
    rows = np.empty(capacity, dtype=DTYPE)
    receipt = {"rows": 0, "represented_moves": 0}
    deltas = [0.0] * len(FLOATS)
    for row in json_rows(path, receipt):
        require(receipt["rows"] < capacity, "reference exceeds fixed audit capacity")
        require(row["type"] == "remote_character_movement", "unexpected reference row type")
        require(row["error_sentinel"] is False, "reference error sentinel")
        require(uint(row["mode_flags"], 8) == uint(row["movement_state"], 8),
                "reference state disagreement")
        key = tuple(uint(row[name]) for name in
                    ("packet_id", "actor_net_guid", "object_net_guid", "channel", "update_index"))
        scalars = scalar_values(row, True)
        bits, values, rounded = numeric_values(row, True)
        count = scalars[-1] + 1
        rows[receipt["rows"]] = (*key, *scalars, count, *bits)
        receipt["rows"] += 1
        receipt["represented_moves"] += count
        for i, (value, quantized) in enumerate(zip(values, rounded)):
            deltas[i] = max(deltas[i], abs(value - quantized))
    receipt["maximum_absolute_quantization_delta"] = dict(zip(FLOATS, deltas))
    return rows[:receipt["rows"]], receipt


def load_rust(path, capacity):
    rows = np.empty(capacity, dtype=DTYPE)
    receipt = {"rows": 0, "component_groups": 0, "fragmented_move_rows": 0}
    iterator = json_rows(path, receipt)
    header = next(iterator, None)
    require(header == {"type": "movement_provenance_header", "schema_version": 1,
                       "float_precision": "f32"} and type(header["schema_version"]) is int,
            "invalid Reader header")
    last_group = last_source = latest = None
    character = None
    component_moves = 0
    footer = None

    def save_component():
        if latest is None:
            return
        require(receipt["component_groups"] < capacity, "Rust exceeds fixed audit capacity")
        key, scalars, bits = latest
        rows[receipt["component_groups"]] = (*key, *scalars, component_moves, *bits)
        receipt["component_groups"] += 1

    for row in iterator:
        require(footer is None, "Reader rows after terminal summary")
        if row["type"] == "movement_provenance_summary":
            footer = row
            continue
        require(row["type"] == "move", "unexpected Reader row type")
        group = tuple(uint(row[name]) for name in
                      ("completion_packet_id", "rpc_index", "array_ordinal",
                       "update_ordinal", "component_ordinal"))
        initial = uint(row["initial_packet_id"])
        completion = uint(row["completion_packet_id"])
        actor = uint(row["actor_net_guid"])
        object_guid = row["object_net_guid"]
        if object_guid is not None:
            uint(object_guid)
        channel = uint(row["channel_index"])
        wire_index = uint(row["update_index"])
        source = (initial, completion, actor, object_guid, channel, wire_index,
                  uint(row["bunch_index"], 64), uint(row["block_index"]),
                  uint(row["rpc_handle"]))
        if group != last_group:
            save_component()
            require(last_group is None or group > last_group, "noncontiguous or repeated Rust component")
            last_group, last_source = group, source
            component_moves = 0
            character = None
        else:
            require(source == last_source, "source metadata changed within component")
        scalars = scalar_values(row, False)
        if character is None:
            character = scalars[0]
        require(character == scalars[0], "character changed within component")
        require(scalars[-1] == component_moves, "move indices are not contiguous from zero")
        component_moves += 1
        require(initial <= completion, "initial packet follows completion")
        bits, _values, _rounded = numeric_values(row, False)
        latest = ((initial, actor, actor if object_guid is None else object_guid, channel, wire_index),
                  scalars, bits)
        receipt["rows"] += 1
        receipt["fragmented_move_rows"] += initial != completion
    save_component()
    require(footer is not None, "missing Reader terminal summary")
    require(uint(footer["movement_rows"], 64) == receipt["rows"], "Reader row count differs")
    for name in ("movement_rpc_errors", "lost_content_blocks", "partial_errors", "unfinished_partials"):
        require(uint(footer[name], 64) == 0, "Reader reported decode or transport errors")
    require(footer["block_verdict"] == "Passed", "Reader block verdict is not Passed")
    receipt["skipped_checkpoint_chunks"] = uint(footer["skipped_checkpoint_chunks"], 64)
    return rows[:receipt["component_groups"]], receipt


def duplicate_keys(rows):
    groups = extra = 0
    previous_duplicate = False
    for start in range(1, len(rows), BLOCK):
        end = min(start + BLOCK, len(rows))
        duplicate = np.ones(end - start, dtype=bool)
        for name in KEYS:
            duplicate &= rows[name][start:end] == rows[name][start - 1:end - 1]
        extra += int(np.count_nonzero(duplicate))
        groups += int(duplicate[0] and not previous_duplicate)
        groups += int(np.count_nonzero(duplicate[1:] & ~duplicate[:-1]))
        previous_duplicate = bool(duplicate[-1])
    return {"groups": groups, "extra_rows": extra}


def compare_arrays(reference, rust):
    # Extra unspecified fields break ties only; duplicate keys are rejected.
    # No numeric movement value participates in identity selection.
    reference.sort(order=KEYS, kind="quicksort")
    rust.sort(order=KEYS, kind="quicksort")
    duplicates = {"reference": duplicate_keys(reference), "rust": duplicate_keys(rust)}
    compared = min(len(reference), len(rust))
    differences = {name: 0 for name in DTYPE.names}
    for start in range(0, compared, BLOCK):
        end = min(start + BLOCK, compared)
        for name in DTYPE.names:
            differences[name] += int(np.count_nonzero(reference[name][start:end] != rust[name][start:end]))
    key_equal = len(reference) == len(rust) and not any(differences[name] for name in KEYS)
    passed = (compared > 0 and key_equal and not any(differences.values())
              and not any(value["groups"] for value in duplicates.values()))
    return {"status": "pass" if passed else "fail", "reference_latest_rows": len(reference),
            "rust_latest_rows": len(rust), "all_sorted_independent_keys_equal": key_equal,
            "duplicate_keys": duplicates, "mismatch_counts": differences,
            "reference_sorted_records_sha256": hashlib.sha256(memoryview(reference).cast("B")).hexdigest(),
            "rust_sorted_records_sha256": hashlib.sha256(memoryview(rust).cast("B")).hexdigest()}


def self_check():
    known = array("I")
    known.frombytes(array("f", [1.1, -0.0, 2.0 ** -149]).tobytes())
    require(list(known) == [0x3F8CCCCD, 0x80000000, 1], "f32 rounding/bit-layout control")
    sample = np.zeros(2, dtype=DTYPE)
    sample["packet"] = [1, 2]
    sample["move_type"] = 1
    sample["move_count"] = 1
    require(compare_arrays(sample.copy(), sample.copy())["status"] == "pass", "positive audit self-check")
    results = {}
    for field, changed in (("timestamp", 1), ("pos_x_bits", 0x80000000), ("packet", 3)):
        mutation = sample.copy()
        mutation[field][0] = changed
        results[field] = compare_arrays(sample.copy(), mutation)["status"] == "fail"
    duplicate = sample.copy()
    duplicate["packet"] = 1
    results["duplicate_identity"] = compare_arrays(duplicate.copy(), duplicate.copy())["status"] == "fail"
    require(all(results.values()), "audit negative control did not fail")
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("rust", type=Path)
    parser.add_argument("reported", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--capacity", type=int, default=3_000_000)
    args = parser.parse_args()
    require(1 <= args.capacity <= 3_000_000, "capacity must be within the fixed QA bound")
    require(sys.byteorder == "little" and array("f").itemsize == array("I").itemsize == 4,
            "audit requires native 32-bit float/unsigned on a little-endian host")
    started = time.monotonic()
    usage_before = resource.getrusage(resource.RUSAGE_SELF)
    controls = self_check()
    reference, reference_receipt = load_reference(args.reference, args.capacity)
    print(json.dumps({"stage": "reference_loaded", "rows": len(reference)}), flush=True)
    rust, rust_receipt = load_rust(args.rust, args.capacity)
    print(json.dumps({"stage": "rust_loaded", "rows": len(rust)}), flush=True)
    result = compare_arrays(reference, rust)
    total_count_equal = reference_receipt["represented_moves"] == rust_receipt["rows"]
    result["all_move_counts_equal"] = total_count_equal
    if not total_count_equal:
        result["status"] = "fail"
    reported = json.loads(args.reported.read_text(encoding="utf-8"))
    consistency = {
        "reference_sha256": reference_receipt["logical_content_sha256"] == reported["reference"]["logical_content_sha256"],
        "rust_sha256": rust_receipt["logical_content_sha256"] == reported["rust"]["logical_content_sha256"],
        "reference_rows": reference_receipt["rows"] == reported["reference"]["rows"],
        "rust_rows": rust_receipt["rows"] == reported["rust"]["rows"],
        "rust_component_groups": rust_receipt["component_groups"] == reported["rust"]["component_groups"],
        "fragmented_move_rows": rust_receipt["fragmented_move_rows"] == reported["rust"]["fragmented_move_rows"],
        "latest_mismatches": {name: result["mismatch_counts"][name] for name in (*SCALARS, *BITS)} == reported["value_comparison"]["mismatch_counts"],
        "raw_reference_deltas": reference_receipt["maximum_absolute_quantization_delta"] == reported["value_comparison"]["maximum_absolute_reference_delta"],
        "pass_status": result["status"] == reported["status"],
    }
    usage_after = resource.getrusage(resource.RUSAGE_SELF)
    receipt = {"schema_version": 1, "independent_qa": True,
               "algorithm": "Fixed NumPy arrays; full independent-key sorting and bitwise latest-value comparison; no SQLite/comparator imports.",
               "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
               "reported_comparison_sha256": hashlib.sha256(args.reported.read_bytes()).hexdigest(),
               "capacity_rows_per_input": args.capacity,
               "maximum_array_bytes": 2 * args.capacity * DTYPE.itemsize,
               "negative_controls_rejected": controls, "reference": reference_receipt, "rust": rust_receipt,
               "comparison": result, "reported_result_consistency": consistency,
               "elapsed_seconds": time.monotonic() - started,
               "audit_cpu_seconds": (usage_after.ru_utime + usage_after.ru_stime
                                     - usage_before.ru_utime - usage_before.ru_stime),
               "peak_rss_kib": usage_after.ru_maxrss,
               "resource_measurement_scope": "QA script resource use only; not replay parser performance.",
               "scope": "All latest values, every emitted Rust move ordinal and count. Earlier move values and absent output fields remain outside reference coverage.",
               "original_replays_opened": False, "raw_identifiers_in_report": False}
    args.out.write_text(json.dumps(receipt, indent=2, allow_nan=False) + "\n", encoding="utf-8")
    print(json.dumps({"stage": "complete", "status": result["status"],
                      "reported_result_consistent": all(consistency.values())}), flush=True)
    return 0 if result["status"] == "pass" and all(consistency.values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
