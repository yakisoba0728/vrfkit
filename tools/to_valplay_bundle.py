"""Convert vrfkit Parquet export into a valplay-compatible NDJSON bundle.

WHY: valplay's compute_metrics.py consumes events.ndjson + movement.ndjson +
manifest.json. This adapter proves the vrfkit Rust parser can replace the C#
parser by converting vrfkit's flat Parquet rows back into the nested JSON
events that compute_metrics.py already understands. No metrics code is
reimplemented -- only the serialization format is bridged.

The Parquet schema stores one row per decoded field. Replicated properties are
grouped by (packet_id, actor_net_guid, group_path) into export_group_received
events. RPCs (group_path contains '_ClassNetCache') are grouped by
(packet_id, actor_net_guid, group_path, handle) into rpc_received events.

KNOWN GAPS (documented, not invented):
- valorant_shot_received: vrfkit does not decode the EffectContainer blob from
  ClientPlayOneShotEffectAtLocation RPCs into typed shot data. Affects:
  weapons, shot_rays, spray_control, posture, and shot denominators in
  weapon_stats.
- actor_spawned location: inferred from first field appearance; location data
  is available only for actors that appear in movement.parquet or have a known
  spawn position from other fields.
- LifeChangeEvents: raw blob only; HP values not decoded into typed output.
- TeamEconomy sub-fields (LoadoutValue/AverageLoadoutValue): raw blob only.
- RoundResults sub-fields: raw blob only.

Usage:
    python tools/to_valplay_bundle.py <vrfkit_export_dir> [-o <output_dir>]
"""

from __future__ import annotations

import argparse
import base64
import json
import math
import os
import re
import sys
import time
from collections import defaultdict
from pathlib import Path

try:
    import pyarrow.parquet as pq
    import pyarrow.compute as pc
except ImportError:
    sys.exit("pyarrow is required: pip install pyarrow")


# ---------------------------------------------------------------------------
# RegionalDamage enum mapping: vrfkit stores as int, valplay expects string
# ---------------------------------------------------------------------------
REGIONAL_DAMAGE_MAP = {
    0: "regional_damage__headshot",
    1: "regional_damage__normal",
    2: "regional_damage__legshot",
    3: "regional_damage__invalid",
}


# ---------------------------------------------------------------------------
# Field name normalization for replicated properties
# ---------------------------------------------------------------------------
def _normalize_prop_field_name(field_name: str, is_bool: bool) -> str:
    """Normalize a replicated property field name to match C# parser output.

    WHY: The C# parser strips the 'b' prefix from UE4 boolean property names
    (e.g. 'bUltimateActive' -> 'UltimateActive', 'bLoadoutFinalized' ->
    'LoadoutFinalized'). vrfkit preserves the raw UE4 names. We normalize
    to match compute_metrics.py's expectations.
    """
    if is_bool and field_name.startswith('b') and len(field_name) > 1 and field_name[1].isupper():
        return field_name[1:]
    return field_name


# ---------------------------------------------------------------------------
# Nested path parser: "Rounds[0].Reports[1].Interactions[2].DamageDealt"
# -> [("Rounds", 0), ("Reports", 1), ("Interactions", 2), ("DamageDealt", None)]
# ---------------------------------------------------------------------------
_PATH_RE = re.compile(r'([A-Za-z_][A-Za-z0-9_]*)(?:\[(\d+)\])?')


def _parse_field_path(path: str):
    """Parse a dot-separated field path with optional array indices."""
    parts = []
    for seg in path.split('.'):
        m = _PATH_RE.fullmatch(seg)
        if m:
            name = m.group(1)
            idx = int(m.group(2)) if m.group(2) is not None else None
            parts.append((name, idx))
        else:
            # Unresolved handle names like "_h27" or bare numbers like "248"
            parts.append((seg, None))
    return parts


