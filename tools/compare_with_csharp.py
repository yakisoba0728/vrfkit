"""Compare vrfkit (Rust) parser output against the C# reference parser's output.

It is a report, not a gate: vrfkit exports more than the C# parser, so
"vrfkit only" is expected, and C#-only pairs are listed under INVESTIGATE, not
gated. Only a comparison that measured nothing fails (`coverage_problems`).
NDJSON is streamed.

Usage:
    python tools/compare_with_csharp.py CSHARP_DIR VRFKIT_DIR

    CSHARP_DIR   the C# export (manifest.json, events.ndjson, movement.ndjson)
    VRFKIT_DIR   vrfkit's export of the same replay (manifest.json,
                 fields.parquet, movement.parquet)
"""

from __future__ import annotations

import itertools
import json
import statistics
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Iterator

import pyarrow as pa
import pyarrow.dataset as ds
import pyarrow.parquet as pq

sys.path.insert(0, str(Path(__file__).resolve().parent))
from atomic_io import atomic_write_text  # noqa: E402
from wire_bits import read_parquet  # noqa: E402
from to_valplay_bundle import (  # noqa: E402
    CLASS_NET_CACHE_SUFFIX,
    UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME,
)

#: C# movement rows whose values are compared.
MOVEMENT_SAMPLE = 50000


def iter_ndjson(path: Path, kind: str | None = None) -> Iterator[dict]:
    """Stream an NDJSON file; with `kind`, only records of that `type`, and a
    line not containing the word is skipped unparsed."""
    needle = kind.encode() if kind else b""
    with path.open("rb") as f:
        for line in f:
            if needle in line and line.strip():
                obj = json.loads(line)
                if kind is None or obj.get("type") == kind:
                    yield obj


def listing(title: str, items: list, limit: int | None = None) -> list[str]:
    """`### title (count):`, the first `limit` items, and how many were cut."""
    lines = [f"\n### {title} ({len(items):,}):"] + [f"  {i}" for i in items[:limit]]
    if limit is not None and len(items) > limit:
        lines.append(f"  ... and {len(items) - limit} more")
    return lines


def compare_totals(cs_manifest: dict, vk_manifest: dict) -> str:
    """Compare packet/bunch/actor/export-group counts from both manifests."""
    lines = ["## 1. Total-count comparison\n"]
    cs_stats = cs_manifest.get("stats", {})
    cs_counts = cs_manifest.get("counts", {})
    vk_stats = vk_manifest.get("stats", {})
    vk_counts = vk_manifest.get("counts", {})

    lines.append(f"{'Metric':<20} {'C#':>12} {'vrfkit':>12} {'Match':>6}")
    lines.append("-" * 55)

    def row(label, cs_val, vk_val):
        # A key absent from a manifest is "?", never a matched 0.
        cs_disp, vk_disp = (f"{'?':>12}" if v is None else f"{v:>12,}"
                            for v in (cs_val, vk_val))
        match = "?" if None in (cs_val, vk_val) else "✓" if cs_val == vk_val else "✗"
        lines.append(f"{label:<20} {cs_disp} {vk_disp} {match:>6}")

    row("Packets",       cs_stats.get("packet_count"),        vk_stats.get("packet_count"))
    # A packet can contain several bunches. The reference emits both counters;
    # packets_with_bunches is neither the preferred value nor a valid fallback.
    row("Bunches",       cs_stats.get("bunch_count"),
                         vk_stats.get("bunch_count", vk_counts.get("bunch_count")))
    row("Actor opens",   cs_counts.get("actor_spawned"),      vk_counts.get("actor_opens"))
    row("Actor closes",  cs_counts.get("actor_closed"),       vk_counts.get("actor_closes"))
    row("Export groups", len(cs_manifest.get("net_field_export_groups", [])),
                         len(vk_manifest.get("net_field_export_groups", [])))
    # vrfkit keeps its movement row count under "quality", not "counts".
    row("Movement rows", cs_counts.get("movement"),
        vk_manifest.get("quality", {}).get("movement_rows"))
    row("RPCs (total)",  cs_counts.get("rpc_received"),       vk_counts.get("rpcs"))

    lines.append("")
    return "\n".join(lines)


