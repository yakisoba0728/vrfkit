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

Shot data: ReplayPlayContinuousEffectAtLocation RPCs carry FloatValues,
ObjectValues, VectorValues blobs. These are decoded in Python using the
gameplay tag table from the manifest, and emitted as valorant_shot_received
events matching the C# parser's output format.

Weapon identity: the shot blob does not name the gun. It carries a FiringState
subobject GUID, whose *outer* is the equippable actor. net_guids.parquet
supplies that containment chain and actors.parquet supplies the class path,
which equippable_table.py maps to a display name. This mirrors the C# parser's
second resolution tier (ValorantShotEventEnricher.ResolveFromFiringState); its
first tier, an equippable GUID inside the effect blob, is never populated in
practice (0 of 2,647 shots in the 02d4d478 reference).

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

import struct as _struct

sys.path.insert(0, str(Path(__file__).parent))
from equippable_table import EQUIPPABLE_BY_PATH  # noqa: E402


# Unreal's own cap in FNetGUIDCache traversal; the C# resolver uses the same
# value. Real chains observed in 02d4d478 are one hop, but a bounded loop is
# what keeps a malformed self-referential chain from hanging the adapter.
MAX_OUTER_DEPTH = 16

# Substrings that mark a firing state as the weapon's secondary fire cycle.
# Copied from ValorantShotFireModeResolver.AlternateMarkers; matched
# case-insensitively against every object path on the FiringState outer chain.
#
# The distinction matters because spray_control drops alternate-fire shots
# outright: ADS and burst cycles have a different recoil pattern, so mixing
# them into a spray would compare shots that were never part of one.
ALTERNATE_FIRE_MARKERS = (
    "altfire",
    "zoomedfire",
    "zoomedfiring",
    "firingstateburst",
    "burstfiringstate",
    "burstmode",
)


def _has_alternate_marker(value) -> bool:
    if not value:
        return False
    lowered = str(value).lower()
    return any(marker in lowered for marker in ALTERNATE_FIRE_MARKERS)


def _resolve_fire_mode(firing_state_guid, source_id, guid_outer, guid_path):
    """Classify a shot as primary / alternate fire.

    Returns ``(fire_mode, evidence)``, mirroring ValorantShotFireModeResolver.

    The signal is the *name* of the firing-state subobject: a gun replicates
    "FiringState" for its primary cycle and "ZoomedFiringState",
    "FiringStateBurst" etc. for its secondary one. Neither the ammo counters
    nor burst_shot_number carry this -- burst_shot_number just indexes shots
    within any spray, so treating a non-zero value as "alternate" (as this
    adapter previously did) misclassified 1,462 of 2,475 shots on 02d4d478.

    "unknown" when no path resolves: those are effects with no firing state at
    all, not shots whose mode we failed to determine.
    """
    if _has_alternate_marker(source_id):
        return "alternate", f"source:{source_id}"

    paths = []
    current = firing_state_guid
    for _ in range(MAX_OUTER_DEPTH):
        if not current:
            break
        path = guid_path.get(current)
        if path and path.strip():
            paths.append(path)
        nxt = guid_outer.get(current)
        if nxt is None or nxt == current:
            break
        current = nxt

    if not paths:
        return "unknown", None

    evidence = "firing-state:" + " -> ".join(paths)
    if any(_has_alternate_marker(p) for p in paths):
        return "alternate", evidence
    return "primary", evidence


def _resolve_equippable(net_guid, guid_outer, guid_path, guid_class):
    """Walk a GUID's outer chain to the equippable actor that contains it.

    Returns ``(owner_net_guid, name, category, class_path)`` or ``None``.

    Two lookups per hop, because the two tables cover different populations:
    ``guid_class`` comes from actors.parquet (channel opens, carrying the spawn
    class path) while ``guid_path`` comes from net_guids.parquet (every GUID the
    replay registered, including subobjects that never opened a channel).
    A weapon appears in the first; its FiringState only in the second.
    """
    current = net_guid
    for _ in range(MAX_OUTER_DEPTH):
        if not current:
            return None
        for table in (guid_class, guid_path):
            path = table.get(current)
            if path:
                hit = EQUIPPABLE_BY_PATH.get(path)
                if hit:
                    name, category, canonical = hit
                    return current, name, category, canonical
        nxt = guid_outer.get(current)
        if nxt is None or nxt == current:
            return None
        current = nxt
    return None