def _set_nested(root: dict, parts: list, value):
    """Set a value deep in a nested dict/list structure using parsed path parts.

    WHY: vrfkit stores each field as a flat row with full path like
    'Rounds[0].Reports[0].Interactions[0].DamageDealt = 30'. We need to
    reconstruct the nested JSON object that compute_metrics.py expects.
    Array elements are auto-extended with None/empty dicts as needed.

    Array elements get an 'Index' field set to their subscript position,
    matching the C# parser's behavior (compute_metrics uses inter.get("Index")
    for deduplication).
    """
    obj = root
    for i, (name, idx) in enumerate(parts):
        is_last = (i == len(parts) - 1)
        # Ensure current level has the key as a dict or list
        if idx is not None:
            # This level is an array
            if name not in obj:
                obj[name] = []
            arr = obj[name]
            if not isinstance(arr, list):
                # Conflict: was set as a non-list value, override
                obj[name] = []
                arr = obj[name]
            # Extend array to have at least idx+1 elements
            while len(arr) <= idx:
                arr.append({} if not is_last else None)
            if is_last:
                arr[idx] = value
            else:
                if arr[idx] is None or not isinstance(arr[idx], dict):
                    arr[idx] = {}
                # Set the Index field on array elements to match C# output
                if "Index" not in arr[idx]:
                    arr[idx]["Index"] = idx
                obj = arr[idx]
        else:
            if is_last:
                obj[name] = value
            else:
                if name not in obj:
                    obj[name] = {}
                next_obj = obj[name]
                if not isinstance(next_obj, dict):
                    # Conflict: overwrite non-dict with dict
                    obj[name] = {}
                    next_obj = obj[name]
                obj = next_obj


def _get_value(row_i64, row_f64, row_bool, row_str, row_raw, row_bits):
    """Extract the typed value from a fields.parquet row.

    Exactly one of the typed columns is non-null when decoded, otherwise the
    raw_bits blob is the value. Returns (value, is_raw).
    """
    if row_i64 is not None:
        return row_i64, False
    if row_f64 is not None:
        # Truncate float to reasonable precision to match C# output
        return row_f64, False
    if row_bool is not None:
        return row_bool, False
    if row_str is not None:
        return row_str, False
    if row_raw is not None:
        # Return as {BitCount, Data, TypeName} blob format matching C# output
        bit_count = row_bits
        data_b64 = base64.b64encode(row_raw).decode('ascii')
        return {"BitCount": bit_count, "Data": data_b64}, True
    return None, False


# ---------------------------------------------------------------------------
# RPC name extraction from field_name: "MulticastNotifyDamage_Point.DamageTaken"
# -> rpc_name = "MulticastNotifyDamage_Point", param_name = "DamageTaken"
# ---------------------------------------------------------------------------
def _split_rpc_field(field_name: str):
    """Split an RPC field_name into (rpc_name, param_name).

    For zero-param RPCs, the field_name IS the RPC name with no dot.
    """
    if field_name is None:
        return None, None
    dot = field_name.find('.')
    if dot == -1:
        return field_name, None
    return field_name[:dot], field_name[dot+1:]


# ---------------------------------------------------------------------------
# Actor class inference: map group_path to replication_class_path
# ---------------------------------------------------------------------------
def _group_path_to_class(gp: str) -> str:
    """Convert a group_path to an approximate replication_class_path.

    WHY: vrfkit does not export explicit actor_spawned events. We infer the
    class from the first group_path seen for each actor. The group_path for
    replicated properties IS the class path (e.g.
    '/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C').
    For ClassNetCache RPCs, strip the _ClassNetCache suffix to get the class.
    """
    if '_ClassNetCache' in gp:
        # e.g. '/Script/ShooterGame.DamageableComponent_ClassNetCache'
        # -> '/Script/ShooterGame.DamageableComponent'
        return gp.replace('_ClassNetCache', '')
    return gp


def _group_path_to_archetype(gp: str) -> str:
    """Derive a Default__X_PC_C archetype path from the group_path."""
    # e.g. '/Game/Characters/Wushu/Wushu_PC.Wushu_PC_C'
    # archetype = 'Default__Wushu_PC_C'
    if '.' in gp:
        leaf = gp.rsplit('.', 1)[-1]
        return f"Default__{leaf}"
    return f"Default__{gp.rsplit('/', 1)[-1]}"


