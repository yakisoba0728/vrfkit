"""Gate complete latest-move correspondence using observed wire provenance.

The reference emits its latest move per update; vrfkit emits every move. This
tool requires the Reader's movement_provenance example output, not ordinary
movement.parquet. It never joins by a numeric movement value or packet offset.
Temporary indexed storage bounds resident memory; only aggregates are reported.
"""

from __future__ import annotations

import argparse
from contextlib import closing
import gzip
import hashlib
import json
import math
import os
import sqlite3
import struct
import sys
import tempfile
import zlib
from pathlib import Path

from atomic_io import atomic_write_text, refuse_input_path, sha256_file


KEY = ("packet", "actor", "object", "channel", "update_index")
SCALARS = ("character", "timestamp", "movement_state", "move_type", "move_index")
FLOATS = ("pos_x", "pos_y", "pos_z", "yaw", "pitch", "vel_x", "vel_y", "vel_z")
GROUP = ("completion_packet_id", "rpc_index", "array_ordinal", "update_ordinal",
         "component_ordinal")
SOURCE = ("initial_packet_id", "completion_packet_id", "channel_index",
          "actor_net_guid", "object_net_guid", "bunch_index", "block_index",
          "rpc_index", "rpc_handle", "array_ordinal", "update_ordinal",
          "update_index", "component_ordinal")
COLUMNS = (*KEY, *SCALARS, "move_count", "completion_packet",
           *FLOATS, *(name + "_bits" for name in FLOATS))
MAX_REFERENCE_MOVES = 16


class ComparisonError(ValueError):
    """Input cannot support the declared comparison contract."""


def uint(value, name, bits=32):
    if type(value) is not int or not 0 <= value < (1 << bits):
        raise ComparisonError(f"{name} must be an unsigned {bits}-bit integer")
    return value


def number(value, name):
    if type(value) not in (int, float):
        raise ComparisonError(f"{name} must be a finite number")
    try:
        result = float(value)
    except OverflowError as exc:
        raise ComparisonError(f"{name} must be a finite number") from exc
    if not math.isfinite(result):
        raise ComparisonError(f"{name} must be a finite number")
    return result


def f32(value, name):
    value = number(value, name)
    try:
        packed = struct.pack("<f", value)
    except (OverflowError, struct.error) as exc:
        raise ComparisonError(f"{name} is outside finite f32") from exc
    rounded = struct.unpack("<f", packed)[0]
    if not math.isfinite(rounded):
        raise ComparisonError(f"{name} is outside finite f32")
    return rounded, struct.unpack("<I", packed)[0]


def _object(pairs):
    obj = {}
    for name, value in pairs:
        if name in obj:
            raise ComparisonError("duplicate JSON member")
        obj[name] = value
    return obj


def _constant(_value):
    raise ComparisonError("non-standard JSON numeric constant")


def records(path, receipt, max_line_bytes):
    """Yield bounded strict JSON objects; hash the decoded JSONL bytes."""
    before = path.stat()
    digest = hashlib.sha256()
    lines = 0
    opener = gzip.open if path.suffix == ".gz" else open
    with opener(path, "rb") as stream:
        while line := stream.readline(max_line_bytes + 1):
            if len(line) > max_line_bytes:
                raise ComparisonError("JSONL line exceeds the configured byte limit")
            digest.update(line)
            lines += 1
            try:
                obj = json.loads(line, object_pairs_hook=_object, parse_constant=_constant)
            except (ValueError, UnicodeError, RecursionError) as exc:
                raise ComparisonError(f"invalid JSON object at line {lines}") from exc
            if not isinstance(obj, dict):
                raise ComparisonError(f"expected JSON object at line {lines}")
            yield obj
    after = path.stat()
    if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) != (
            after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns):
        raise ComparisonError("input changed during comparison")
    receipt.update(logical_content_sha256=digest.hexdigest(), lines=lines,
                   compressed=path.suffix == ".gz", file_bytes=after.st_size)


