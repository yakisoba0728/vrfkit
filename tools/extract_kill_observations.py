"""Extract versioned KillData serialized-update snapshots from one vrfkit export.

The output is an observation stream, not a deduplicated kill ledger. Missing
members remain null and all three serialized clocks remain independent.
"""

from __future__ import annotations
import hashlib, json, math, struct
from pathlib import Path
import pyarrow.compute as pc

from atomic_io import run_json_cli, sha256_file as sha
from wire_bits import InputError, exact_ref, iter_selected, parse_array, read_parquet, text, weapon_theme

SCHEMA_VERSION = 1
GROUP = "/Script/ShooterGame.PlayerMatchStatsComponent"
PARENT = ("KillData", 1493759848)
#: Builds whose KillData identities and values were measured (not the parser's
#: supported builds): any other build fails in declarations() before a row is
#: read, even when its names and checksums agree.
MEASURED_BUILDS = {
    # 12.10, 12.11 and 13.00 export no KillData children.
    "++Ares-Core+release-11.06",
    "++Ares-Core+release-11.07",
    "++Ares-Core+release-11.08",
    "++Ares-Core+release-11.09",
    "++Ares-Core+release-11.10",
    "++Ares-Core+release-11.11",
    "++Ares-Core+release-12.00",
    "++Ares-Core+release-12.01",
    "++Ares-Core+release-12.02",
    "++Ares-Core+release-12.03",
    "++Ares-Core+release-12.04",
    "++Ares-Core+release-12.05",
    "++Ares-Core+release-12.06",
    "++Ares-Core+release-12.07",
    "++Ares-Core+release-12.08",
    "++Ares-Core+release-12.09",
    "++Ares-Core+release-13.01",
    "++Ares-Core+release-13.02",
    "++Ares-Core+release-13.04",
    "++Ares-Core+release-13.05",
    "++Ares-Core+release-13.06",
    "++Ares-Core+release-13.08",
    "++Ares-Core+release-13.09",
}
DECL = {
    3: ("Victim", 3990035472),
    4: ("KillingEquippableClass", 2071131011),
    5: ("WeaponTheme", 1839952321),
    6: ("AssistingPlayers", 1689463717),
    7: ("AssistingPlayers", 1417448159),
    9: ("DamageType", 2992423760),
    10: ("DamageTaken", 2001471495),
    11: ("DamageRegion", 3229265809),
    12: ("GameTimeElapsed", 3684431363),
    13: ("RoundTimestamp", 2328473242),
    14: ("RoundNumber", 843024485),
    15: ("bDidKillTriggerFinisher", 2795684046),
}
MEMBERS = {
    3: "victim_ref",
    4: "killing_equippable_class_ref",
    5: "weapon_theme",
    9: "damage_type_ref",
    10: "damage_taken",
    11: "damage_region",
    12: "game_time_elapsed",
    13: "round_timestamp",
    14: "round_number",
    15: "did_kill_trigger_finisher",
}
#: Raw members a complete (non-partial) element update carries.
REQUIRED = {DECL[handle][0] for handle in MEMBERS}


def declarations(export):
    manifest = json.loads((export / "manifest.json").read_text(encoding="utf-8"))
    out = {}
    if manifest.get("replay_build") not in MEASURED_BUILDS:
        raise InputError(
            f"replay build is outside the measured KillData set: {manifest.get('replay_build')!r}"
        )
    groups = [
        g for g in manifest.get("net_field_export_groups", []) if g.get("path") == GROUP
    ]
    if len(groups) != 1:
        raise InputError("expected exactly one main KillData declaration group")
    main = {}
    for f in groups[0].get("fields", []):
        if f["handle"] in main:
            raise InputError("duplicate main declaration handle")
        main[f["handle"]] = (f.get("name"), f.get("compatible_checksum"))
    if PARENT not in main.values() or any(
        main.get(h) != identity for h, identity in DECL.items()
    ):
        raise InputError("main KillData declarations differ from measured identities")
    out[None] = main
    groups = read_parquet(
        export / "checkpoint_export_groups.parquet",
        columns=["checkpoint_index", "ordinal", "group_path"],
        filters=[("group_path", "=", GROUP)],
        use_threads=False,
    ).to_pylist()
    owners = {}
    for g in groups:
        key = (g["checkpoint_index"], g["ordinal"])
        if key in owners or (g["checkpoint_index"] in out):
            raise InputError("duplicate checkpoint declaration group")
        owners[key] = g["checkpoint_index"]
        out.setdefault(g["checkpoint_index"], {})
    for f in read_parquet(
        export / "checkpoint_export_fields.parquet",
        columns=[
            "checkpoint_index",
            "group_ordinal",
            "handle",
            "rendered_name",
            "compatible_checksum",
        ],
        use_threads=False,
    ).to_pylist():
        key = (f["checkpoint_index"], f["group_ordinal"])
        if key not in owners:
            continue
        scope = owners[key]
        target = out[scope]
        if f["handle"] in target:
            raise InputError("duplicate checkpoint declaration handle")
        target[f["handle"]] = (f["rendered_name"], f["compatible_checksum"])
    return manifest, out


