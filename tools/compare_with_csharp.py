"""Compare vrfkit (Rust) parser output against ValorantReplayParser (C#) output.

Why this exists:
    vrfkit aims to reproduce and EXCEED the C# parser's field coverage. This tool
    quantifies exactly what matches, what vrfkit adds (expected — C# drops groups
    without descriptors), and what vrfkit misses (bugs to fix). It operates in
    streaming mode to handle 86+ MB NDJSON files without loading them into memory.

Usage (the exact command used to produce the first report):
    python tools/compare_with_csharp.py ^
        "<VRFKIT_VALPLAY_DIR>\\pipeline\\exports\\02d4d478-1dfb-4412-9a77-29ca29105a9d" ^
        "<repo-root>\\out\\02d4d478"

Arguments:
    csharp_dir   Path to the C# bundle (manifest.json, events.ndjson, movement.ndjson)
    vrfkit_dir   Path to vrfkit output  (manifest.json, fields.parquet, movement.parquet)

Important caveat — slim_export.py:
    The C# bundle is typically "slimmed": ~97% of rpc_received dropped, and keys
    (was_decoded, diagnostic_fields, categories, parsed_bits, decoded_field_count)
    removed from kept events. movement.ndjson also loses 5 constant fields (type,
    actor_net_guid, movement_state, mode_flags, move_type). The full pre-slim counts
    survive in manifest.json — this tool uses those for total-count comparisons.
"""

from __future__ import annotations

import json
import sys
from collections import Counter, defaultdict, deque
from pathlib import Path
from typing import Iterator

import pyarrow.parquet as pq

sys.path.insert(0, str(Path(__file__).resolve().parent))
from to_valplay_bundle import (  # noqa: E402
    UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME,
)


# ─── Helpers ───────────────────────────────────────────────────────────────────