def _values(row, *, reference):
    if reference:
        if row.get("error_sentinel") is not False:
            raise ComparisonError("reference error sentinel must be false")
        if uint(row.get("mode_flags"), "mode_flags", 8) != uint(
                row.get("movement_state"), "movement_state", 8):
            raise ComparisonError("reference mode_flags differs from movement_state")
        vectors = []
        for field in ("position", "velocity"):
            value = row.get(field)
            if not isinstance(value, dict) or not all(k in value for k in ("x", "y", "z")):
                raise ComparisonError(f"reference {field} must contain x, y and z")
            vectors.append([value[axis] for axis in ("x", "y", "z")])
        values = [*vectors[0], row.get("yaw"), row.get("pitch"), *vectors[1]]
    else:
        values = [row.get(name) for name in FLOATS]
    actual = [number(v, name) for name, v in zip(FLOATS, values)]
    rounded = [f32(v, name) for name, v in zip(FLOATS, actual)]
    if not reference and any(v != q for v, (q, _bits) in zip(actual, rounded)):
        raise ComparisonError("Rust values must be exact f32 values expanded to JSON f64")
    move_type = uint(row.get("move_type"), "move_type", 8)
    if move_type != 1:
        raise ComparisonError("this comparison supports variant-1 moves only")
    scalars = (uint(row.get("shooter_character_net_guid" if reference else
                            "character_net_guid"), "character_net_guid"),
               uint(row.get("timestamp"), "timestamp"),
               uint(row.get("movement_state"), "movement_state", 8),
               move_type, uint(row.get("move_index"), "move_index"))
    return scalars, actual, [bits for _value, bits in rounded]


def _insert(connection, table, values):
    connection.execute(f"INSERT INTO {table} VALUES ({','.join('?' for _ in COLUMNS)})", values)


def load_reference(connection, path, receipt, max_rows, max_line_bytes):
    count = represented = 0
    for row in records(path, receipt, max_line_bytes):
        count += 1
        if count > max_rows:
            raise ComparisonError("reference row limit exceeded")
        if row.get("type") != "remote_character_movement":
            raise ComparisonError("reference file contains a non-movement row")
        key = tuple(uint(row.get(name), name) for name in
                    ("packet_id", "actor_net_guid", "object_net_guid", "channel", "update_index"))
        scalars, values, bits = _values(row, reference=True)
        moves = scalars[-1] + 1
        if moves > MAX_REFERENCE_MOVES:
            raise ComparisonError("reference exceeds its supported per-component move cap")
        represented += moves
        _insert(connection, "reference", (*key, *scalars, moves, key[0], *values, *bits))
    receipt.update(rows=count, represented_moves=represented)


def load_rust(connection, path, receipt, max_rows, max_line_bytes):
    iterator = records(path, receipt, max_line_bytes)
    header = next(iterator, None)
    if (header != {"type": "movement_provenance_header", "schema_version": 1,
                   "float_precision": "f32"}
            or type(header.get("schema_version")) is not int):
        raise ComparisonError("missing or unsupported movement provenance header")
    count = groups = fragmented = 0
    current = latest = previous_group = None
    moves = 0
    character = None
    trailer = None

    def flush():
        nonlocal groups
        if latest is not None:
            _insert(connection, "rust", (*latest[:10], moves, latest[10], *latest[11:]))
            groups += 1

    for row in iterator:
        if trailer is not None:
            raise ComparisonError("data follows the terminal Reader summary")
        if row.get("type") == "movement_provenance_summary":
            trailer = row
            continue
        if row.get("type") != "move":
            raise ComparisonError("unexpected movement provenance record")
        count += 1
        if count > max_rows:
            raise ComparisonError("Rust row limit exceeded")
        for name in SOURCE:
            if name not in row:
                raise ComparisonError(f"missing provenance member {name}")
            if name != "object_net_guid":
                uint(row.get(name), name, 63 if name == "bunch_index" else 32)
        object_guid = row.get("object_net_guid")
        if object_guid is not None:
            uint(object_guid, "object_net_guid")
        group = tuple(row[name] for name in GROUP)
        source = tuple(row.get(name) for name in SOURCE)
        if current is None or group != current[0]:
            flush()
            if previous_group is not None and group <= previous_group:
                raise ComparisonError("component provenance is repeated or out of emission order")
            previous_group = group
            current = (group, source)
            moves = 0
            character = None
        elif source != current[1]:
            raise ComparisonError("source identity changes inside a movement component")
        scalars, values, bits = _values(row, reference=False)
        if character is None:
            character = scalars[0]
        elif character != scalars[0]:
            raise ComparisonError("character identity changes inside a movement component")
        if scalars[-1] != moves:
            raise ComparisonError("component move indices must start at zero and be contiguous")
        moves += 1
        if moves > MAX_REFERENCE_MOVES:
            raise ComparisonError("Rust component exceeds the reference's 16-move contract")
        initial, completion = row["initial_packet_id"], row["completion_packet_id"]
        if initial > completion:
            raise ComparisonError("initial packet follows its completion packet")
        fragmented += initial != completion
        key = (initial, row["actor_net_guid"], row["actor_net_guid"] if object_guid is None
               else object_guid, row["channel_index"], row["update_index"])
        latest = (*key, *scalars, completion, *values, *bits)
    flush()
    if trailer is None:
        raise ComparisonError("missing successful terminal Reader summary")
    if uint(trailer.get("movement_rows"), "summary movement_rows", 63) != count:
        raise ComparisonError("Reader movement count disagrees with emitted rows")
    for name in ("movement_rpc_errors", "lost_content_blocks", "partial_errors", "unfinished_partials"):
        if uint(trailer.get(name), name, 63) != 0:
            raise ComparisonError(f"Reader reported nonzero {name}")
    if trailer.get("block_verdict") != "Passed":
        raise ComparisonError("Reader block verdict must be Passed")
    receipt.update(rows=count, component_groups=groups, fragmented_move_rows=fragmented,
                   skipped_checkpoint_chunks=uint(trailer.get("skipped_checkpoint_chunks"),
                                                  "skipped_checkpoint_chunks", 63))