def compare_group_paths(cs_manifest: dict, vk_manifest: dict,
                        vk_parquet_path: Path) -> str:
    """Compare the set of export group paths between both manifests."""
    lines = ["## 2. Export group path set comparison\n"]

    cs_paths = {g["path"] for g in cs_manifest.get("net_field_export_groups", [])}
    vk_paths = {g["path"] for g in vk_manifest.get("net_field_export_groups", [])}

    if vk_parquet_path.exists():
        with pq.ParquetFile(vk_parquet_path, pre_buffer=False) as source:
            tbl = source.read(columns=["group_path"])
        distinct = len(set(tbl.column("group_path").to_pylist()))
        lines.append(f"vrfkit parquet distinct group_path: {distinct}")
    else:
        lines.append("(fields.parquet not found — using manifest only)")

    cs_only, vk_only = sorted(cs_paths - vk_paths), sorted(vk_paths - cs_paths)
    lines += [f"\nC# manifest groups: {len(cs_paths)}",
              f"vrfkit manifest groups: {len(vk_paths)}",
              f"Both: {len(cs_paths & vk_paths)}",
              f"C# only: {len(cs_only)}",
              f"vrfkit only: {len(vk_only)}"]
    for title, paths in (("C# only", cs_only), ("vrfkit only", vk_only)):
        if paths:
            lines += listing(title, paths, 50)

    lines.append("")
    return "\n".join(lines)