def scoped_refs(export, table):
    if table == "fields":
        actors = set(
            read_parquet(
                export / "actors.parquet", columns=["actor_net_guid"], use_threads=False
            )["actor_net_guid"].to_pylist()
        )
        guids = set(
            read_parquet(
                export / "net_guids.parquet", columns=["net_guid"], use_threads=False
            )["net_guid"].to_pylist()
        )
        return {None: (actors, guids)}
    from collections import defaultdict

    out = defaultdict(lambda: (set(), set()))
    for r in read_parquet(
        export / "checkpoint_actors.parquet",
        columns=["checkpoint_index", "actor_net_guid"],
        use_threads=False,
    ).to_pylist():
        out[r["checkpoint_index"]][0].add(r["actor_net_guid"])
    for r in read_parquet(
        export / "checkpoint_net_guids.parquet",
        columns=["checkpoint_index", "net_guid"],
        use_threads=False,
    ).to_pylist():
        out[r["checkpoint_index"]][1].add(r["net_guid"])
    return out


def selected(path):
    return iter_selected(path, None, lambda batch: pc.and_(
        pc.equal(text(batch, "group_path"), GROUP),
        pc.match_substring_regex(text(batch, "field_name"), r"^KillData(?:\[[0-9]+\](?:\..*)?)?$")))


#: Typed column of each directly decoded member; handle 6 (the assistant
#: container) has none.
COLUMN = {3: "value_i64", 4: "value_i64", 9: "value_i64", 11: "value_i64", 14: "value_i64",
          10: "value_f64", 12: "value_f64", 13: "value_f64", 15: "value_bool", 5: "value_str"}
PRIMITIVE = {"value_i64": int, "value_f64": float, "value_bool": bool, "value_str": str}


def value(row, handle):
    return row[COLUMN[handle]] if handle in COLUMN else None


def decode_direct(handle, raw, width):
    """The value a directly decoded KillData leaf's raw bits encode."""
    if handle in (3, 4, 9):
        return exact_ref(raw, width)
    if handle in (10, 12, 13):
        if width != 32:
            raise InputError(f"KillData handle {handle} float width differs")
        value = float(struct.unpack("<f", raw)[0])
        if not math.isfinite(value):
            raise InputError(f"KillData handle {handle} is non-finite")
        return value
    if handle == 11:
        if width != 8:
            raise InputError("DamageRegion byte width differs")
        return raw[0]
    if handle == 14:
        if width != 32:
            raise InputError("RoundNumber Int32 width differs")
        return int.from_bytes(raw, "little", signed=True)
    if handle == 15:
        if width != 1:
            raise InputError("finisher bool width differs")
        return bool(raw[0] & 1)
    if handle == 5:
        return weapon_theme(raw, width)
    raise InputError(f"KillData handle {handle} has no direct value")


def validate_direct(row, handle, raw, width):
    column = COLUMN.get(handle)
    if {name for name in PRIMITIVE if row[name] is not None} != ({column} if column else set()):
        raise InputError(f"KillData handle {handle} populated wrong typed columns")
    if column is None:
        return
    actual = row[column]
    if type(actual) is not PRIMITIVE[column]:
        raise InputError(f"KillData handle {handle} populated the wrong primitive type")
    expected = decode_direct(handle, raw, width)
    if column == "value_f64":
        matches = struct.pack("<d", actual) == struct.pack("<d", expected)
    else:
        matches = actual == expected
    if not matches:
        raise InputError(f"KillData handle {handle} typed value differs from raw")