def _duplicates(connection, table):
    fields = ",".join(KEY)
    groups, extra = connection.execute(
        f"SELECT COUNT(*),COALESCE(SUM(n-1),0) FROM "
        f"(SELECT COUNT(*) AS n FROM {table} GROUP BY {fields} HAVING COUNT(*)>1)"
    ).fetchone()
    return {"groups": groups, "extra_rows": extra}


def compare(reference_path, rust_path, *, max_rows=10_000_000,
            max_line_bytes=1_048_576, max_db_bytes=2_147_483_648, work_dir=None):
    """Compare complete inputs; return an aggregate with an explicit pass gate."""
    for value, name in ((max_rows, "max_rows"), (max_line_bytes, "max_line_bytes"),
                        (max_db_bytes, "max_db_bytes")):
        if type(value) is not int or value <= 0:
            raise ComparisonError(f"{name} must be positive")
    if max_db_bytes < 65536:
        raise ComparisonError("max_db_bytes must be at least 65536")
    reference_path, rust_path = Path(reference_path), Path(rust_path)
    report = {"schema_version": 1, "status": "fail", "reference": {}, "rust": {},
              "contract": "unique wire-update identity and f32-quantized latest-move parity",
              "join_fields": list(KEY),
              "numeric_values_used_for_join": False,
              "resource_limits": {"max_rows_per_input": max_rows,
                                  "max_line_bytes": max_line_bytes,
                                  "max_database_bytes": max_db_bytes,
                                  "sqlite_cache_kib": 16384},
              "limits": ["Main stream only; checkpoints are counted as skipped.",
                         "Only variant-1 successful movement components are supported.",
                         "Earlier moves are counted and indexed, but the reference does not emit their values.",
                         "Float parity is after f32 quantization; original reference deltas remain reported.",
                         "This does not validate fields absent from the vrfkit movement record."]}
    with tempfile.TemporaryDirectory(prefix="vrfkit-movement-", dir=work_dir) as directory:
        with closing(sqlite3.connect(Path(directory) / "comparison.sqlite")) as connection:
            connection.execute("PRAGMA page_size=4096")
            connection.execute(f"PRAGMA max_page_count={max_db_bytes // 4096}")
            connection.execute("PRAGMA cache_size=-16384")
            connection.execute("PRAGMA journal_mode=OFF")
            connection.execute("PRAGMA synchronous=OFF")
            definitions = ",".join(f"{name} {'REAL' if name in FLOATS else 'INTEGER'} NOT NULL"
                                   for name in COLUMNS)
            for table in ("rust", "reference"):
                connection.execute(f"CREATE TABLE {table} ({definitions})")
                connection.execute(f"CREATE INDEX {table}_identity ON {table} ({','.join(KEY)})")
            load_rust(connection, rust_path, report["rust"], max_rows, max_line_bytes)
            load_reference(connection, reference_path, report["reference"], max_rows, max_line_bytes)
            connection.commit()
            for name in ("rust", "reference"):
                report[name]["duplicate_identities"] = _duplicates(connection, name)
            join = " AND ".join(f"r.{name}=c.{name}" for name in KEY)
            missing = connection.execute(
                f"SELECT COUNT(*) FROM reference c WHERE NOT EXISTS (SELECT 1 FROM rust r WHERE {join})"
            ).fetchone()[0]
            extra = connection.execute(
                f"SELECT COUNT(*) FROM rust r WHERE NOT EXISTS (SELECT 1 FROM reference c WHERE {join})"
            ).fetchone()[0]
            report.update(missing_reference_updates=missing, extra_rust_components=extra)
            duplicates = any(report[name]["duplicate_identities"]["groups"]
                             for name in ("rust", "reference"))
            if duplicates:
                report["value_comparison"] = {"status": "not_run_ambiguous_identity"}
            else:
                expressions = ["COUNT(*)"]
                checked = (*SCALARS, "move_count", *(name + "_bits" for name in FLOATS))
                expressions += [f"COALESCE(SUM(r.{name}!=c.{name}),0)" for name in checked]
                expressions += [f"MAX(ABS(r.{name}-c.{name}))" for name in FLOATS]
                result = connection.execute(
                    f"SELECT {','.join(expressions)} FROM reference c JOIN rust r ON {join}"
                ).fetchone()
                mismatches = dict(zip(checked, result[1:1 + len(checked)]))
                report["value_comparison"] = {
                    "status": "complete", "matched_updates": result[0],
                    "mismatch_counts": mismatches,
                    "maximum_absolute_reference_delta": dict(zip(FLOATS, result[1 + len(checked):]))}
                if (result[0] > 0 and not missing and not extra and not any(mismatches.values())
                        and report["rust"]["rows"] == report["reference"]["represented_moves"]):
                    report["status"] = "pass"
            report["database_bytes"] = (Path(directory) / "comparison.sqlite").stat().st_size
    return report