def _load_net_guids(export_dir):
    """Read net_guids.parquet into (guid -> outer, guid -> path) dicts.

    Returns empty dicts when the file is absent, so bundles produced by an
    older vrfkit still convert -- weapon identity is simply left unresolved
    rather than the run failing.
    """
    path = export_dir / "net_guids.parquet"
    if not path.exists():
        return {}, {}
    table = pq.read_table(path)
    guids = table.column("net_guid").to_pylist()
    paths = table.column("path").cast("string").to_pylist()
    outers = table.column("outer_net_guid").to_pylist()
    guid_outer = {g: o for g, o in zip(guids, outers) if o is not None}
    guid_path = {g: p for g, p in zip(guids, paths) if p}
    return guid_outer, guid_path


# ---------------------------------------------------------------------------
# Effect blob decoder (Python port of the Rust vrf-decode/src/effect.rs logic)
# ---------------------------------------------------------------------------
class _BitReader:
    """Minimal bit-level reader for UE4 IntPacked, f32, f64."""

    def __init__(self, data: bytes, bit_len: int):
        self._data = data
        self._bit_len = bit_len
        self._pos = 0

    def at_end(self) -> bool:
        return self._pos >= self._bit_len

    def bits_remaining(self) -> int:
        return max(0, self._bit_len - self._pos)

    def read_bit(self) -> int:
        if self._pos >= self._bit_len:
            raise EOFError
        byte_idx = self._pos >> 3
        bit_idx = self._pos & 7
        self._pos += 1
        return (self._data[byte_idx] >> bit_idx) & 1

    def read_bits(self, n: int) -> int:
        val = 0
        for i in range(n):
            val |= self.read_bit() << i
        return val

    def read_int_packed(self) -> int:
        """Read a UE4 IntPacked value (7-bit variable-length, LSB first)."""
        value = 0
        shift = 0
        while True:
            if self._pos + 8 > self._bit_len:
                raise EOFError
            byte_val = self.read_bits(8)
            has_more = byte_val & 1
            value |= (byte_val >> 1) << shift
            shift += 7
            if not has_more:
                break
            if shift > 35:
                raise ValueError("IntPacked overflow")
        return value

    def read_f32(self) -> float:
        bits = self.read_bits(32)
        return _struct.unpack('<f', _struct.pack('<I', bits))[0]

    def read_f64(self) -> float:
        lo = self.read_bits(32)
        hi = self.read_bits(32)
        raw = lo | (hi << 32)
        return _struct.unpack('<d', _struct.pack('<Q', raw))[0]

    def skip_bits(self, n: int):
        self._pos += n
        if self._pos > self._bit_len:
            self._pos = self._bit_len


def _decode_effect_floats(data: bytes, bit_count: int):
    """Decode FloatValues blob -> list of (tag_index, value) tuples."""
    r = _BitReader(data, bit_count)
    try:
        count = r.read_int_packed()
    except (EOFError, ValueError):
        return []
    if count > 256 or count == 0:
        return []
    elements = [(None, None)] * count
    while not r.at_end():
        try:
            enc_idx = r.read_int_packed()
        except (EOFError, ValueError):
            break
        if enc_idx == 0:
            if r.bits_remaining() == 8:
                try:
                    r.read_int_packed()
                except (EOFError, ValueError):
                    pass
            break
        idx = enc_idx - 1
        if idx >= count:
            break
        tag = None
        val = None
        while not r.at_end():
            try:
                enc_h = r.read_int_packed()
            except (EOFError, ValueError):
                break
            if enc_h == 0:
                break
            handle = enc_h - 1
            try:
                payload_bits = r.read_int_packed()
            except (EOFError, ValueError):
                break
            if payload_bits > r.bits_remaining():
                break
            start = r._pos
            if handle == 7:
                try:
                    tag = r.read_int_packed()
                except (EOFError, ValueError):
                    pass
            elif handle == 8:
                try:
                    val = r.read_f32()
                except (EOFError, ValueError):
                    pass
            consumed = r._pos - start
            if consumed < payload_bits:
                r.skip_bits(payload_bits - consumed)
        elements[idx] = (tag, val)
    return elements