def extract_table(export, table, declared, refs):
    observations = []
    pending = []
    counts = {
        "parent_rows": 0,
        "element_updates": 0,
        "partial_updates": 0,
        "assistant_refs": 0,
        "unresolved_actor_refs": 0,
        "unresolved_guid_refs": 0,
    }
    for ordinal, row in selected(export / f"{table}.parquet"):
        if row["field_name"] == "KillData" and row["compatible_checksum"] == PARENT[1]:
            scope = row.get("checkpoint_index")
            scope_decl = declared.get(scope)
            if scope_decl is None or scope_decl.get(row["handle"]) != PARENT:
                raise InputError(
                    f"{table}: parent declaration mismatch in scope {scope}"
                )
            _, _, parts = parse_array(row["raw_bits"], row["bit_count"])
            expected = []
            for index, handle, width, raw in parts:
                if handle not in DECL:
                    raise InputError(f"{table}: unexpected KillData handle {handle}")
                if scope_decl.get(handle) != DECL[handle]:
                    raise InputError(
                        f"{table}: declaration mismatch for observed handle {handle} in scope {scope}"
                    )
                direct = f"KillData[{index}].{DECL[handle][0]}"
                expected.append(("direct", index, handle, width, raw, direct))
                if handle == 6:
                    if scope_decl.get(7) != DECL[7]:
                        raise InputError(
                            f"{table}: nested declaration mismatch in scope {scope}"
                        )
                    _, _, inner = parse_array(raw, width, {7})
                    for inner_index, inner_handle, inner_width, inner_raw in inner:
                        expected.append(
                            (
                                "assistant",
                                index,
                                inner_handle,
                                inner_width,
                                inner_raw,
                                f"{direct}[{inner_index}].AssistingPlayers",
                            )
                        )
            if len(pending) != len(expected):
                raise InputError(
                    f"{table}: child count before parent differs from raw array"
                )
            first = ordinal - len(expected)
            members = {}
            raw_members = {}
            assistants = {}
            for position, (
                (actual_ordinal, actual),
                (kind, index, handle, width, raw, name),
            ) in enumerate(zip(pending, expected)):
                if actual_ordinal != first + position:
                    raise InputError(
                        f"{table}: child is not physically adjacent to parent"
                    )
                if (
                    actual["field_name"],
                    actual["handle"],
                    actual["compatible_checksum"],
                    actual["bit_count"],
                    actual["raw_bits"],
                ) != (name, handle, None, width, raw):
                    raise InputError(
                        f"{table}: emitted child differs from parent raw window"
                    )
                if any(
                    actual.get(k) != row.get(k)
                    for k in (
                        "checkpoint_index",
                        "checkpoint_id",
                        "time_ms",
                        "packet_id",
                        "channel_index",
                        "actor_net_guid",
                        "object_net_guid",
                        "group_path",
                    )
                    if k in row
                ):
                    raise InputError(
                        f"{table}: child scope/coordinates differ from parent"
                    )
                if kind == "assistant":
                    ref = exact_ref(raw, width)
                    if (
                        type(actual["value_i64"]) is not int
                        or actual["value_i64"] != ref
                        or any(
                            actual[x] is not None
                            for x in ("value_f64", "value_bool", "value_str")
                        )
                    ):
                        raise InputError(
                            f"{table}: assistant ObjectNetGuid typing mismatch"
                        )
                    resolution = resolve(ref, refs[scope][0], "actor")
                    assistants.setdefault(index, []).append(
                        {
                            "element_index": int(name.split("[")[2].split("]")[0]),
                            "ref": ref,
                            "actor_resolution": resolution,
                            "raw_bits_hex": raw.hex(),
                            "bit_count": width,
                        }
                    )
                    counts["assistant_refs"] += 1
                    if ref != 0:
                        counts["unresolved_actor_refs"] += ref not in refs[scope][0]
                else:
                    validate_direct(actual, handle, raw, width)
                    if handle == 6:
                        assistants.setdefault(index, [])
                    members.setdefault(index, {})[
                        MEMBERS.get(handle, "assisting_players_raw")
                    ] = value(actual, handle)
                    raw_members.setdefault(index, {})[DECL[handle][0]] = {
                        "handle": handle,
                        "bit_count": width,
                        "raw_bits_hex": raw.hex(),
                    }
            for index in dict.fromkeys(x[1] for x in expected):
                item = {name: None for name in MEMBERS.values()}
                item.update(members.get(index, {}))
                actor_refs = refs[scope][0]
                guid_refs = refs[scope][1]
                victim = item.get("victim_ref")
                item["victim_ref_resolution"] = resolve(victim, actor_refs, "actor")
                if victim not in (None, 0):
                    counts["unresolved_actor_refs"] += victim not in actor_refs
                for key in ("killing_equippable_class_ref", "damage_type_ref"):
                    ref = item.get(key)
                    item[key + "_resolution"] = resolve(ref, guid_refs, "net_guid")
                    if ref not in (None, 0):
                        counts["unresolved_guid_refs"] += ref not in guid_refs
                item["assisting_players"] = (
                    assistants.get(index) if "assisting_players_raw" in item else None
                )
                item["raw_members"] = raw_members.get(index, {})
                complete = set(raw_members.get(index, {})) >= REQUIRED
                counts["partial_updates"] += not complete
                counts["element_updates"] += 1
                observations.append(
                    {
                        "schema_version": SCHEMA_VERSION,
                        "kind": "killdata_serialized_update",
                        "source_table": table,
                        "checkpoint_index": scope,
                        "checkpoint_id": row.get("checkpoint_id"),
                        "physical_parent_row_ordinal": ordinal,
                        "element_index": index,
                        "time_ms": row["time_ms"],
                        "packet_id": row["packet_id"],
                        "channel_index": row["channel_index"],
                        "actor_net_guid": row["actor_net_guid"],
                        "object_net_guid": row["object_net_guid"],
                        "parent_raw_sha256": hashlib.sha256(
                            row["raw_bits"]
                        ).hexdigest(),
                        "members_complete": complete,
                        "members": item,
                    }
                )
            counts["parent_rows"] += 1
            pending = []
        else:
            pending.append((ordinal, row))
    if pending:
        raise InputError(f"{table}: unlinked KillData child rows")
    return observations, counts