def _error_report(error_type, error):
    return {"schema_version": 1, "status": "error", "error_type": error_type,
            "error": error, "comparison_complete": False}


def _publish(path, report):
    atomic_write_text(path, json.dumps(report, indent=2, allow_nan=False) + "\n")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path, help="reference movement.ndjson (or .gz)")
    parser.add_argument("rust", type=Path, help="movement_provenance example JSONL (or .gz)")
    parser.add_argument("--out", type=Path, required=True, help="aggregate JSON report")
    parser.add_argument("--work-dir", type=Path, help="parent for private temporary indexed storage")
    parser.add_argument("--max-rows", type=int, default=10_000_000)
    parser.add_argument("--max-line-bytes", type=int, default=1_048_576)
    parser.add_argument("--max-db-bytes", type=int, default=2_147_483_648)
    args = parser.parse_args(argv)
    try:
        refuse_input_path(args.out, (args.reference, args.rust, Path(__file__),
                                    Path(__file__).with_name("atomic_io.py")))
    except (OSError, ValueError) as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        return 2
    # Replace any earlier report before reading input: an exception this
    # handler does not name must leave `error` behind, never a previous `pass`.
    try:
        _publish(args.out, _error_report("Interrupted", "comparison did not finish"))
    except (OSError, ValueError):
        print("FAILED: aggregate report could not be published", file=sys.stderr)
        return 2
    try:
        report = compare(args.reference, args.rust, max_rows=args.max_rows,
                         max_line_bytes=args.max_line_bytes, max_db_bytes=args.max_db_bytes,
                         work_dir=args.work_dir)
        report["tool_sha256"] = sha256_file(Path(__file__))
    # A truncated gzip member raises EOFError; corrupt deflate data, zlib.error.
    except (OSError, ValueError, KeyError, TypeError, EOFError, zlib.error,
            sqlite3.Error) as exc:
        report = _error_report(type(exc).__name__, os.strerror(exc.errno)
                               if isinstance(exc, OSError) and exc.errno else str(exc))
    try:
        _publish(args.out, report)
    except (OSError, ValueError):
        print("FAILED: aggregate report could not be published", file=sys.stderr)
        return 2
    print(f"movement correspondence: {report['status']}")
    return 0 if report["status"] == "pass" else 2 if report["status"] == "error" else 1


if __name__ == "__main__":
    raise SystemExit(main())