def _decode_effect_objects(data: bytes, bit_count: int):
    """Decode ObjectValues blob -> list of (tag_index, net_guid) tuples."""
    r = _BitReader(data, bit_count)
    try:
        count = r.read_int_packed()
    except (EOFError, ValueError):
        return []
    if count > 256 or count == 0:
        return []
    elements = [(None, None)] * count
    while not r.at_end():
        try:
            enc_idx = r.read_int_packed()
        except (EOFError, ValueError):
            break
        if enc_idx == 0:
            if r.bits_remaining() == 8:
                try:
                    r.read_int_packed()
                except (EOFError, ValueError):
                    pass
            break
        idx = enc_idx - 1
        if idx >= count:
            break
        tag = None
        val = None
        while not r.at_end():
            try:
                enc_h = r.read_int_packed()
            except (EOFError, ValueError):
                break
            if enc_h == 0:
                break
            handle = enc_h - 1
            try:
                payload_bits = r.read_int_packed()
            except (EOFError, ValueError):
                break
            if payload_bits > r.bits_remaining():
                break
            start = r._pos
            if handle == 15:
                try:
                    tag = r.read_int_packed()
                except (EOFError, ValueError):
                    pass
            elif handle == 16:
                try:
                    val = r.read_int_packed()
                except (EOFError, ValueError):
                    pass
            consumed = r._pos - start
            if consumed < payload_bits:
                r.skip_bits(payload_bits - consumed)
        elements[idx] = (tag, val)
    return elements


def _decode_effect_vectors(data: bytes, bit_count: int):
    """Decode VectorValues blob -> list of (tag_index, (x,y,z)) tuples."""
    r = _BitReader(data, bit_count)
    try:
        count = r.read_int_packed()
    except (EOFError, ValueError):
        return []
    if count > 256 or count == 0:
        return []
    elements = [(None, None)] * count
    while not r.at_end():
        try:
            enc_idx = r.read_int_packed()
        except (EOFError, ValueError):
            break
        if enc_idx == 0:
            if r.bits_remaining() == 8:
                try:
                    r.read_int_packed()
                except (EOFError, ValueError):
                    pass
            break
        idx = enc_idx - 1
        if idx >= count:
            break
        tag = None
        val = None
        while not r.at_end():
            try:
                enc_h = r.read_int_packed()
            except (EOFError, ValueError):
                break
            if enc_h == 0:
                break
            handle = enc_h - 1
            try:
                payload_bits = r.read_int_packed()
            except (EOFError, ValueError):
                break
            if payload_bits > r.bits_remaining():
                break
            start = r._pos
            if handle == 11:
                try:
                    tag = r.read_int_packed()
                except (EOFError, ValueError):
                    pass
            elif handle == 12:
                try:
                    x = r.read_f64()
                    y = r.read_f64()
                    z = r.read_f64()
                    val = (x, y, z)
                except (EOFError, ValueError):
                    pass
            consumed = r._pos - start
            if consumed < payload_bits:
                r.skip_bits(payload_bits - consumed)
        elements[idx] = (tag, val)
    return elements


def _build_tag_table(manifest: dict) -> dict:
    """Build tag_index -> tag_name from the manifest's gameplay tag group."""
    groups = manifest.get("net_field_export_groups", [])
    for g in groups:
        if g.get("path") == "NetworkGameplayTagNodeIndex":
            return {f["handle"]: f["name"] for f in g.get("fields", [])}
    return {}


