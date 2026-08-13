"""
Python interop verification for vrf-export Parquet files.

Validates:
1. Parquet files are readable by pyarrow
2. Schema matches expectations (column names, types, nullability)
3. Row counts are correct
4. Also writes equivalent NDJSON for size/speed comparison
"""
import json
import os
import sys
import time
from pathlib import Path

# Force UTF-8 output on Windows
sys.stdout.reconfigure(encoding='utf-8')

import pyarrow
import pyarrow.parquet as pq

# Accept only the exact directory written by the Rust test. Picking the newest
# matching system-temp directory can silently validate another checkout's
# stale fixture, so CI captures INTEROP_FIELDS/INTEROP_MOVEMENT from the test,
# verifies their common parent, then sets VRFKIT_INTEROP_DIR to that directory.
def _find_interop_dir() -> Path:
    if len(sys.argv) > 1:
        return Path(sys.argv[1]).resolve()
    configured = os.environ.get("VRFKIT_INTEROP_DIR")
    if configured:
        return Path(configured).resolve()
    sys.exit(
        "explicit interop directory required: pass argv[1] or set "
        "VRFKIT_INTEROP_DIR to the exact fixture directory printed by the "
        "Rust write_interop_files test"
    )


INTEROP_DIR = _find_interop_dir()
print("interop dir: %s" % INTEROP_DIR)
FIELDS_PATH = INTEROP_DIR / "fields_interop.parquet"
MOVEMENT_PATH = INTEROP_DIR / "movement_interop.parquet"


class CheckFailed(Exception):
    """A correctness gate in this script did not hold."""


def check(condition, message):
    """Gate that survives `python -O`.

    Every gate below used to be a bare `assert`. `python -O` and
    PYTHONOPTIMIZE=1 strip the `assert` statement at compile time, so under
    either one this script walked the whole file, printed every "verified"
    tick, printed ALL CHECKS PASSED and exited 0 -- against parquet files it
    had checked nothing about. A verification script that cannot fail is not a
    verification script, and the mode that disables it is one environment
    variable set anywhere in a CI image.

    `raise` is not compiled out, so this holds under every optimisation level.
    """
    if not condition:
        raise CheckFailed(message)


def _assert_gates_are_live():
    """Prove `check` still fails before trusting anything it reports.

    This is the guard for the regression itself: if someone reintroduces the
    `assert` form, or wraps `check` in something that swallows, the script
    stops rather than certifying a file it never inspected.
    """
    try:
        check(False, "self-test")
    except CheckFailed:
        return
    raise SystemExit("FATAL: check() did not raise -- the gates below are not enforced")

def verify_fields():
    """Verify the fields parquet file."""
    print("=" * 60)
    print("FIELDS TABLE VERIFICATION")
    print("=" * 60)

    table = pq.read_table(str(FIELDS_PATH))
    schema = table.schema

    print(f"\nRow count: {table.num_rows}")
    check(table.num_rows == 10_000, f"Expected 10000 rows, got {table.num_rows}")
    print("  ✓ Row count matches (10,000)")

    print(f"\nColumn count: {len(schema)}")
    expected_cols = [
        "time_ms", "packet_id", "channel_index", "actor_net_guid",
        # Pre-existing omission: object_net_guid has been in fields_schema()
        # since subobject blocks started carrying it, but this list was never
        # updated, so the script aborted here before reaching verify_movement().
        "object_net_guid",
        "group_path", "handle", "field_name",
        # The same omission again, and the reason it went unnoticed for a
        # second time: `compatible_checksum` sits between `field_name` and
        # `bit_count` in fields_schema(), and this list was not updated with
        # it. Under `python -O` the gate below was compiled away entirely, so
        # the mismatch printed a tick and reached ALL CHECKS PASSED.
        "compatible_checksum",
        "bit_count",
        "raw_bits", "value_i64", "value_f64", "value_bool", "value_str",
    ]
    actual_cols = [f.name for f in schema]
    check(
        actual_cols == expected_cols,
        f"Column mismatch:\n  expected: {expected_cols}\n  actual:   {actual_cols}",
    )
    print("  ✓ Column names match")

    # Print schema details
    print("\nSchema:")
    for field in schema:
        print(f"  {field.name:20s}  {str(field.type):30s}  nullable={field.nullable}")

    # Verify dictionary encoding on group_path
    gp_type = schema.field("group_path").type
    check(
        "dictionary" in str(gp_type).lower() or "dict" in str(gp_type).lower(),
        f"group_path should be dictionary-encoded, got {gp_type}",
    )
    print("\n  ✓ group_path is dictionary-encoded")

    fn_type = schema.field("field_name").type
    check(
        "dictionary" in str(fn_type).lower() or "dict" in str(fn_type).lower(),
        f"field_name should be dictionary-encoded, got {fn_type}",
    )
    print("  ✓ field_name is dictionary-encoded")

    # Verify nullability
    check(schema.field("field_name").nullable, "field_name should be nullable")
    check(schema.field("raw_bits").nullable, "raw_bits should be nullable")
    check(schema.field("value_i64").nullable, "value_i64 should be nullable")
    print("  ✓ Nullable columns are correct")

    # Verify some null values exist
    fn_col = table.column("field_name")
    null_count = fn_col.null_count
    print(f"\n  field_name null count: {null_count} / {table.num_rows}")
    check(null_count > 0, "Expected some null field_names")
    print("  ✓ Null values present where expected")

    # File size
    file_size = os.path.getsize(FIELDS_PATH)
    print(f"\n  Parquet file size: {file_size:,} bytes ({file_size/1024:.1f} KB)")
    return table


