"""Independent, bounded wire check for Clay_PC_C.FocusProjectiles.

No route is enabled by this script. Numeric object identities and raw replay
bytes are not included in its aggregate receipt.
"""
from collections import Counter
import hashlib
import json
from pathlib import Path
import sys

import pyarrow.compute as pc
import pyarrow.parquet as pq

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "tools"))
from validate_type_evidence import Bits

GROUP = "/Game/Characters/Clay/Clay_PC.Clay_PC_C"
PARENT = (45, "FocusProjectiles", 957945936)
MEMBER = (46, "FocusProjectiles", 831657485)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def decode(raw, width):
    """Strict independent delta-array framing and one packed GUID per element."""
    bits = Bits(raw, width)
    capacity = bits.int_packed()
    if capacity > 4096:
        raise ValueError("capacity limit")
    values = []
    while True:
        index = bits.int_packed()
        if index == 0:
            break
        if index > capacity:
            raise ValueError("element outside capacity")
        handle = bits.int_packed()
        if handle != MEMBER[0] + 1:
            raise ValueError("unproven member handle")
        length = bits.int_packed()
        if length not in (8, 16, 24, 32, 40):
            raise ValueError("packed GUID byte width")
        value_raw = bits.bits(length).to_bytes(length // 8, "little")
        value_bits = Bits(value_raw, length)
        value = value_bits.int_packed()
        if value_bits.remaining():
            raise ValueError("GUID suffix")
        if bits.int_packed() != 0:
            raise ValueError("unexpected second member")
        values.append((index - 1, value, length))
    trailer = False
    if not values and bits.remaining() == 8:
        if bits.int_packed() != 0:
            raise ValueError("nonzero empty trailer")
        trailer = True
    if bits.remaining():
        raise ValueError("array suffix")
    return capacity, values, trailer


def rows(path):
    columns = ["group_path", "field_name", "handle", "compatible_checksum", "bit_count", "raw_bits"]
    if path.name == "checkpoint_fields.parquet":
        columns += ["checkpoint_index", "checkpoint_id"]
    with pq.ParquetFile(path, pre_buffer=False) as file:
        for batch in file.iter_batches(batch_size=4096, columns=columns, use_threads=False):
            mask = pc.and_(pc.equal(batch["group_path"], GROUP),
                           pc.equal(batch["field_name"], PARENT[1]))
            yield from batch.filter(mask).to_pylist()


def declarations(export):
    manifest = json.loads((export / "manifest.json").read_text())
    main = [row for row in manifest["net_field_export_groups"] if row["path"] == GROUP]
    assert len(main) == 1
    expected = {PARENT, MEMBER}
    got = {(row["handle"], row["name"], row["compatible_checksum"]) for row in main[0]["fields"]}
    assert expected <= got
    groups = pq.read_table(export / "checkpoint_export_groups.parquet").to_pylist()
    wanted = {(row["checkpoint_index"], row["path_name_index"]): row["checkpoint_id"]
              for row in groups if row["group_path"] == GROUP}
    fields = pq.read_table(export / "checkpoint_export_fields.parquet").to_pylist()
    observed = {key: set() for key in wanted}
    for row in fields:
        key = (row["checkpoint_index"], row["path_name_index"])
        if key in wanted:
            assert row["checkpoint_id"] == wanted[key]
            observed[key].add((row["handle"], row["rendered_name"], row["compatible_checksum"]))
    assert observed and all(expected <= seen for seen in observed.values())
    return manifest["replay_build"], {key[0]: value for key, value in wanted.items()}


def inspect(export):
    build, checkpoints = declarations(export)
    assert build == "++Ares-Core+release-12.05"
    guid_rows = pq.read_table(export / "net_guids.parquet").to_pylist()
    guids = {row["net_guid"]: row["path"] for row in guid_rows}
    actor_rows = pq.read_table(export / "actors.parquet",
        columns=["actor_net_guid", "event", "archetype_path", "class_path"]).to_pylist()
    actor_opens = {row["actor_net_guid"]: row for row in actor_rows if row["event"] == "open"}
    reports = []
    for name in ("fields", "checkpoint_fields"):
        counts, widths, capacities, members, references = Counter(), Counter(), Counter(), Counter(), Counter()
        archetypes = Counter()
        digest = hashlib.sha256()
        for row in rows(export / (name + ".parquet")):
            assert (row["handle"], row["field_name"], row["compatible_checksum"]) == PARENT
            if name == "checkpoint_fields":
                assert checkpoints[row["checkpoint_index"]] == row["checkpoint_id"]
            raw = row["raw_bits"]
            assert raw is not None
            capacity, values, trailer = decode(raw, row["bit_count"])
            digest.update(row["bit_count"].to_bytes(4, "little") + raw)
            counts["parents"] += 1
            counts["populated" if values else "empty"] += 1
            counts["empty_zero_trailer"] += trailer
            counts["member_values"] += len(values)
            widths[row["bit_count"]] += 1
            capacities[capacity] += 1
            for _index, guid, width in values:
                members[width] += 1
                if guid == 0:
                    references["null"] += 1
                elif guid in guids:
                    references["resolved"] += 1
                    path = guids[guid]
                    # Counts only: no object instance paths or identifiers leave the local report.
                    references["projectile_path" if "Projectile" in path else "other_path"] += 1
                else:
                    references["unresolved"] += 1
                if guid in actor_opens:
                    references["actor_open_match"] += 1
                    actor = actor_opens[guid]
                    archetypes[actor["archetype_path"] or actor["class_path"] or "path-unresolved"] += 1
        reports.append({"table": name, "source_table_sha256": sha(export / (name + ".parquet")),
            "parent_raw_sequence_sha256": digest.hexdigest(), "counts": dict(counts),
            "widths": sorted(widths.items()), "capacities": sorted(capacities.items()),
            "member_widths": sorted(members.items()), "reference_resolution": dict(references),
            "matched_actor_archetypes": dict(archetypes)})
    return {"build": build, "group": GROUP, "parent": PARENT, "member": MEMBER,
            "scope": "Public 12.05 historical Phase B export; independently re-read in Phase C",
            "source_manifest_sha256": sha(export / "manifest.json"),
            "checkpoint_declaration_sets": len(checkpoints), "reports": reports}


def controls():
    # Independent synthetic bytes: capacity 1, index 0, child handle46,
    # 8-bit IntPacked GUID7, child/root terminators. No player data.
    good = bytes((2, 2, 94, 16, 14, 0, 0))
    assert decode(good, 56) == (1, [(0, 7, 8)], False)
    assert decode(bytes((2, 0, 0)), 24) == (1, [], True)
    assert decode(bytes((0, 0)), 16) == (0, [], False)
    cases = {
        "nonzero_empty_trailer": bytes((2, 0, 2)),
        "truncated_empty_trailer": bytes((2, 0, 1)),
        "truncated_root_terminator": good[:-1],
        "wrong_member_handle": bytes((2, 2, 92, 16, 14, 0, 0)),
        "member_width_overrun": bytes((2, 2, 94, 80, 14, 0, 0)),
        "index_out_of_capacity": bytes((2, 4, 94, 16, 14, 0, 0)),
        "populated_array_suffix": good + b"\0",
        "missing_nested_terminator": good[:-2] + b"\2\0",
        "runaway_guid": bytes((2, 2, 94, 80, 255, 255, 255, 255, 255, 0, 0)),
        "overflow_guid": bytes((2, 2, 94, 80, 255, 255, 255, 255, 32, 0, 0)),
    }
    rejected = []
    for name, raw in cases.items():
        try:
            decode(raw, len(raw) * 8)
        except ValueError:
            rejected.append(name)
        else:
            raise AssertionError("accepted malformed control: " + name)
    return {"positive_shapes": 3, "malformed_rejected": rejected}


if __name__ == "__main__":
    report = inspect(Path(sys.argv[1]))
    report["controls"] = controls()
    report["performance_claim"] = None
    print(json.dumps(report, indent=2))