def _build_shot_event(
    time_ms, packet_id, actor_net_guid, channel_index,
    scalar_params: dict, float_blob, object_blob, vector_blob,
    tag_table: dict, equippable_lookup=None, fire_mode_lookup=None,
) -> dict | None:
    """Build a valorant_shot_received event from decoded RPC params.

    Returns None if this is not a gun-shot event (no FiringState tags).
    """
    # Decode blobs
    floats = {}  # tag_name -> float_value
    objects = {}  # tag_name -> net_guid
    vectors = {}  # tag_name -> (x,y,z)

    if float_blob is not None:
        raw_bytes = float_blob if isinstance(float_blob, bytes) else b''
        bit_count = len(raw_bytes) * 8
        for (tag_idx, val) in _decode_effect_floats(raw_bytes, bit_count):
            if tag_idx is not None and val is not None:
                name = tag_table.get(tag_idx, str(tag_idx))
                floats[name] = val

    if object_blob is not None:
        raw_bytes = object_blob if isinstance(object_blob, bytes) else b''
        bit_count = len(raw_bytes) * 8
        for (tag_idx, val) in _decode_effect_objects(raw_bytes, bit_count):
            if tag_idx is not None and val is not None:
                name = tag_table.get(tag_idx, str(tag_idx))
                objects[name] = val

    if vector_blob is not None:
        raw_bytes = vector_blob if isinstance(vector_blob, bytes) else b''
        bit_count = len(raw_bytes) * 8
        for (tag_idx, val) in _decode_effect_vectors(raw_bytes, bit_count):
            if tag_idx is not None and val is not None:
                name = tag_table.get(tag_idx, str(tag_idx))
                vectors[name] = val

    # Only emit as a shot if FiringState data is present
    if "FiringState.FiringPlayerState" not in objects:
        return None

    # Extract scalar params from the RPC payload
    location = scalar_params.get("Location")
    rotation = scalar_params.get("Rotation")
    # Fallback: Location/Rotation may arrive as unnamed params "248"/"249"
    if location is None:
        raw248 = scalar_params.get("248")
        if isinstance(raw248, dict) and "Data" in raw248:
            raw_bytes = base64.b64decode(raw248["Data"])
            if len(raw_bytes) >= 24:
                x = _struct.unpack_from('<d', raw_bytes, 0)[0]
                y = _struct.unpack_from('<d', raw_bytes, 8)[0]
                z = _struct.unpack_from('<d', raw_bytes, 16)[0]
                # Full precision: the reference emits the raw double
                # (559.962145690918), and rounding here was the only thing
                # keeping shot_rays.sample_rays from matching.
                location = _vec3(x, y, z)
    if rotation is None:
        raw249 = scalar_params.get("249")
        if isinstance(raw249, dict) and "Data" in raw249:
            raw_bytes = base64.b64decode(raw249["Data"])
            bit_count_rot = raw249.get("BitCount", len(raw_bytes) * 8)
            rotation = _decode_rotation_short(raw_bytes, bit_count_rot)
    effect_id = scalar_params.get("EffectID")
    source_id = scalar_params.get("SourceID")
    start_time = scalar_params.get("StartMovementTime")
    is_local = scalar_params.get("bLocalEffect") or scalar_params.get("LocalEffect")
    is_transient = scalar_params.get("bTransient") or scalar_params.get("Transient")
    wait_on = scalar_params.get("WaitOnReplicationActor")
    alliance = scalar_params.get("AllianceFilter")

    # Parse location/rotation from value_str compact format if needed
    loc_obj = _parse_location(location)
    rot_obj = _parse_rotation(rotation)

    # Build attack vectors
    attack_vectors = []
    for i in range(1, 16):
        key = f"FiringState.AttackVector.{i}"
        if key in vectors:
            x, y, z = vectors[key]
            attack_vectors.append({"x": x, "y": y, "z": z})

    burst = floats.get("FiringState.BurstShotNumber")
    yaw_switch = floats.get("FiringState.YawSwitch")

    # Ammo
    ammo = floats.get("FiringState.AmmoRemaining")
    if ammo is not None:
        ammo = int(ammo)

    num_proj = floats.get("FiringState.NumProjectiles")
    if num_proj is not None:
        num_proj = int(num_proj)

    random_seed = floats.get("FiringState.RandomSeed")
    tracer_opt = floats.get("FiringState.TracerOption")
    if tracer_opt is not None:
        tracer_opt = int(tracer_opt) if tracer_opt == int(tracer_opt) else tracer_opt

    firing_player = objects.get("FiringState.FiringPlayerState")
    firing_state = objects.get("FiringState.FiringState")

    # Weapon identity: FiringState is a subobject of the gun, so its outer is
    # the equippable actor. Null when the chain does not reach a known
    # equippable -- never guessed.
    equippable = None
    if equippable_lookup is not None and firing_state:
        hit = equippable_lookup(firing_state)
        if hit is not None:
            owner_guid, name, category, class_path = hit
            equippable = {
                "net_guid": owner_guid,
                "name": name,
                "category": category,
                "class_path": class_path,
            }

    # Fire mode comes from the same chain: the firing-state subobject's own name.
    if fire_mode_lookup is not None:
        fire_mode, fire_mode_evidence = fire_mode_lookup(firing_state, source_id)
    else:
        fire_mode, fire_mode_evidence = "unknown", None

    # Alliance filter string
    alliance_str = None
    if alliance is not None:
        if isinstance(alliance, int):
            alliance_map = {0: "alliance_self", 1: "alliance_ally",
                           2: "alliance_enemy", 3: "alliance_any", 4: "alliance_any"}
            alliance_str = alliance_map.get(alliance, f"alliance_{alliance}")
        else:
            alliance_str = str(alliance)

    shot = {
        "effect_id": effect_id,
        "start_movement_time": start_time,
        "source_id": source_id,
        "is_local_effect": bool(is_local) if is_local is not None else False,
        "is_transient": bool(is_transient) if is_transient is not None else True,
        "wait_on_replication_actor": wait_on or 0,
        "alliance_filter": alliance_str or "alliance_any",
        "location": loc_obj,
        "rotation": rot_obj,
        "ammo_remaining": ammo,
        "num_projectiles": num_proj or 1,
        "random_seed": random_seed,
        "tracer_option": tracer_opt,
        "burst_shot_number": floats.get("FiringState.BurstShotNumber"),
        "yaw_switch": yaw_switch,
        "firing_player_state": firing_player,
        "firing_state": firing_state,
        "attack_vectors": attack_vectors if attack_vectors else [],
        # The C# parser's tier-1 source; never populated in any observed replay.
        "effect_equippable": None,
        "equippable": equippable,
        "fire_mode": fire_mode,
        "fire_mode_evidence": fire_mode_evidence,
    }

    return {
        "type": "valorant_shot_received",
        "time_ms": time_ms,
        "packet_id": packet_id,
        "actor_net_guid": actor_net_guid,
        "object_net_guid": actor_net_guid,
        "channel": channel_index,
        "shot": shot,
    }