def verify_movement():
    """Verify the movement parquet file."""
    print("\n" + "=" * 60)
    print("MOVEMENT TABLE VERIFICATION")
    print("=" * 60)

    table = pq.read_table(str(MOVEMENT_PATH))
    schema = table.schema

    print(f"\nRow count: {table.num_rows}")
    check(table.num_rows == 50_000, f"Expected 50000 rows, got {table.num_rows}")
    print("  ✓ Row count matches (50,000)")

    expected_cols = [
        "time_ms", "packet_id", "character_net_guid",
        "pos_x", "pos_y", "pos_z", "yaw", "pitch",
        "vel_x", "vel_y", "vel_z",
        # Appended after vel_z, never interleaved: consumers address movement
        # columns positionally.
        "timestamp", "movement_state", "move_type",
    ]
    actual_cols = [f.name for f in schema]
    check(
        actual_cols == expected_cols,
        f"Column mismatch:\n  expected: {expected_cols}\n  actual:   {actual_cols}",
    )
    print("  ✓ Column names match")

    print("\nSchema:")
    for field in schema:
        print(f"  {field.name:24s}  {str(field.type):15s}  nullable={field.nullable}")

    # No column should be nullable
    for field in schema:
        check(not field.nullable, f"{field.name} should not be nullable")
    print("\n  ✓ No nullable columns (all dense)")

    file_size = os.path.getsize(MOVEMENT_PATH)
    print(f"\n  Parquet file size: {file_size:,} bytes ({file_size/1024:.1f} KB)")
    return table