def resolve(ref, known, kind):
    if ref is None:
        return "missing"
    if ref == 0:
        return "null"
    return f"resolved_{kind}" if ref in known else f"unresolved_{kind}"


def extract(export):
    inputs = (
        "fields",
        "checkpoint_fields",
        "actors",
        "net_guids",
        "checkpoint_actors",
        "checkpoint_net_guids",
        "checkpoint_export_groups",
        "checkpoint_export_fields",
    )
    input_before = {f"{t}.parquet": sha(export / f"{t}.parquet") for t in inputs}
    manifest_before = sha(export / "manifest.json")
    manifest, declared = declarations(export)
    tables = {}
    counts = {}
    for table in ("fields", "checkpoint_fields"):
        tables[table], counts[table] = extract_table(
            export, table, declared, scoped_refs(export, table)
        )
    input_after = {f"{t}.parquet": sha(export / f"{t}.parquet") for t in inputs}
    manifest_after = sha(export / "manifest.json")
    if input_after != input_before or manifest_after != manifest_before:
        raise InputError("source export changed while it was being read")
    return {
        "schema_version": SCHEMA_VERSION,
        "kind": "vrfkit_killdata_observation_export",
        "provenance": {
            "export_id": export.name,
            "replay_build": manifest["replay_build"],
            "manifest_sha256": manifest_before,
            "input_sha256": input_before,
            "extractor_sha256": sha(Path(__file__)),
            "wire_bits_sha256": sha(Path(__file__).with_name("wire_bits.py")),
        },
        "counts": counts,
        "observations": tables["fields"] + tables["checkpoint_fields"],
    }


def main(argv=None):
    return run_json_cli(__doc__, extract, lambda d, out: [
        f"wrote {out}: {len(d['observations'])} serialized updates"],
        argv, sources=[Path(__file__)], ensure_ascii=True, separators=(",", ":"), allow_nan=False)


if __name__ == "__main__":
    raise SystemExit(main())