def iter_ndjson(path: Path) -> Iterator[dict]:
    """Yield parsed dicts from an NDJSON file, one line at a time (streaming)."""
    with path.open("r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                yield json.loads(line)


# ─── 1. Total-count comparison ────────────────────────────────────────────────

def compare_totals(cs_manifest: dict, vk_manifest: dict) -> str:
    """Compare packet/bunch/actor/export-group counts from both manifests."""
    lines = ["## 1. Total-count comparison\n"]
    # Map C# keys → vrfkit keys
    mapping = [
        ("Packets",        "packet_count",        "packet_count"),
        ("Bunches",        "packets_with_bunches","bunch_count"),
        ("Actor opens",    "actor_spawned",       "actor_opens"),
        ("Actor closes",   "actor_closed",        "actor_closes"),
        ("Export groups",  None,                  None),  # special
    ]
    cs_stats = cs_manifest.get("stats", {})
    cs_counts = cs_manifest.get("counts", {})
    vk_stats = vk_manifest.get("stats", {})
    vk_counts = vk_manifest.get("counts", {})

    lines.append(f"{'Metric':<20} {'C#':>12} {'vrfkit':>12} {'Match':>6}")
    lines.append("-" * 55)

    def row(label, cs_val, vk_val):
        match = "✓" if cs_val == vk_val else "✗"
        lines.append(f"{label:<20} {cs_val:>12,} {vk_val:>12,} {match:>6}")

    row("Packets",       cs_stats.get("packet_count", 0),        vk_stats.get("packet_count", 0))
    row("Bunches",       cs_stats.get("packets_with_bunches", cs_stats.get("bunch_count", 0)),
                         vk_stats.get("bunch_count", vk_counts.get("bunch_count", 0)))
    row("Actor opens",   cs_counts.get("actor_spawned", 0),      vk_counts.get("actor_opens", 0))
    row("Actor closes",  cs_counts.get("actor_closed", 0),       vk_counts.get("actor_closes", 0))

    # Export groups — count from net_field_export_groups array length
    cs_groups = len(cs_manifest.get("net_field_export_groups", []))
    vk_groups = len(vk_manifest.get("net_field_export_groups", []))
    row("Export groups",  cs_groups, vk_groups)

    # Additional counts
    row("Movement rows", cs_counts.get("movement", 0),
        vk_counts.get("movement_rows", vk_manifest.get("movement_rows", 0)))
    row("Fields decoded", cs_counts.get("export_group_received", 0) + cs_counts.get("filtered_export_groups", 0),
        vk_counts.get("fields", 0))
    row("RPCs (total)",  cs_counts.get("rpc_received", 0),       vk_counts.get("rpcs", 0))

    lines.append("")
    return "\n".join(lines)


# ─── 2. Export group path set comparison ──────────────────────────────────────

def compare_group_paths(cs_manifest: dict, vk_manifest: dict,
                        vk_parquet_path: Path) -> str:
    """Compare the set of export group paths between both manifests."""
    lines = ["## 2. Export group path set comparison\n"]

    cs_paths = {g["path"] for g in cs_manifest.get("net_field_export_groups", [])}
    vk_paths = {g["path"] for g in vk_manifest.get("net_field_export_groups", [])}

    # Also check what's actually in the parquet
    if vk_parquet_path.exists():
        tbl = pq.read_table(vk_parquet_path, columns=["group_path"])
        vk_parquet_paths = set(tbl.column("group_path").to_pylist())
        lines.append(f"vrfkit manifest groups: {len(vk_paths)}")
        lines.append(f"vrfkit parquet distinct group_path: {len(vk_parquet_paths)}")
    else:
        vk_parquet_paths = set()
        lines.append("(fields.parquet not found — using manifest only)")

    both = cs_paths & vk_paths
    cs_only = cs_paths - vk_paths
    vk_only = vk_paths - cs_paths

    lines.append(f"\nC# manifest groups: {len(cs_paths)}")
    lines.append(f"vrfkit manifest groups: {len(vk_paths)}")
    lines.append(f"Both: {len(both)}")
    lines.append(f"C# only: {len(cs_only)}")
    lines.append(f"vrfkit only: {len(vk_only)}")

    if cs_only:
        lines.append(f"\n### C# only ({len(cs_only)}):")
        for p in sorted(cs_only)[:50]:
            lines.append(f"  {p}")
        if len(cs_only) > 50:
            lines.append(f"  ... and {len(cs_only) - 50} more")

    if vk_only:
        lines.append(f"\n### vrfkit only ({len(vk_only)}):")
        for p in sorted(vk_only)[:50]:
            lines.append(f"  {p}")
        if len(vk_only) > 50:
            lines.append(f"  ... and {len(vk_only) - 50} more")

    lines.append("")
    return "\n".join(lines)


# ─── 3. (Group, Field) coverage comparison ───────────────────────────────────

def coverage_problems(cs_pairs: set, vk_pairs: set) -> list[str]:
    """Why this comparison compared nothing, if it compared nothing.

    This script is a REPORT: vrfkit deliberately exports more than the C#
    parser, so "vrfkit only" is expected and no threshold on the differences
    can be defended without the corpus in hand. What a report may still not do
    is announce a result it never measured, and it did: `cs_only` is empty when
    the C# side yielded NO pairs at all -- an empty, slimmed, or wrong
    events.ndjson -- and the section then printed "vrfkit covers everything C#
    has".

    C#-only pairs are deliberately NOT reported here. They are the report's
    subject, listed in full under INVESTIGATE; gating on them would make a tool
    that measures known-incomplete coverage permanently red.
    """
    if not cs_pairs and not vk_pairs:
        return ["neither side produced a single (group, field) pair: "
                "nothing was compared"]
    if not cs_pairs:
        return ["the C# side produced no (group, field) pairs at all, so "
                "'vrfkit covers everything C# has' would be vacuous"]
    if not vk_pairs:
        return ["the vrfkit side produced no (group, field) pairs at all: "
                "check fields.parquet"]
    if not (cs_pairs & vk_pairs):
        return [f"the two sides share no (group, field) pair at all "
                f"({len(cs_pairs):,} C#, {len(vk_pairs):,} vrfkit): total "
                f"disagreement, not a coverage difference"]
    return []


def coverage_lines(cs_pairs: set, vk_pairs: set) -> list[str]:
    """The C#-only / vrfkit-only breakdown, split out so it can be tested."""
    both = cs_pairs & vk_pairs
    cs_only = cs_pairs - vk_pairs
    vk_only = vk_pairs - cs_pairs

    lines = [
        f"\n  Both (intersection): {len(both):,}",
        f"  C# only:             {len(cs_only):,}",
        f"  vrfkit only:         {len(vk_only):,}",
        f"\n### Both -- sample (first 30 of {len(both):,}):",
    ]
    lines += [f"  ({gp}, {fn})" for gp, fn in sorted(both)[:30]]

    if cs_only:
        lines.append(f"\n### C# only -- ALL {len(cs_only)} entries (INVESTIGATE):")
        lines += [f"  ({gp}, {fn})" for gp, fn in sorted(cs_only)]
    elif cs_pairs:
        lines.append("\n### C# only: NONE -- vrfkit covers everything C# has!")
    else:
        # The claim above is only meaningful when the C# side had pairs to
        # miss. Without them an empty difference is an empty measurement.
        lines.append("\n### C# only: NOT MEASURED -- the C# side produced no "
                     "pairs, so this says nothing about coverage.")

    lines.append(f"\n### vrfkit only -- sample (first 30 of {len(vk_only):,}):")
    lines += [f"  ({gp}, {fn})" for gp, fn in sorted(vk_only)[:30]]
    if len(vk_only) > 30:
        lines.append(f"  ... and {len(vk_only) - 30} more")
    return lines


def compare_group_field_coverage(cs_events_path: Path, vk_parquet_path: Path):
    """Compare (group_path, field_name) pairs between C# events and vrfkit parquet.

    Returns `(report text, problems)`.
    """
    lines = ["## 3. (Group, Field) coverage comparison\n"]

    # Collect C# (group, field) pairs from export_group_received events
    cs_pairs: set[tuple[str, str]] = set()
    lines.append("Scanning C# events.ndjson for export_group_received...")
    count = 0
    for obj in iter_ndjson(cs_events_path):
        if obj.get("type") != "export_group_received":
            continue
        count += 1
        group_path = obj.get("export_group_path", "")
        payload = obj.get("payload", {})
        if isinstance(payload, dict):
            for key in payload.keys():
                cs_pairs.add((group_path, key))

    lines.append(f"  Scanned {count:,} export_group_received records")
    lines.append(f"  Distinct (group, field) pairs from C#: {len(cs_pairs):,}")

    # Collect vrfkit (group, field) pairs from fields.parquet
    if not vk_parquet_path.exists():
        lines.append("  fields.parquet not found — cannot compare.")
        return "\n".join(lines), [f"fields.parquet not found at {vk_parquet_path}"]

    tbl = pq.read_table(vk_parquet_path, columns=["group_path", "field_name"])
    vk_pairs: set[tuple[str, str]] = set()
    for gp, fn in zip(tbl.column("group_path").to_pylist(),
                       tbl.column("field_name").to_pylist()):
        # A preserved whole-block payload is not a field, so it is not a pair
        # this comparison is about. Without this it lands in "vrfkit only" for
        # every unresolved group and reads as coverage we do not have.
        if fn == UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME:
            continue
        vk_pairs.add((gp, fn))

    lines.append(f"  Distinct (group, field) pairs from vrfkit: {len(vk_pairs):,}")

    lines += coverage_lines(cs_pairs, vk_pairs)
    lines.append("")
    return "\n".join(lines), coverage_problems(cs_pairs, vk_pairs)


# ─── 4. RPC name comparison ──────────────────────────────────────────────────

def compare_rpc_names(cs_events_path: Path, vk_manifest: dict,
                      vk_parquet_path: Path) -> str:
    """Compare RPC function names between the two parsers."""
    lines = ["## 4. RPC name comparison\n"]
    lines.append("NOTE: C# events.ndjson is slimmed — only ~3% of RPCs survive.")
    lines.append("      Full RPC count comes from manifest.json counts.rpc_received.")

    # Collect function_names from slim events
    cs_rpc_names: Counter = Counter()
    for obj in iter_ndjson(cs_events_path):
        if obj.get("type") != "rpc_received":
            continue
        fname = obj.get("function_name", "<unknown>")
        cs_rpc_names[fname] += 1

    lines.append(f"\n  C# slim RPC distinct names: {len(cs_rpc_names)}")
    lines.append(f"  C# slim RPC total records: {sum(cs_rpc_names.values()):,}")

    # vrfkit RPCs — check if manifest has rpc breakdown
    vk_rpcs = vk_manifest.get("rpcs_by_name", {})
    if vk_rpcs:
        lines.append(f"  vrfkit manifest rpcs_by_name: {len(vk_rpcs)} distinct")
    else:
        lines.append("  vrfkit manifest does NOT have rpcs_by_name breakdown.")
        # Try to get RPC names from parquet — check schema
        if vk_parquet_path.exists():
            schema = pq.read_schema(vk_parquet_path)
            field_names = [f.name for f in schema]
            lines.append(f"  fields.parquet columns: {field_names}")
            # Check if there's an rpc_name column or similar
            if "rpc_name" in field_names:
                tbl = pq.read_table(vk_parquet_path, columns=["rpc_name"])
                # filter non-null
                rpc_col = tbl.column("rpc_name")
                vk_rpc_counter: Counter = Counter()
                for val in rpc_col.to_pylist():
                    if val is not None:
                        vk_rpc_counter[val] += 1
                lines.append(f"  vrfkit parquet distinct RPC names: {len(vk_rpc_counter)}")
            else:
                lines.append("  No rpc_name column in fields.parquet — cannot extract RPC names from parquet.")

    lines.append("\n  C# slim RPC function_name breakdown:")
    for name, count in cs_rpc_names.most_common():
        lines.append(f"    {name}: {count:,}")

    lines.append("")
    return "\n".join(lines)


# ─── 5. Movement value comparison ────────────────────────────────────────────

def compare_movement(cs_movement_path: Path, vk_movement_path: Path) -> str:
    """Compare movement data by joining on (time_ms, character_net_guid)."""
    lines = ["## 5. Movement value comparison\n"]

    if not cs_movement_path.exists():
        lines.append("C# movement.ndjson not found.")
        return "\n".join(lines)
    if not vk_movement_path.exists():
        lines.append("vrfkit movement.parquet not found.")
        return "\n".join(lines)

    # Count C# rows
    cs_row_count = 0
    with cs_movement_path.open("r", encoding="utf-8") as f:
        for _ in f:
            cs_row_count += 1
    lines.append(f"C# movement rows (in file): {cs_row_count:,}")

    # vrfkit movement rows
    vk_meta = pq.read_metadata(vk_movement_path)
    vk_row_count = vk_meta.num_rows
    lines.append(f"vrfkit movement rows: {vk_row_count:,}")
    lines.append(f"Difference: {vk_row_count - cs_row_count:,} (vrfkit - C#)")

    # Schema check
    schema = pq.read_schema(vk_movement_path)
    lines.append(f"vrfkit movement columns: {[f.name for f in schema]}")

    # Sample-based comparison: read first 50000 rows from both and join
    # Join method: exact match on (time_ms, shooter_character_net_guid)
    # with ±1ms tolerance fallback
    SAMPLE_SIZE = 50000

    lines.append(f"\nSampling first {SAMPLE_SIZE:,} C# rows for value comparison...")

    # Read C# sample
    cs_sample: list[tuple[int, int, dict]] = []
    cs_read = 0
    for obj in iter_ndjson(cs_movement_path):
        if cs_read >= SAMPLE_SIZE:
            break
        time_ms = obj.get("time_ms")
        char_guid = obj.get("shooter_character_net_guid")
        if time_ms is not None and char_guid is not None:
            cs_sample.append((time_ms, char_guid, obj))
        cs_read += 1

    # Read vrfkit sample (same time range)
    if cs_sample:
        min_time = min(row[0] for row in cs_sample)
        max_time = max(row[0] for row in cs_sample)
    else:
        lines.append("No C# samples found.")
        return "\n".join(lines)

    # Read the parquet for the same time range using filters
    import pyarrow.dataset as ds
    dataset = ds.dataset(vk_movement_path)
    # Read rows in the time range
    vk_tbl = dataset.to_table(
        filter=(ds.field("time_ms") >= min_time) & (ds.field("time_ms") <= max_time + 1)
    )
    vk_cols = [f.name for f in vk_tbl.schema]

    # Build vrfkit lookup — (time_ms, character_net_guid)
    vk_sample: defaultdict[tuple[int, int], deque[int]] = defaultdict(deque)
    # Determine the character GUID column name in vrfkit
    char_col = None
    for candidate in ["character_net_guid", "shooter_character_net_guid", "char_net_guid"]:
        if candidate in vk_cols:
            char_col = candidate
            break
    if char_col is None:
        lines.append(f"Cannot find character GUID column in vrfkit. Columns: {vk_cols}")
        return "\n".join(lines)

    time_col = vk_tbl.column("time_ms").to_pylist()
    guid_col = vk_tbl.column(char_col).to_pylist()
    # Collect all columns into row dicts for joined entries
    for i in range(vk_tbl.num_rows):
        vk_sample[(time_col[i], guid_col[i])].append(i)
    # Precompute column arrays
    vk_columns_data = {}
    for col_name in vk_cols:
        vk_columns_data[col_name] = vk_tbl.column(col_name).to_pylist()

    # Join: exact, then ±1ms fallback
    joined = 0
    missed = 0
    errors_pos = []
    errors_yaw = []
    errors_pitch = []
    errors_vel = []

    for t, g, cs_row in cs_sample:
        vk_idx = None
        for key in ((t, g), (t - 1, g), (t + 1, g)):
            candidates = vk_sample.get(key)
            if candidates:
                vk_idx = candidates.popleft()
                break
        if vk_idx is None:
            missed += 1
            continue
        joined += 1

        # Compare position
        cs_pos = cs_row.get("position", {})
        if "pos_x" in vk_cols:
            vk_x = vk_columns_data["pos_x"][vk_idx]
            vk_y = vk_columns_data["pos_y"][vk_idx]
            vk_z = vk_columns_data["pos_z"][vk_idx]
        elif "position_x" in vk_cols:
            vk_x = vk_columns_data["position_x"][vk_idx]
            vk_y = vk_columns_data["position_y"][vk_idx]
            vk_z = vk_columns_data["position_z"][vk_idx]
        else:
            vk_x = vk_y = vk_z = None

        if isinstance(cs_pos, dict) and vk_x is not None:
            dx = abs((cs_pos.get("x", 0) or 0) - (vk_x or 0))
            dy = abs((cs_pos.get("y", 0) or 0) - (vk_y or 0))
            dz = abs((cs_pos.get("z", 0) or 0) - (vk_z or 0))
            errors_pos.append(max(dx, dy, dz))

        # Yaw / Pitch
        cs_yaw = cs_row.get("yaw", 0) or 0
        cs_pitch = cs_row.get("pitch", 0) or 0
        if "yaw" in vk_cols:
            vk_yaw = vk_columns_data["yaw"][vk_idx] or 0
            vk_pitch = vk_columns_data["pitch"][vk_idx] or 0
            errors_yaw.append(abs(cs_yaw - vk_yaw))
            errors_pitch.append(abs(cs_pitch - vk_pitch))

        # Velocity
        cs_vel = cs_row.get("velocity", {})
        if "vel_x" in vk_cols:
            vvx = vk_columns_data["vel_x"][vk_idx]
            vvy = vk_columns_data["vel_y"][vk_idx]
            vvz = vk_columns_data["vel_z"][vk_idx]
        elif "velocity_x" in vk_cols:
            vvx = vk_columns_data["velocity_x"][vk_idx]
            vvy = vk_columns_data["velocity_y"][vk_idx]
            vvz = vk_columns_data["velocity_z"][vk_idx]
        else:
            vvx = vvy = vvz = None

        if isinstance(cs_vel, dict) and vvx is not None:
            dvx = abs((cs_vel.get("x", 0) or 0) - (vvx or 0))
            dvy = abs((cs_vel.get("y", 0) or 0) - (vvy or 0))
            dvz = abs((cs_vel.get("z", 0) or 0) - (vvz or 0))
            errors_vel.append(max(dvx, dvy, dvz))

    lines.append(f"  Joined: {joined:,} / {len(cs_sample):,} ({100*joined/max(1,len(cs_sample)):.1f}%)")
    lines.append(f"  Missed (no match even ±1ms): {missed:,}")
    lines.append(f"  Join method: exact (time_ms, {char_col}), fallback ±1ms")

    def stats_line(name, vals):
        if not vals:
            return f"  {name}: no data"
        import statistics
        vals_sorted = sorted(vals)
        p99_idx = int(len(vals_sorted) * 0.99)
        return (f"  {name}: max={max(vals):.4f}, mean={statistics.mean(vals):.4f}, "
                f"p99={vals_sorted[p99_idx]:.4f}, median={statistics.median(vals):.4f}")

    lines.append(f"\n  Error statistics (over {joined:,} joined rows):")
    lines.append(stats_line("Position (max axis)", errors_pos))
    lines.append(stats_line("Yaw", errors_yaw))
    lines.append(stats_line("Pitch", errors_pitch))
    lines.append(stats_line("Velocity (max axis)", errors_vel))

    lines.append("")
    return "\n".join(lines)


# ─── 6. Raw blob TypeName comparison ─────────────────────────────────────────

def compare_raw_blobs(cs_events_path: Path, vk_parquet_path: Path) -> str:
    """Find {BitCount, Data, TypeName} blobs in C# and check vrfkit coverage."""
    lines = ["## 6. Raw blob TypeName comparison\n"]

    # Scan C# events for fields with TypeName/BitCount/Data structure
    typename_counts: Counter = Counter()
    blob_groups: defaultdict = defaultdict(set)  # TypeName -> set of group_paths

    for obj in iter_ndjson(cs_events_path):
        if obj.get("type") != "export_group_received":
            continue
        group_path = obj.get("export_group_path", "")
        payload = obj.get("payload", {})
        if not isinstance(payload, dict):
            continue
        for key, val in payload.items():
            if isinstance(val, dict) and "TypeName" in val and "BitCount" in val:
                tname = val["TypeName"]
                typename_counts[tname] += 1
                blob_groups[tname].add(group_path)

    lines.append(f"Distinct TypeName values in C# blobs: {len(typename_counts)}")
    lines.append(f"Total blob instances: {sum(typename_counts.values()):,}")
    lines.append(f"\n{'TypeName':<50} {'Count':>8}  Groups (sample)")
    lines.append("-" * 90)
    for tname, count in typename_counts.most_common():
        groups_sample = sorted(blob_groups[tname])[:3]
        groups_str = "; ".join(g.split("/")[-1] for g in groups_sample)
        lines.append(f"{tname:<50} {count:>8}  {groups_str}")

    # Check vrfkit parquet for raw_bits column
    if vk_parquet_path.exists():
        schema = pq.read_schema(vk_parquet_path)
        col_names = [f.name for f in schema]
        raw_cols = [c for c in col_names if "raw" in c.lower() or "blob" in c.lower() or "bits" in c.lower()]
        if raw_cols:
            lines.append(f"\nvrfkit parquet raw-related columns: {raw_cols}")
        else:
            lines.append(f"\nvrfkit parquet has NO raw_bits/blob columns.")
            lines.append(f"  Available columns: {col_names}")
            lines.append("  → Raw blobs are likely stored as the field value itself (binary type)")
    else:
        lines.append("\nfields.parquet not found.")

    lines.append("")
    return "\n".join(lines)


# ─── Main ─────────────────────────────────────────────────────────────────────

def main():
    if len(sys.argv) != 3:
        print(f"Usage: python {sys.argv[0]} <csharp_dir> <vrfkit_dir>")
        sys.exit(1)

    cs_dir = Path(sys.argv[1])
    vk_dir = Path(sys.argv[2])

    cs_manifest_path = cs_dir / "manifest.json"
    cs_events_path = cs_dir / "events.ndjson"
    cs_movement_path = cs_dir / "movement.ndjson"
    vk_manifest_path = vk_dir / "manifest.json"
    vk_fields_path = vk_dir / "fields.parquet"
    vk_movement_path = vk_dir / "movement.parquet"

    # Load manifests
    with cs_manifest_path.open("r", encoding="utf-8") as f:
        cs_manifest = json.load(f)
    with vk_manifest_path.open("r", encoding="utf-8") as f:
        vk_manifest = json.load(f)

    report_parts = []
    report_parts.append("# vrfkit vs C# Parser Comparison Report\n")
    report_parts.append(f"Replay: {cs_manifest.get('source_file', 'unknown')}")
    report_parts.append(f"Build: {cs_manifest.get('replay_build', 'unknown')}")
    report_parts.append(f"Duration: {cs_manifest.get('duration_ms', 0)} ms\n")

    # 1. Totals
    report_parts.append(compare_totals(cs_manifest, vk_manifest))

    # 2. Export group paths
    report_parts.append(compare_group_paths(cs_manifest, vk_manifest, vk_fields_path))

    # 3. (Group, Field) coverage
    coverage_text, problems = compare_group_field_coverage(
        cs_events_path, vk_fields_path)
    report_parts.append(coverage_text)

    # 4. RPC names
    report_parts.append(compare_rpc_names(cs_events_path, vk_manifest, vk_fields_path))

    # 5. Movement
    report_parts.append(compare_movement(cs_movement_path, vk_movement_path))

    # 6. Raw blobs
    report_parts.append(compare_raw_blobs(cs_events_path, vk_fields_path))

    # Output
    full_report = "\n".join(report_parts)
    print(full_report)

    # Also write to file
    out_path = vk_dir / "comparison_report.txt"
    out_path.write_text(full_report, encoding="utf-8")
    print(f"\n[Report written to {out_path}]")

    # This stays a report -- see `coverage_problems` for why nothing else in it
    # is gated. What it must not do is finish quietly after comparing nothing.
    if problems:
        print(f"\nFAILED: {len(problems)} reason(s) this comparison measured "
              f"nothing", file=sys.stderr)
        for line in problems:
            print(f"    {line}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    import io
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
    raise SystemExit(main())