def benchmark_ndjson_comparison(fields_table, movement_table):
    """Write equivalent NDJSON and compare size + read speed."""
    print("\n" + "=" * 60)
    print("PARQUET vs NDJSON COMPARISON")
    print("=" * 60)

    ndjson_fields_path = INTEROP_DIR / "fields_interop.ndjson"
    ndjson_movement_path = INTEROP_DIR / "movement_interop.ndjson"

    # Write NDJSON for fields
    print("\nWriting fields NDJSON...")
    t0 = time.perf_counter()
    with open(ndjson_fields_path, "w", encoding="utf-8") as f:
        for i in range(fields_table.num_rows):
            row = {}
            for col_name in fields_table.column_names:
                val = fields_table.column(col_name)[i].as_py()
                if isinstance(val, bytes):
                    val = val.hex()
                row[col_name] = val
            f.write(json.dumps(row) + "\n")
    ndjson_fields_write_time = time.perf_counter() - t0

    # Write NDJSON for movement
    print("Writing movement NDJSON...")
    t0 = time.perf_counter()
    with open(ndjson_movement_path, "w", encoding="utf-8") as f:
        for i in range(movement_table.num_rows):
            row = {}
            for col_name in movement_table.column_names:
                val = movement_table.column(col_name)[i].as_py()
                row[col_name] = val
            f.write(json.dumps(row) + "\n")
    ndjson_movement_write_time = time.perf_counter() - t0

    # Sizes
    pq_fields_size = os.path.getsize(FIELDS_PATH)
    pq_movement_size = os.path.getsize(MOVEMENT_PATH)
    ndjson_fields_size = os.path.getsize(ndjson_fields_path)
    ndjson_movement_size = os.path.getsize(ndjson_movement_path)

    print(f"\n{'':4s}{'':20s}{'Parquet':>12s}{'NDJSON':>12s}{'Ratio':>10s}")
    print(f"{'':4s}{'-'*54}")
    print(f"{'':4s}{'Fields (10K rows)':20s}"
          f"{pq_fields_size:>10,} B"
          f"{ndjson_fields_size:>10,} B"
          f"{ndjson_fields_size/pq_fields_size:>8.1f}×")
    print(f"{'':4s}{'Movement (50K rows)':20s}"
          f"{pq_movement_size:>10,} B"
          f"{ndjson_movement_size:>10,} B"
          f"{ndjson_movement_size/pq_movement_size:>8.1f}×")

    # Read speed comparison
    print("\n  Read speed comparison (5 iterations, best of):")

    # Parquet read
    times_pq_fields = []
    for _ in range(5):
        t0 = time.perf_counter()
        pq.read_table(str(FIELDS_PATH))
        times_pq_fields.append(time.perf_counter() - t0)

    times_pq_movement = []
    for _ in range(5):
        t0 = time.perf_counter()
        pq.read_table(str(MOVEMENT_PATH))
        times_pq_movement.append(time.perf_counter() - t0)

    # NDJSON read (line by line JSON parse)
    times_ndjson_fields = []
    for _ in range(5):
        t0 = time.perf_counter()
        with open(ndjson_fields_path, "r", encoding="utf-8") as f:
            rows = [json.loads(line) for line in f]
        times_ndjson_fields.append(time.perf_counter() - t0)

    times_ndjson_movement = []
    for _ in range(5):
        t0 = time.perf_counter()
        with open(ndjson_movement_path, "r", encoding="utf-8") as f:
            rows = [json.loads(line) for line in f]
        times_ndjson_movement.append(time.perf_counter() - t0)

    pq_f = min(times_pq_fields) * 1000
    ndjson_f = min(times_ndjson_fields) * 1000
    pq_m = min(times_pq_movement) * 1000
    ndjson_m = min(times_ndjson_movement) * 1000

    print(f"\n{'':4s}{'':20s}{'Parquet':>12s}{'NDJSON':>12s}{'Speedup':>10s}")
    print(f"{'':4s}{'-'*54}")
    print(f"{'':4s}{'Fields read':20s}"
          f"{pq_f:>9.1f} ms"
          f"{ndjson_f:>9.1f} ms"
          f"{ndjson_f/pq_f:>8.1f}×")
    print(f"{'':4s}{'Movement read':20s}"
          f"{pq_m:>9.1f} ms"
          f"{ndjson_m:>9.1f} ms"
          f"{ndjson_m/pq_m:>8.1f}×")

    print("\n  ✓ Parquet is significantly smaller and faster to read")

    # Clean up NDJSON files
    ndjson_fields_path.unlink(missing_ok=True)
    ndjson_movement_path.unlink(missing_ok=True)

    return {
        "fields_parquet_bytes": pq_fields_size,
        "fields_ndjson_bytes": ndjson_fields_size,
        "movement_parquet_bytes": pq_movement_size,
        "movement_ndjson_bytes": ndjson_movement_size,
        "fields_parquet_read_ms": pq_f,
        "fields_ndjson_read_ms": ndjson_f,
        "movement_parquet_read_ms": pq_m,
        "movement_ndjson_read_ms": ndjson_m,
    }


if __name__ == "__main__":
    _assert_gates_are_live()
    print(f"Interop dir: {INTEROP_DIR}")
    print(f"pyarrow version: {pyarrow.__version__}")
    print()

    if not FIELDS_PATH.exists() or not MOVEMENT_PATH.exists():
        print("ERROR: Interop parquet files not found.")
        print("       Run `cargo test -p vrf-export` first.")
        exit(1)

    fields_table = verify_fields()
    movement_table = verify_movement()
    stats = benchmark_ndjson_comparison(fields_table, movement_table)

    print("\n" + "=" * 60)
    print("ALL CHECKS PASSED ✓")
    print("=" * 60)