# ---------------------------------------------------------------------------
# Main conversion
# ---------------------------------------------------------------------------
def convert(export_dir: Path, output_dir: Path, *, verbose: bool = False):
    """Read vrfkit Parquet export and write valplay-compatible bundle."""
    fields_path = export_dir / "fields.parquet"
    movement_path = export_dir / "movement.parquet"
    manifest_path = export_dir / "manifest.json"

    if not fields_path.exists():
        sys.exit(f"fields.parquet not found in {export_dir}")

    output_dir.mkdir(parents=True, exist_ok=True)

    # ---- Load manifest ----
    manifest = {}
    if manifest_path.exists():
        manifest = json.loads(manifest_path.read_text(encoding='utf-8'))

    # Write minimal manifest for compute_metrics
    out_manifest = {
        "replay_version": manifest.get("replay_version", "unknown"),
        "duration_ms": manifest.get("duration_ms", 0),
        "replay_build": manifest.get("replay_build", ""),
        "replay_changelist": manifest.get("replay_changelist", 0),
        "source_file": manifest.get("source_file", ""),
        # Mark as vrfkit-converted
        "converter": "vrfkit/tools/to_valplay_bundle.py",
    }
    (output_dir / "manifest.json").write_text(
        json.dumps(out_manifest, indent=2), encoding='utf-8'
    )

    # ---- Load fields.parquet ----
    t0 = time.time()
    if verbose:
        print("Loading fields.parquet...")
    table = pq.read_table(fields_path)
    n_rows = len(table)
    if verbose:
        print(f"  {n_rows:,} rows loaded in {time.time()-t0:.1f}s")

    # Extract columns as Python lists for fast iteration
    # (pyarrow iteration row-by-row is slow; batch extract is much faster)
    t0 = time.time()
    if verbose:
        print("Extracting columns...")

    col_time = table.column('time_ms').to_pylist()
    col_pid = table.column('packet_id').to_pylist()
    col_actor = table.column('actor_net_guid').to_pylist()
    col_gp = table.column('group_path').cast('string').to_pylist()
    col_handle = table.column('handle').to_pylist()
    col_fn = table.column('field_name').cast('string').to_pylist()
    col_bits = table.column('bit_count').to_pylist()
    col_raw = table.column('raw_bits').to_pylist()
    col_i64 = table.column('value_i64').to_pylist()
    col_f64 = table.column('value_f64').to_pylist()
    col_bool = table.column('value_bool').to_pylist()
    col_str = table.column('value_str').to_pylist()

    if verbose:
        print(f"  Columns extracted in {time.time()-t0:.1f}s")

    # ---- Classify rows: RPC vs replicated property ----
    # RPCs: group_path contains '_ClassNetCache'
    # Properties: everything else

    # We need to group and emit events in packet_id order (time order).
    # Strategy: build a list of events keyed by packet_id, then sort and write.

    t0 = time.time()
    if verbose:
        print("Grouping rows into events...")

    # Track actor first/last appearance for actor_spawned/actor_closed
    actor_first = {}  # actor_net_guid -> (time_ms, packet_id, group_path)
    actor_last = {}   # actor_net_guid -> (time_ms, packet_id)

    # Group key -> list of row indices
    # For properties: (packet_id, actor_net_guid, group_path) -> [row_indices]
    # For RPCs: (packet_id, actor_net_guid, group_path, handle) -> [row_indices]
    prop_groups = defaultdict(list)
    rpc_groups = defaultdict(list)

    for i in range(n_rows):
        actor = col_actor[i]
        gp = col_gp[i]
        pid = col_pid[i]
        ms = col_time[i]

        # Track actor lifecycle
        if actor not in actor_first:
            actor_first[actor] = (ms, pid, gp)
        actor_last[actor] = (ms, pid)

        is_rpc = '_ClassNetCache' in gp
        if is_rpc:
            handle = col_handle[i]
            rpc_groups[(pid, actor, gp, handle)].append(i)
        else:
            prop_groups[(pid, actor, gp)].append(i)

    if verbose:
        print(f"  {len(prop_groups):,} property events, {len(rpc_groups):,} RPC invocations")
        print(f"  Grouped in {time.time()-t0:.1f}s")

    # ---- Build events list ----
    t0 = time.time()
    if verbose:
        print("Building event records...")

    events = []  # (packet_id, time_ms, event_dict)

    # 1. actor_spawned events (inferred from first appearance)
    for actor, (ms, pid, gp) in actor_first.items():
        class_path = _group_path_to_class(gp)
        archetype = _group_path_to_archetype(gp)
        event = {
            "type": "actor_spawned",
            "time_ms": ms,
            "actor_net_guid": actor,
            "replication_class_path": class_path,
            "archetype_path": archetype,
            "location": {"x": 0, "y": 0, "z": 0},
        }
        events.append((pid, ms, event))

    # 2. actor_closed events (inferred from last appearance)
    for actor, (ms, pid) in actor_last.items():
        event = {
            "type": "actor_closed",
            "time_ms": ms,
            "actor_net_guid": actor,
        }
        # Use a packet_id just after the last to ensure ordering
        events.append((pid + 1, ms, event))

    # 3. export_group_received events (replicated properties)
    for (pid, actor, gp), row_indices in prop_groups.items():
        ms = col_time[row_indices[0]]

        # Build nested payload from field paths.
        # WHY two passes: vrfkit emits BOTH a bare array-container blob (e.g.
        # field_name="Rounds", raw_bits=the whole serialized array) AND the
        # individually-decoded sub-fields (e.g. "Rounds[0].Reports[0]...").
        # If we naively set the bare blob first, it clobbers the list that
        # _set_nested needs. Solution: first pass collects names that have
        # indexed versions (contain '['), second pass skips bare blobs for
        # those names.
        indexed_names = set()
        for ri in row_indices:
            fn = col_fn[ri]
            if fn and '[' in fn:
                # Top-level array name = everything before first '['
                indexed_names.add(fn[:fn.index('[')])

        payload = {}
        for ri in row_indices:
            fn = col_fn[ri]
            if fn is None:
                continue
            value, is_raw = _get_value(
                col_i64[ri], col_f64[ri], col_bool[ri], col_str[ri],
                col_raw[ri], col_bits[ri]
            )
            if value is None and not is_raw:
                continue

            # Normalize boolean field names (strip 'b' prefix)
            is_bool = col_bool[ri] is not None
            fn = _normalize_prop_field_name(fn, is_bool)

            # Parse the field path and set in nested structure
            parts = _parse_field_path(fn)
            if len(parts) == 1 and parts[0][1] is None:
                # Simple top-level field. Skip if it's a raw blob that has
                # indexed sub-fields (the sub-fields carry the decoded data).
                bare_name = parts[0][0]
                if is_raw and bare_name in indexed_names:
                    continue
                payload[bare_name] = value
            else:
                _set_nested(payload, parts, value)

        # Emit even if payload is empty (some events are just existence signals)
        event = {
            "type": "export_group_received",
            "time_ms": ms,
            "export_group_path": gp,
            "actor_net_guid": actor,
            "object_net_guid": actor,  # best approximation
            "payload": payload,
        }
        events.append((pid, ms, event))

    # 4. rpc_received events
    for (pid, actor, gp, handle), row_indices in rpc_groups.items():
        ms = col_time[row_indices[0]]

        # Determine RPC name from first field_name
        rpc_name = None
        payload = {}
        for ri in row_indices:
            fn = col_fn[ri]
            if fn is None:
                continue
            name, param = _split_rpc_field(fn)
            if rpc_name is None:
                rpc_name = name
            if param is None:
                # Zero-param RPC or just the function name row
                continue
            value, is_raw = _get_value(
                col_i64[ri], col_f64[ri], col_bool[ri], col_str[ri],
                col_raw[ri], col_bits[ri]
            )
            if value is None and not is_raw:
                continue
            # Map parameter names to match C# parser output
            param_out = _normalize_rpc_param(rpc_name, param, value, is_raw)
            if param_out is not None:
                for k, v in param_out.items():
                    payload[k] = v

        if rpc_name is None:
            continue

        # Normalize the RPC function name to match C# parser output
        function_name = _normalize_rpc_name(rpc_name)

        event = {
            "type": "rpc_received",
            "time_ms": ms,
            "function_name": function_name,
            "actor_net_guid": actor,
            "payload": payload if payload else None,
        }
        events.append((pid, ms, event))

    if verbose:
        print(f"  {len(events):,} total events built in {time.time()-t0:.1f}s")

    # ---- Sort by (packet_id, time_ms) and write ----
    t0 = time.time()
    if verbose:
        print("Sorting and writing events.ndjson...")
    events.sort(key=lambda x: (x[0], x[1]))

    events_written = 0
    with open(output_dir / "events.ndjson", 'w', encoding='utf-8') as f:
        for _, _, evt in events:
            f.write(json.dumps(evt, separators=(',', ':'), ensure_ascii=True))
            f.write('\n')
            events_written += 1

    if verbose:
        print(f"  {events_written:,} events written in {time.time()-t0:.1f}s")

    # ---- Convert movement.parquet ----
    movement_written = 0
    if movement_path.exists():
        t0 = time.time()
        if verbose:
            print("Converting movement.parquet...")
        mv_table = pq.read_table(movement_path)
        n_mv = len(mv_table)

        mv_time = mv_table.column('time_ms').to_pylist()
        mv_char = mv_table.column('character_net_guid').to_pylist()
        mv_px = mv_table.column('pos_x').to_pylist()
        mv_py = mv_table.column('pos_y').to_pylist()
        mv_pz = mv_table.column('pos_z').to_pylist()
        mv_yaw = mv_table.column('yaw').to_pylist()
        mv_pitch = mv_table.column('pitch').to_pylist()
        mv_vx = mv_table.column('vel_x').to_pylist()
        mv_vy = mv_table.column('vel_y').to_pylist()
        mv_vz = mv_table.column('vel_z').to_pylist()

        with open(output_dir / "movement.ndjson", 'w', encoding='utf-8') as f:
            for i in range(n_mv):
                rec = {
                    "time_ms": mv_time[i],
                    "shooter_character_net_guid": mv_char[i],
                    "position": {"x": mv_px[i], "y": mv_py[i], "z": mv_pz[i]},
                    "velocity": {"x": mv_vx[i], "y": mv_vy[i], "z": mv_vz[i]},
                    "yaw": mv_yaw[i],
                    "pitch": mv_pitch[i],
                }
                f.write(json.dumps(rec, separators=(',', ':'), ensure_ascii=True))
                f.write('\n')
                movement_written += 1

        if verbose:
            print(f"  {movement_written:,} movement rows written in {time.time()-t0:.1f}s")
    else:
        if verbose:
            print("  movement.parquet not found, skipping movement.ndjson")

    # ---- Summary ----
    print(f"\nConversion complete: {output_dir}")
    print(f"  events.ndjson:   {events_written:,} lines")
    print(f"  movement.ndjson: {movement_written:,} lines")
    print(f"  manifest.json:   written")

    return {
        "events_written": events_written,
        "movement_written": movement_written,
    }