def _parse_location(val) -> dict:
    """Parse a Location value into {x, y, z}, preserving full precision.

    This used to round to 2 decimals, which was the last thing keeping
    shot_rays.sample_rays from matching the reference -- it emits the raw
    double (559.962145690918).

    Callers expect a dict, so an unparseable value still yields a zero vector
    rather than None. That is a fabricated value and the one place in this
    adapter that has one; it is retained because the shot filter upstream
    already guarantees a location is present, so it should be unreachable.
    """
    if isinstance(val, dict):
        return val
    parsed = _parse_exact_vector(val) if val is not None else None
    return parsed if parsed is not None else {"x": 0, "y": 0, "z": 0}


def _vec3(x, y, z) -> dict:
    """Build an {x, y, z} dict, emitting integral components as ints.

    Matches how the C# reference serializes a double: System.Text.Json writes
    0.0 as `0`, so keeping Python floats here would put `0.0` where the
    reference has `0`.
    """
    return {
        axis: (int(n) if float(n).is_integer() else n)
        for axis, n in zip(("x", "y", "z"), (x, y, z))
    }


def _parse_exact_vector(val):
    """Parse a "(x,y,z)" vector without losing precision.

    Distinct from _parse_location, which rounds to 2 decimals and substitutes
    a zero vector when it cannot parse. Neither is acceptable here: the damage
    direction is a unit vector the reference emits at full float precision
    (0.055482650227362894), and a zero vector would be a silent wrong value
    rather than a visible absence.

    Integral components come back as ints so the output matches the C#
    reference exactly -- it emits {"x": 0, "y": 1, "z": 0}, not 0.0/1.0/0.0.

    Returns None when the value is not a parseable vector.
    """
    if isinstance(val, dict):
        return val
    if not isinstance(val, str):
        return None
    parts = val.strip("()").split(",")
    if len(parts) != 3:
        return None
    try:
        nums = [float(p) for p in parts]
    except ValueError:
        return None
    return {
        axis: (int(n) if n.is_integer() else n)
        for axis, n in zip(("x", "y", "z"), nums)
    }