def coverage_problems(cs_pairs: set, vk_pairs: set) -> list[str]:
    """Why this comparison compared nothing, if it compared nothing.

    `cs_only` is also empty when the C# side yielded no pairs at all, so an
    empty difference alone is no result. C#-only pairs are not gated: that
    would keep a tool measuring known-incomplete coverage red forever.
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
    def pairs(s):
        return [f"({gp}, {fn})" for gp, fn in sorted(s)]

    both, cs_only, vk_only = cs_pairs & vk_pairs, cs_pairs - vk_pairs, vk_pairs - cs_pairs
    lines = [f"\n  Both (intersection): {len(both):,}",
             f"  C# only:             {len(cs_only):,}",
             f"  vrfkit only:         {len(vk_only):,}"]
    lines += listing("Both, sample", pairs(both), 30)
    if cs_only:
        lines += listing("C# only, INVESTIGATE", pairs(cs_only))
    elif cs_pairs:
        lines.append("\n### C# only: NONE -- vrfkit covers everything C# has!")
    else:
        lines.append("\n### C# only: NOT MEASURED -- the C# side produced no "
                     "pairs, so this says nothing about coverage.")
    return lines + listing("vrfkit only, sample", pairs(vk_only), 30)


def compare_group_field_coverage(cs_events_path: Path, vk_parquet_path: Path):
    """Compare (group_path, field_name) pairs between C# events and vrfkit
    parquet: `(report text, problems)`."""
    lines = ["## 3. (Group, Field) coverage comparison\n",
             "Scanning C# events.ndjson for export_group_received..."]

    cs_pairs: set[tuple[str, str]] = set()
    count = cs_unnamed = 0
    for obj in iter_ndjson(cs_events_path, "export_group_received"):
        count += 1
        group_path = obj.get("export_group_path", "")
        payload = obj.get("payload", {})
        if isinstance(payload, dict):
            for key in payload:
                if group_path and key:
                    cs_pairs.add((group_path, key))
                else:
                    cs_unnamed += 1

    lines.append(f"  Scanned {count:,} export_group_received records")
    lines.append(f"  Distinct (group, field) pairs from C#: {len(cs_pairs):,}")
    lines.append(f"  C# payload fields without a group/name: {cs_unnamed:,} (excluded)")

    if not vk_parquet_path.exists():
        lines.append("  fields.parquet not found — cannot compare.")
        return "\n".join(lines), [f"fields.parquet not found at {vk_parquet_path}"]

    with pq.ParquetFile(vk_parquet_path, pre_buffer=False) as source:
        tbl = source.read(columns=["group_path", "field_name"])
    vk_pairs: set[tuple[str, str]] = set()
    vk_unnamed = 0
    for gp, fn in zip(tbl.column("group_path").to_pylist(),
                      tbl.column("field_name").to_pylist()):
        # Neither a preserved whole-block payload nor an unnamed row is field
        # coverage vrfkit has.
        if fn == UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME:
            continue
        if not gp or not fn:
            vk_unnamed += 1
            continue
        vk_pairs.add((gp, fn))

    lines.append(f"  Distinct (group, field) pairs from vrfkit: {len(vk_pairs):,}")
    lines.append(f"  vrfkit rows without a group/name: {vk_unnamed:,} (excluded)")

    lines += coverage_lines(cs_pairs, vk_pairs)
    lines.append("")
    return "\n".join(lines), coverage_problems(cs_pairs, vk_pairs)


def vrfkit_rpc_names(vk_parquet_path: Path) -> Counter:
    """vrfkit's RPC function names in fields.parquet, with their row counts.

    vrfkit writes no RPC-name column: its RPC rows are those in a ClassNetCache
    group (the rows to_valplay_bundle.py builds rpc_received from), and the
    function is the field_name before the first '.' ("Func.Param", "Func._hN",
    or a zero-parameter RPC's bare "Func"). The group check keeps array leaves
    such as "Rounds[3].Score" out. A ClassNetCache custom-delta property
    (`ActiveGameplayEffects` on 02d4d478) counts among the functions, as the
    bundle publishes it.
    """
    with pq.ParquetFile(vk_parquet_path, pre_buffer=False) as source:
        tbl = source.read(columns=["group_path", "field_name"])
    names: Counter = Counter()
    for gp, fn in zip(tbl.column("group_path").to_pylist(),
                      tbl.column("field_name").to_pylist()):
        if (gp and CLASS_NET_CACHE_SUFFIX in gp and fn
                and fn != UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME):
            names[fn.split(".", 1)[0]] += 1
    return names


def rpc_name_lines(cs_names: set, vk_names: set) -> list[str]:
    """The C#-only / vrfkit-only RPC names, the way `coverage_lines` splits
    pairs: a C# side with no names at all is not measured, never NONE."""
    cs_only, vk_only = sorted(cs_names - vk_names), sorted(vk_names - cs_names)
    lines = [f"  Both: {len(cs_names & vk_names)}",
             f"  C# only: {len(cs_only)}",
             f"  vrfkit only: {len(vk_only)}"]
    if not cs_names:
        lines.append("\n### C# only: NOT MEASURED -- the C# side has no rpc_received "
                     "records, so this says nothing about vrfkit.")
    elif cs_only:
        lines += listing("C# only, INVESTIGATE", cs_only)
    else:
        lines.append("\n### C# only: NONE -- vrfkit emits every RPC name the C# side has.")
    return lines + listing("vrfkit only, sample", vk_only, 30)


def compare_rpc_names(cs_events_path: Path, vk_parquet_path: Path) -> str:
    """Compare RPC function names between the two parsers."""
    lines = ["## 4. RPC name comparison\n"]
    cs_rpc_names = Counter(obj.get("function_name", "<unknown>")
                           for obj in iter_ndjson(cs_events_path, "rpc_received"))
    lines.append(f"  C# RPC distinct names: {len(cs_rpc_names)}")
    lines.append(f"  C# RPC total records: {sum(cs_rpc_names.values()):,}")

    if vk_parquet_path.exists():
        vk_rpc_names = vrfkit_rpc_names(vk_parquet_path)
        lines.append(f"  vrfkit RPC distinct names: {len(vk_rpc_names)} (field_name "
                     f"prefixes of {sum(vk_rpc_names.values()):,} ClassNetCache rows)")
        lines += rpc_name_lines(set(cs_rpc_names), set(vk_rpc_names))
    else:
        lines.append("  fields.parquet not found -- vrfkit's RPC names NOT MEASURED.")

    lines.append("\n  C# RPC function_name breakdown:")
    for name, count in cs_rpc_names.most_common():
        lines.append(f"    {name}: {count:,}")

    lines.append("")
    return "\n".join(lines)


def _xyz(value) -> list:
    return [value.get(axis) for axis in "xyz"] if isinstance(value, dict) else [None]


def compare_movement(cs_movement_path: Path, vk_movement_path: Path) -> str:
    """Compare sampled moves by packet, character and movement timestamp.

    Older exports without identity metadata use display time (+-1 ms). An
    ambiguous identity is reported, never selected by position or velocity.
    """
    lines = ["## 5. Movement value comparison\n"]

    if not cs_movement_path.exists():
        lines.append("C# movement.ndjson not found.")
        return "\n".join(lines)
    if not vk_movement_path.exists():
        lines.append("vrfkit movement.parquet not found.")
        return "\n".join(lines)

    with cs_movement_path.open("r", encoding="utf-8") as f:
        cs_row_count = sum(1 for _ in f)
    with pq.ParquetFile(vk_movement_path, pre_buffer=False) as source:
        vk_row_count = source.metadata.num_rows
        vk_columns = source.schema_arrow.names
    lines += [f"C# movement rows (in file): {cs_row_count:,}",
              f"vrfkit movement rows: {vk_row_count:,}",
              f"Difference: {vk_row_count - cs_row_count:,} (vrfkit - C#)",
              f"vrfkit movement columns: {vk_columns}",
              f"\nSampling first {MOVEMENT_SAMPLE:,} C# rows for value comparison..."]

    cs_sample: list[tuple[int, int, dict]] = []
    for obj in itertools.islice(iter_ndjson(cs_movement_path), MOVEMENT_SAMPLE):
        time_ms, char_guid = obj.get("time_ms"), obj.get("shooter_character_net_guid")
        if time_ms is not None and char_guid is not None:
            cs_sample.append((time_ms, char_guid, obj))
    if not cs_sample:
        lines.append("No C# samples found.")
        return "\n".join(lines)

    min_time = min(row[0] for row in cs_sample)
    max_time = max(row[0] for row in cs_sample)
    # movement.parquet's schema is fixed and non-nullable (vrf-export's movement_schema).
    columns = ["time_ms", "character_net_guid", "pos_x", "pos_y", "pos_z", "yaw", "pitch",
               "vel_x", "vel_y", "vel_z"]
    identity_columns = {"packet_id", "timestamp"}.issubset(vk_columns)

    def has_identity(row):
        return (identity_columns and row.get("packet_id") is not None
                and row.get("timestamp") is not None)

    candidate_filter = ((ds.field("time_ms") >= max(0, min_time - 1))
                        & (ds.field("time_ms") <= max_time + 1))
    if identity_columns:
        columns += ["packet_id", "timestamp"]
        strong_packet_ids = {row["packet_id"] for _t, _g, row in cs_sample if has_identity(row)}
        if strong_packet_ids:
            # Display time is not part of the strong identity. Retain every
            # candidate packet even when its display timestamp disagrees.
            candidate_filter |= ds.field("packet_id").isin(
                pa.array(sorted(strong_packet_ids), type=pa.uint64()))
    vk_tbl = read_parquet(
        vk_movement_path, columns=columns, filters=candidate_filter)
    vk = {name: vk_tbl.column(name).to_pylist() for name in columns}
    vk_rows: defaultdict[tuple[int, int], list[int]] = defaultdict(list)
    vk_identity: defaultdict[tuple, list[int]] = defaultdict(list)
    for i, key in enumerate(zip(vk["time_ms"], vk["character_net_guid"])):
        vk_rows[key].append(i)
        if identity_columns:
            vk_identity[(vk["packet_id"][i], vk["character_net_guid"][i], vk["timestamp"][i])].append(i)

    def available(indices, *, legacy=False):
        return [index for index in indices if index not in used
                and (not legacy or index not in reserved)]

    # A value the C# row lacks is not compared, never read as 0.
    errors: dict[str, list[float]] = {"Position (max axis)": [], "Yaw": [], "Pitch": [],
                                      "Velocity (max axis)": []}
    joined = missed = ambiguous = unmatched_identity = 0
    identity_rows = sum(has_identity(row) for _t, _g, row in cs_sample)
    used: set[int] = set()
    reserved: set[int] = set()
    # Reserve strong identities before any legacy row can consume a candidate.
    for t, g, cs_row in sorted(cs_sample, key=lambda sample: not has_identity(sample[2])):
        strong = has_identity(cs_row)
        if strong:
            candidates = available(vk_identity.get((cs_row["packet_id"], g, cs_row["timestamp"]), ()))
        else:
            candidates = available(vk_rows.get((t, g), ()), legacy=True)
            if not candidates:
                candidates = available([*vk_rows.get((t - 1, g), ()), *vk_rows.get((t + 1, g), ())],
                                       legacy=True)
        if not candidates:
            missed += 1
            unmatched_identity += strong
            continue
        if len(candidates) != 1:
            if strong:
                # Duplicated identities remain ambiguous to other strong rows,
                # but cannot be repurposed as a weaker display-time match.
                reserved.update(candidates)
            ambiguous += 1
            continue
        vk_idx = candidates[0]
        used.add(vk_idx)
        joined += 1
        for label, cs_vals, columns in (
            ("Position (max axis)", _xyz(cs_row.get("position")), ("pos_x", "pos_y", "pos_z")),
            ("Yaw", [cs_row.get("yaw")], ("yaw",)),
            ("Pitch", [cs_row.get("pitch")], ("pitch",)),
            ("Velocity (max axis)", _xyz(cs_row.get("velocity")), ("vel_x", "vel_y", "vel_z")),
        ):
            if None not in cs_vals:
                errors[label].append(max(abs(c - vk[col][vk_idx])
                                         for c, col in zip(cs_vals, columns)))

    lines.append(f"  Joined: {joined:,} / {len(cs_sample):,} ({100*joined/len(cs_sample):.1f}%)")
    lines.append(f"  Missed (no match under selected join): {missed:,}")
    lines.append(f"  Ambiguous (multiple candidate rows): {ambiguous:,}")
    lines.append(f"  Exact identity rows: {identity_rows:,}")
    lines.append(f"  Unmatched exact identities: {unmatched_identity:,}")
    lines.append(f"  Legacy rows (identity metadata absent): {len(cs_sample) - identity_rows:,}")
    lines.append("  Join method: exact (packet_id, character_net_guid, movement timestamp)")
    lines.append("  Legacy only: exact (time_ms, character_net_guid), then ±1ms; ambiguous candidates are not compared")

    lines.append(f"\n  Error statistics (over {joined:,} joined rows):")
    for label, vals in errors.items():
        if not vals:
            lines.append(f"  {label}: no data")
            continue
        vals.sort()
        lines.append(f"  {label} ({len(vals):,} rows): max={vals[-1]:.4f}, "
                     f"mean={statistics.mean(vals):.4f}, p99={vals[int(len(vals) * 0.99)]:.4f}, "
                     f"median={statistics.median(vals):.4f}")

    lines.append("")
    lines.append("  Comparison only: numeric tolerances are not applied; nonzero differences require interpretation.")
    return "\n".join(lines)


def compare_raw_blobs(cs_events_path: Path) -> str:
    """Count the C# side's {BitCount, Data, TypeName} blobs by TypeName; the
    vrfkit side is not measured here."""
    lines = ["## 6. C# raw blob TypeNames\n"]

    typename_counts: Counter = Counter()
    blob_groups: defaultdict = defaultdict(set)  # TypeName -> set of group_paths

    for obj in iter_ndjson(cs_events_path, "export_group_received"):
        payload = obj.get("payload", {})
        if not isinstance(payload, dict):
            continue
        for val in payload.values():
            if isinstance(val, dict) and "TypeName" in val and "BitCount" in val:
                typename_counts[val["TypeName"]] += 1
                blob_groups[val["TypeName"]].add(obj.get("export_group_path", ""))

    lines.append(f"Distinct TypeName values in C# blobs: {len(typename_counts)}")
    lines.append(f"Total blob instances: {sum(typename_counts.values()):,}")
    lines.append(f"\n{'TypeName':<50} {'Count':>8}  Groups (sample)")
    lines.append("-" * 90)
    for tname, count in typename_counts.most_common():
        groups_str = "; ".join(g.split("/")[-1] for g in sorted(blob_groups[tname])[:3])
        lines.append(f"{tname:<50} {count:>8}  {groups_str}")

    lines.append("")
    return "\n".join(lines)


def main():
    if len(sys.argv) != 3:
        sys.exit(f"Usage: python {sys.argv[0]} <csharp_dir> <vrfkit_dir>")
    cs_dir, vk_dir = Path(sys.argv[1]), Path(sys.argv[2])
    cs_events = cs_dir / "events.ndjson"
    vk_fields = vk_dir / "fields.parquet"
    cs_manifest = json.loads((cs_dir / "manifest.json").read_text(encoding="utf-8"))
    vk_manifest = json.loads((vk_dir / "manifest.json").read_text(encoding="utf-8"))

    report_parts = [
        "# vrfkit vs C# Parser Comparison Report\n",
        f"Replay: {cs_manifest.get('source_file', 'unknown')}",
        f"Build: {cs_manifest.get('replay_build', 'unknown')}",
        f"Duration: {cs_manifest.get('duration_ms', 'unknown')} ms\n",
        compare_totals(cs_manifest, vk_manifest),
        compare_group_paths(cs_manifest, vk_manifest, vk_fields),
    ]
    coverage_text, problems = compare_group_field_coverage(cs_events, vk_fields)
    report_parts += [
        coverage_text,
        compare_rpc_names(cs_events, vk_fields),
        compare_movement(cs_dir / "movement.ndjson", vk_dir / "movement.parquet"),
        compare_raw_blobs(cs_events),
    ]

    full_report = "\n".join(report_parts)
    print(full_report)
    out_path = vk_dir / "comparison_report.txt"
    atomic_write_text(out_path, full_report)
    print(f"\n[Report written to {out_path}]")

    # A report, but not one that finishes quietly after comparing nothing.
    if problems:
        print(f"\nFAILED: {len(problems)} reason(s) this comparison measured "
              f"nothing", file=sys.stderr)
        for line in problems:
            print(f"    {line}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    raise SystemExit(main())