# ---------------------------------------------------------------------------
# RPC name normalization
# ---------------------------------------------------------------------------
def _normalize_rpc_name(name: str) -> str:
    """Map vrfkit's RPC field_name prefix to the C# parser's function_name.

    WHY: vrfkit uses the exact ClassNetCache field name as the RPC name prefix
    in field_name (e.g. 'MulticastNotifyDamage_Point'). The C# parser emits
    the same names, but some zero-param RPCs may differ in casing or prefix.
    """
    # Most are identical. Known mappings:
    return name


# ---------------------------------------------------------------------------
# RPC parameter normalization
# ---------------------------------------------------------------------------
def _normalize_rpc_param(rpc_name: str, param: str, value, is_raw: bool) -> dict | None:
    """Normalize an RPC parameter name and value to match C# parser output.

    WHY: vrfkit uses prefixed 'b' for booleans (e.g. 'bDamageKilledTarget')
    while C# emits 'DamageKilledTarget'. Also, RegionalDamage is stored as
    int enum in vrfkit but as string in C# output.
    """
    result = {}

    if rpc_name in ("MulticastNotifyDamage_Point", "MulticastNotifyDamage_Base"):
        # Parameter name mapping for damage RPCs
        param_map = {
            "bDamageKilledTarget": "DamageKilledTarget",
            "bAliveAfterDamage": "AliveAfterDamage",
            "bIsWallPenetration": "IsWallPenetration",
            "bEquippableUsedZoomed": "EquippableUsedZoomed",
            "bEquippableUsedInFocusMode": "EquippableUsedInFocusMode",
        }
        out_name = param_map.get(param, param)

        # RegionalDamage: int -> string
        if param == "RegionalDamage" and not is_raw:
            value = REGIONAL_DAMAGE_MAP.get(value, f"regional_damage__unknown_{value}")

        # EquippableUsed: raw blob -> {NetGuid: int} structure
        if param == "EquippableUsed" and is_raw:
            # The EquippableUsed is a net GUID packed in 16 bits
            # Try to decode as a simple int
            if isinstance(value, dict) and "Data" in value:
                raw_bytes = base64.b64decode(value["Data"])
                if len(raw_bytes) >= 2:
                    # Little-endian uint16 net GUID
                    net_guid = int.from_bytes(raw_bytes[:2], 'little')
                    result[out_name] = {
                        "NetGuid": net_guid,
                        "Name": None,
                        "ClassPath": None,
                        "Category": "unknown",
                    }
                    return result
            result[out_name] = value
            return result

        # LifeChangeEvents: keep as blob
        if param == "LifeChangeEvents" and is_raw:
            # Pass through as {BitCount, Data, TypeName} matching C# format
            if isinstance(value, dict):
                value["TypeName"] = "LifeChangeEvents"
            result[out_name] = value
            return result

        # DamagedBone: raw -> string "0" or actual bone name
        if param == "DamagedBone" and is_raw:
            if isinstance(value, dict) and "Data" in value:
                raw_bytes = base64.b64decode(value["Data"])
                # Try to interpret as a null-terminated string or simple int
                try:
                    # It's typically a short string or "0"
                    decoded = raw_bytes.rstrip(b'\x00').decode('ascii', errors='replace')
                    if decoded:
                        result[out_name] = decoded
                    else:
                        result[out_name] = "0"
                except Exception:
                    result[out_name] = "0"
                return result
            result[out_name] = value
            return result

        # Vector fields stored as raw blobs
        if param in ("DamageOrigin", "DamageDirection", "DamageImpactLocation",
                     "DamageImpactNormal", "DamageImpactBoneRelativeLocation"):
            if is_raw:
                # These are packed vectors, skip for now (not consumed by metrics)
                result[out_name] = value
                return result

        # DeathMontageEffectOverride* blobs: pass through
        if param.startswith("DeathMontageEffect") and is_raw:
            if isinstance(value, dict):
                value["TypeName"] = param
            result[out_name] = value
            return result

        result[out_name] = value
        return result

    elif rpc_name == "MulticastNotifyKilledEnemy":
        # Parameters: KillerCharacter, KilledCharacter, MultikillLevel
        result[param] = value
        return result

    elif rpc_name in ("MulticastEndRound", "ClientRoundStart"):
        # NewRoundNumber
        result[param] = value
        return result

    elif rpc_name == "MulticastSetPhase":
        result[param] = value
        return result

    elif rpc_name == "MulticastReceivePlayerResurrectEvent":
        result[param] = value
        return result

    else:
        # Generic: pass through all params
        if is_raw and isinstance(value, dict):
            # Keep blob format
            pass
        result[param] = value
        return result


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------
def main():
    parser = argparse.ArgumentParser(
        description="Convert vrfkit Parquet export to valplay NDJSON bundle"
    )
    parser.add_argument("export_dir", type=Path,
                        help="vrfkit export directory (contains fields.parquet)")
    parser.add_argument("-o", "--output", type=Path, default=None,
                        help="Output bundle directory (default: out/valplay_bundle/<stem>)")
    parser.add_argument("-v", "--verbose", action="store_true",
                        help="Print progress messages")
    args = parser.parse_args()

    export_dir = args.export_dir.resolve()
    if args.output:
        output_dir = args.output.resolve()
    else:
        # Derive stem from source_file in manifest or directory name
        manifest_path = export_dir / "manifest.json"
        if manifest_path.exists():
            m = json.loads(manifest_path.read_text(encoding='utf-8'))
            source = m.get("source_file", "")
            stem = Path(source).stem if source else export_dir.name
        else:
            stem = export_dir.name
        output_dir = Path(__file__).resolve().parent.parent / "out" / "valplay_bundle" / stem

    convert(export_dir, output_dir, verbose=args.verbose)


if __name__ == "__main__":
    main()