def _parse_rotation(val) -> dict:
    """Parse a Rotation value into {pitch, yaw, roll}."""
    if val is None:
        return {"pitch": 0, "yaw": 0, "roll": 0}
    if isinstance(val, dict):
        return val
    if isinstance(val, str):
        # Compact rotator format: "rot(pitch,yaw,roll)"
        s = val
        if s.startswith("rot(") and s.endswith(")"):
            s = s[4:-1]
        else:
            s = s.strip("()")
        parts = s.split(",")
        if len(parts) == 3:
            try:
                return {"pitch": float(parts[0]),
                        "yaw": float(parts[1]),
                        "roll": float(parts[2])}
            except ValueError:
                pass
    return {"pitch": 0, "yaw": 0, "roll": 0}


def _decode_rotation_short(data: bytes, bit_count: int) -> dict:
    """Decode a UE4 RotationShort from raw bits.

    Wire format: for each of pitch/yaw/roll:
      1 bit: is_non_zero
      if non_zero: 16 bits LE unsigned -> degrees = value * 360 / 65536
    """
    r = _BitReader(data, bit_count)
    result = {"pitch": 0.0, "yaw": 0.0, "roll": 0.0}
    for axis in ("pitch", "yaw", "roll"):
        try:
            is_non_zero = r.read_bit()
            if is_non_zero:
                val = r.read_bits(16)
                degrees = val * 360.0 / 65536.0
                result[axis] = degrees
        except (EOFError, ValueError):
            break
    return result


# ---------------------------------------------------------------------------
# RegionalDamage enum mapping: vrfkit stores as int, valplay expects string
# ---------------------------------------------------------------------------
# Damage RPC parameters that carry an FVector_NetQuantize* payload. The C#
# call sites are DamageParameters.cs:50 and
# MulticastNotifyDamagePointParameters.cs:40-46.
DAMAGE_VECTOR_PARAMS = frozenset({
    "DamageOrigin",
    "DamageImpactLocation",
    "DamageImpactBoneRelativeLocation",
    "DamageDirection",
    "DamageImpactNormal",
})


# EAresRegionalDamage.cs. The ordinals are the C# enum's, not an invention:
#
#   RegionalDamage_Normal         = 0
#   RegionalDamage_Headshot       = 1
#   RegionalDamage_Legshot        = 2
#   RegionalDamage_RegionCount    = 3
#   RegionalDamage_Invalid_Radial = 4
#   RegionalDamage_Invalid        = 5
#   RegionalDamage_CountPlusOne   = 6
#
# This map previously had 0 and 1 swapped and put invalid at 3, so every
# body shot was reported as a headshot and vice versa (Vandal: 109 head /
# 35 body instead of 35 / 109), and the 18 genuine "no hit region" damage
# events at ordinal 5 fell through to unknown_5.
#
# The four strings that appear on the wire are verified against 02d4d478's
# reference bundle. The remaining three are derived with the same
# name-to-string rule (insert "_" before each capital, lowercase) which that
# verification confirms on 4 of 4 cases; they are sentinels and have not been
# observed, so an unexpected ordinal still falls through to a loud
# unknown_{n} rather than being silently absorbed.
REGIONAL_DAMAGE_MAP = {
    0: "regional_damage__normal",           # verified: 446 occurrences
    1: "regional_damage__headshot",         # verified: 83
    2: "regional_damage__legshot",          # verified: 26
    3: "regional_damage__region_count",     # derived, sentinel
    4: "regional_damage__invalid__radial",  # derived, not observed
    5: "regional_damage__invalid",          # verified: 76
    6: "regional_damage__count_plus_one",   # derived, sentinel
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
    # (fallback when actors.parquet is absent).
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

    # Containment chain for weapon identity. Loaded before the actor pass so
    # guid_class can be filled from the same loop that emits actor_spawned.
    guid_outer, guid_path = _load_net_guids(export_dir)
    guid_class = {}  # actor net guid -> spawn class path

    # 1 & 2. actor_spawned and actor_closed events from actors.parquet
    #         (authoritative: carries class/archetype/location from spawn data)
    actors_path = export_dir / "actors.parquet"
    if actors_path.exists():
        actors_table = pq.read_table(actors_path)
        a_time = actors_table.column('time_ms').to_pylist()
        a_pid = actors_table.column('packet_id').to_pylist()
        a_chan = actors_table.column('channel_index').to_pylist()
        a_guid = actors_table.column('actor_net_guid').to_pylist()
        a_event = actors_table.column('event').to_pylist()
        a_class = actors_table.column('class_path').cast('string').to_pylist()
        a_arch = actors_table.column('archetype_path').cast('string').to_pylist()
        a_sx = actors_table.column('spawn_x').to_pylist()
        a_sy = actors_table.column('spawn_y').to_pylist()
        a_sz = actors_table.column('spawn_z').to_pylist()

        for i in range(len(actors_table)):
            if a_event[i] == 'open':
                loc_x = a_sx[i] if a_sx[i] is not None else 0
                loc_y = a_sy[i] if a_sy[i] is not None else 0
                loc_z = a_sz[i] if a_sz[i] is not None else 0
                class_path = a_class[i] if a_class[i] is not None else ""
                if class_path:
                    # First open wins: a GUID can be reused after a close, but
                    # the shot events that reference it belong to its first life.
                    guid_class.setdefault(a_guid[i], class_path)
                archetype = a_arch[i] if a_arch[i] is not None else _group_path_to_archetype(class_path)
                event = {
                    "type": "actor_spawned",
                    "time_ms": a_time[i],
                    "actor_net_guid": a_guid[i],
                    "replication_class_path": class_path,
                    "archetype_path": archetype,
                    "location": {"x": loc_x, "y": loc_y, "z": loc_z},
                }
                events.append((a_pid[i], a_time[i], event))
            else:
                event = {
                    "type": "actor_closed",
                    "time_ms": a_time[i],
                    "actor_net_guid": a_guid[i],
                }
                events.append((a_pid[i], a_time[i], event))

        if verbose:
            print(f"  {len(actors_table):,} actor lifecycle events from actors.parquet")
    else:
        # Fallback: infer from first/last field appearance (legacy behavior)
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

        for actor, (ms, pid) in actor_last.items():
            event = {
                "type": "actor_closed",
                "time_ms": ms,
                "actor_net_guid": actor,
            }
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
    # Build gameplay tag table for effect blob decoding
    tag_table = _build_tag_table(manifest)
    shot_count = 0
    resolved_weapon_count = 0

    def equippable_lookup(firing_state_guid):
        return _resolve_equippable(firing_state_guid, guid_outer, guid_path, guid_class)

    def fire_mode_lookup(firing_state_guid, source_id):
        return _resolve_fire_mode(firing_state_guid, source_id, guid_outer, guid_path)

    for (pid, actor, gp, handle), row_indices in rpc_groups.items():
        ms = col_time[row_indices[0]]

        # Determine RPC name from first field_name
        rpc_name = None
        payload = {}
        # Collect raw blobs for effect decoding
        float_blob = None
        object_blob = None
        vector_blob = None
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
            # Collect raw blobs for shot events
            if name == "ReplayPlayContinuousEffectAtLocation" and is_raw and col_raw[ri] is not None:
                if param == "FloatValues":
                    float_blob = bytes(col_raw[ri])
                elif param == "ObjectValues":
                    object_blob = bytes(col_raw[ri])
                elif param == "VectorValues":
                    vector_blob = bytes(col_raw[ri])
            if value is None and not is_raw:
                continue
            # Map parameter names to match C# parser output
            param_out = _normalize_rpc_param(rpc_name, param, value, is_raw)
            if param_out is not None:
                for k, v in param_out.items():
                    payload[k] = v

        if rpc_name is None:
            continue

        # Build valorant_shot_received for shot RPCs
        if rpc_name == "ReplayPlayContinuousEffectAtLocation":
            if float_blob is not None or object_blob is not None:
                channel = col_time[row_indices[0]]  # approximate
                shot_event = _build_shot_event(
                    ms, pid, actor, 0,
                    payload, float_blob, object_blob, vector_blob,
                    tag_table, equippable_lookup, fire_mode_lookup,
                )
                if shot_event is not None:
                    events.append((pid, ms, shot_event))
                    shot_count += 1
                    if shot_event["shot"]["equippable"] is not None:
                        resolved_weapon_count += 1
            # Still emit as rpc_received too (some downstream might need it)

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
        print(f"  {shot_count:,} valorant_shot_received events")
        pct = 100 * resolved_weapon_count / shot_count if shot_count else 0
        print(f"  {resolved_weapon_count:,} with a resolved weapon ({pct:.2f}%)")

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

        # Damage geometry: the parser now decodes these as quantized vectors
        # (previously raw blobs, because the C# custom decoder hid the type).
        # They arrive as the compact "(x,y,z)" string and the reference emits
        # {x, y, z}. Left as the raw payload if a value ever fails to parse --
        # visibly absent beats a fabricated zero vector.
        if param in DAMAGE_VECTOR_PARAMS:
            parsed = _parse_exact_vector(value)
            result[out_name] = parsed if parsed is not None else value
            return result

        # EquippableUsed: net GUID -> the C# ValorantEquippable shape.
        #
        # Name/ClassPath stay null and Category stays "unknown" because that is
        # what the C# parser emits: its resolver looks the GUID up in the
        # NetGuidCache path table, and weapon instances are dynamic actors that
        # never register a path there. valplay resolves the gun downstream from
        # actor_spawned instead (_actorindex.build_actor_class_index), so
        # filling these in here would diverge from the reference for no gain.
        #
        # This used to read the raw bits as a fixed little-endian uint16, which
        # was wrong twice over: the field is IntPacked (8/16/24 bits wide
        # depending on the value), and the low bit of the first byte is
        # IntPacked's continuation flag, so every multi-byte value came out odd
        # and could never be a valid dynamic NetGUID. The overlay now types the
        # field as ObjectNetGuid, so the parser hands us the decoded integer.
        if param == "EquippableUsed":
            if isinstance(value, int) and not is_raw:
                result[out_name] = {
                    "NetGuid": value,
                    "Name": None,
                    "ClassPath": None,
                    "Category": "unknown",
                }
            else:
                # Undecodable: pass the bits through rather than guessing.
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
