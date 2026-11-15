"""Convert a vrfkit Parquet export into a valplay-compatible NDJSON bundle.

Writes the events.ndjson, movement.ndjson and manifest.json that valplay's
compute_metrics.py consumes; only the serialization is bridged, no metric is
reimplemented. Replicated properties group by (packet_id, actor, object,
group_path) into export_group_received events; RPCs (group_path contains
'_ClassNetCache') by (packet_id, actor, object, group_path, handle) into rpc_received.
A shot RPC also becomes a valorant_shot_received event: its effect blobs are
decoded with the manifest's gameplay-tag table, and the gun is the
FiringState subobject's outer (net_guids.parquet) named from its class path
(actors.parquet, equippable_table.py).

Usage:
    python tools/to_valplay_bundle.py <vrfkit_export_dir> [-o <output_dir>]
"""

from __future__ import annotations

import argparse
import base64
import gc
import json
import math
import os
import re
import struct as _struct
import sys
import tempfile
import time
from collections import Counter, defaultdict
from functools import lru_cache
from pathlib import Path, PureWindowsPath
from typing import NamedTuple

try:
    import pyarrow as pa
    import pyarrow.compute as pc
    import numpy
except ImportError:
    sys.exit("pyarrow and numpy are required: pip install pyarrow numpy")

sys.path.insert(0, str(Path(__file__).parent))
from equippable_table import EQUIPPABLE_BY_PATH  # noqa: E402
from atomic_io import remove_tree, require_descendant  # noqa: E402
from wire_bits import read_parquet  # noqa: E402


# ---------------------------------------------------------------------------
# Constants and lookup tables
# ---------------------------------------------------------------------------

# Outer-chain hop cap (real chains are one hop): a cycle cannot hang a walk.
MAX_OUTER_DEPTH = 16

# vrf-export's reserved field name for a whole unresolved ClassNetCache block:
# preservation data, excluded before grouping (its path need not carry the suffix).
UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME = (
    "__vrfkit_unresolved_class_net_cache_payload__"
)

# Separates an RPC group from a property group. Both constants are pinned by
# crates/vrfkit/tests/adapter_contract.rs: a drift reclassifies every RPC.
CLASS_NET_CACHE_SUFFIX = "_ClassNetCache"

# Matched case-insensitively on every path of the FiringState outer chain;
# valplay's spray_control drops alternate fire (ADS and burst recoil differ).
ALTERNATE_FIRE_MARKERS = (
    "altfire",
    "zoomedfire",
    "zoomedfiring",
    "firingstateburst",
    "burstfiringstate",
    "burstmode",
)

# Array fields whose consumer bit-decodes the blob itself (valplay's
# collect_round_infos skips a non-dict), mapped to the TypeName the reference
# labels it with; the general rule keeps the decoded elements instead
# (CombatReport's `Rounds` needs them). The raw-blob rule, here and in
# `_RAW_SOURCED_RPC_PARAMS`: built from raw_bits whenever the row has them,
# typed or not (`_get_value` reports raw only when every typed column is null).
RAW_BLOB_PREFERRED = {
    "RoundInfos": "TArray<FAresPlayerRoundInfo>",
}

# The two damage RPCs, by the function name `_split_rpc_field` returns.
DAMAGE_RPC_NAMES = frozenset({
    "MulticastNotifyDamage_Point",
    "MulticastNotifyDamage_Base",
})

# Damage RPC parameters the reference bundle emits as labelled blobs, and
# rpc_received keeps as blobs, although the parser types them (ObjectNetGuid).
DEATH_MONTAGE_BLOB_PARAMS = frozenset({
    "DeathMontageEffectOverride",
    "DeathMontageEffectOverrideContext",
})

# RPC parameters built from raw_bits by the raw-blob rule, so typing one
# upstream cannot swap its blob for a value its consumer cannot read: the shot
# effect arrays (`_decode_effect_elements` needs the exact window), a damage
# RPC's LifeChangeEvents (valplay's `_decode_remaining_hp` reads only bits)
# and the death-montage pair. valplay reads either shape of its other
# RETAINED_RAW_BLOB_KEYS.
_RAW_SOURCED_RPC_PARAMS = {
    "ReplayPlayContinuousEffectAtLocation":
        frozenset({"FloatValues", "ObjectValues", "VectorValues"}),
    **dict.fromkeys(DAMAGE_RPC_NAMES,
                    DEATH_MONTAGE_BLOB_PARAMS | {"LifeChangeEvents"}),
}


# Replicated properties whose "(x,y,z)" value_str the reference emits as
# {x, y, z} (the only one on 02d4d478). Listed by name, never sniffed: a value
# that merely looks like a vector is not evidence that it is one.
VECTOR_PROPERTIES = frozenset({
    "ReplicatedGravityDirection",
})


# Replicated properties the parser writes as a JSON object in value_str
# (FRepMovement fits no single column), listed by name: the overlay's only
# RepMovement name. Passed through untouched; `location` is in world units,
# and an export carrying location/100 is regenerated, never rescaled.
JSON_OBJECT_PROPERTIES = frozenset({
    "ReplicatedMovement",
})


# Damage RPC parameters carrying an FVector_NetQuantize* payload: DamageOrigin
# on the shared damage base, the four impact fields on point damage only.
DAMAGE_VECTOR_PARAMS = frozenset({
    "DamageOrigin",
    "DamageImpactLocation",
    "DamageImpactBoneRelativeLocation",
    "DamageDirection",
    "DamageImpactNormal",
})


# EAresAlliance ordinal -> the reference's string; an unmapped ordinal becomes
# a loud alliance_unknown_{n}.
ALLIANCE_MAP = {
    0: "alliance_ally",
    1: "alliance_enemy",
    2: "alliance_neutral",
    3: "alliance_any",
    4: "alliance_count",
    5: "alliance_max",
}

# EAresRegionalDamage: the four observed strings match the reference; the
# three sentinels follow their rule ("_" before each capital, lowercase).
REGIONAL_DAMAGE_MAP = {
    0: "regional_damage__normal",
    1: "regional_damage__headshot",
    2: "regional_damage__legshot",
    3: "regional_damage__region_count",     # derived
    4: "regional_damage__invalid__radial",  # derived
    5: "regional_damage__invalid",
    6: "regional_damage__count_plus_one",   # derived
}


# Nested path parser: "Rounds[0].Reports[1].Interactions[2].DamageDealt"
# -> [("Rounds", 0), ("Reports", 1), ("Interactions", 2), ("DamageDealt", None)]
_PATH_RE = re.compile(r'([A-Za-z_][A-Za-z0-9_]*)(?:\[(\d+)\])?')


# ---------------------------------------------------------------------------
# Loss accounting
# ---------------------------------------------------------------------------
class _Tally(dict):
    """Every row this conversion dropped and every value it invented: counted,
    not raised (property key collisions are structural, tens of thousands per
    replay), because an uncounted drop here is invisible to every upstream
    check. The key set is fixed: `bump` on an unknown name raises KeyError.
    """

    #: counter -> the wording its summary line uses.
    REASONS = {
        "unnamed_property_rows":
            "property rows with no field name (value dropped)",
        "unnamed_rpc_rows":
            "RPC rows with no field name (parameter dropped)",
        "unnamed_rpc_invocations":
            "RPC invocations dropped whole (no row supplied a name)",
        "rpc_param_collisions":
            "RPC parameters overwritten inside one invocation group",
        "property_key_collisions":
            "property values overwritten by a same-named row in one event",
        "unparsable_path_segments":
            "field-path segments kept as literal object keys",
        "payload_shape_conflicts":
            "payload values destroyed by a row of an incompatible shape",
        "multi_typed_rows":
            "rows with more than one typed column set (extras discarded)",
        "fabricated_shot_locations":
            "shot locations fabricated as the world origin",
        "fabricated_shot_rotations":
            "shot rotations fabricated as (0,0,0), indistinguishable from a "
            "real aim of world +X",
        "effect_half_read_pairs":
            "shot effect (tag, value) pairs with one half unreadable "
            "(pair dropped, e.g. a missing FiringState.AttackVector.N)",
        "effect_array_residual_bits":
            "shot effect blobs with bits left over after decoding "
            "(framing did not end where this parse expected)",
        "empty_gameplay_tag_table":
            "shots decoded with no gameplay-tag table (fields keyed by index)",
        "events_time_ms_regressions":
            "events whose time_ms is lower than the line before (tie-break unsound)",
        "upstream_row_count_disagreement":
            "row counts the export declared that its own tables contradict",
        "unknown_actor_lifecycle_events":
            "actors.event values this adapter has no event type for "
            "(published as actor_lifecycle_unknown)",
        "non_finite_movement_rows":
            "movement rows written with a non-finite value (spelled "
            "Infinity/NaN; strict JSON parsers reject the line)",
        "raw_blobs_unavailable":
            "wire blobs a consumer decodes itself (RoundInfos, damage "
            "LifeChangeEvents, shot effect arrays) that could not be built: "
            "the row had no raw bits, or only its decoded members arrived",
        "damaged_bone_undecoded":
            "DamagedBone values the parser could not decode (published as "
            "null, never guessed from the raw bytes)",
    }

    def __init__(self):
        super().__init__((name, 0) for name in self.REASONS)

    def bump(self, name: str, n: int = 1) -> None:
        self[name] = self[name] + n

    @property
    def total(self) -> int:
        return sum(self.values())

    def lines(self) -> list[str]:
        """One line per counter, zeros included: a line printed only when
        non-zero cannot tell "nothing was lost" from "this stopped running"."""
        return [f"  {name}: {self[name]:,} -- {reason}"
                for name, reason in self.REASONS.items()]


def _bump(tally, name: str, n: int = 1) -> None:
    """Increment a counter; a `None` tally is a no-op, so leaf helpers stay
    callable alone."""
    if tally is not None:
        tally.bump(name, n)


# ---------------------------------------------------------------------------
# Scalar and vector formatting: three load-bearing precision policies, each
# named by its function.
# ---------------------------------------------------------------------------
def _f32_shortest(value):
    """Shortest decimal that round-trips through float32, as System.Text.Json
    writes a float: 2382.2f is "2382.2", not the widened 2382.199951171875."""
    if value is None:
        return None
    packed = _struct.unpack("f", _struct.pack("f", value))[0]
    for digits in range(1, 10):
        candidate = float(f"{packed:.{digits}g}")
        # A candidate rounded past FLT_MAX cannot round-trip: 3.12 packs it
        # as inf, 3.13 raises OverflowError.
        try:
            back = _struct.unpack("f", _struct.pack("f", candidate))[0]
        except OverflowError:
            continue
        if back == packed:
            return int(candidate) if candidate.is_integer() else candidate
    return int(packed) if float(packed).is_integer() else packed


#: One reused encoder: `json.dumps` with kwargs builds one per call (1.8 s).
_JSON = json.JSONEncoder(separators=(',', ':'), ensure_ascii=True)

#: The movement record as `_JSON.encode` wrote its dict, one `%s` per slot:
#: time_ms, character_net_guid, then `_MOVEMENT_COLUMNS[3:]` in order.
_MOVEMENT_LINE = (
    '{"time_ms":%s,"shooter_character_net_guid":%s,'
    '"position":{"x":%s,"y":%s,"z":%s},'
    '"velocity":{"x":%s,"y":%s,"z":%s},'
    '"yaw":%s,"pitch":%s}\n'
)

#: Rows per movement.ndjson block assembled in Arrow (~48 MB of text): 2**16
#: and 2**18 run equally fast, 2**14 ~4% slower, 2**20 adds 50-80 MB of peak.
_MOVEMENT_BLOCK_ROWS = 1 << 18


#: Below this magnitude every integer is exact in float32, so an integral
#: float32's shortest round-trip text IS its integer text. Above, they part:
#: 123456792.0 round-trips as 123456790.
_F32_EXACT_INT_LIMIT = 2 ** 24

#: numpy's float32 text is positional only for 1e-4 <= |v| < 1e6 (judged on
#: the binary value), repr's from 1e-4 to 1e16 (on the decimal): 1234567.5 is
#: '1.2345675e+06' to numpy. Compared in float64: 1e-4 is not a float32.
_F32_POSITIONAL_BAND = (1e-4, 1e6)


def _json_scalar_column(arr, *, shorten=False):
    """The JSON text of a 1-D numpy float array, once per distinct value.

    Returns ``(texts, inverse)``: a pa.string() array of each distinct value's
    text and a pa.int32() array mapping every row to it, so
    `texts.take(inverse)` is the column's text with no Python object per row.
    The columns are quantized: 02d4d478's six shortened ones hold 1,499,222
    distinct values in 11,023,320.

    The contract is per value (`MovementTextRuleTests`): `_JSON.encode(
    _f32_shortest(v))` with `shorten=True`, else `_JSON.encode(v)`,
    `Infinity`/`NaN` included. Unique over bit patterns, so -0.0 keeps its
    sign. `shorten=True` vectorises only where proven equal over every float32
    (numpy 2.5.2): an integral value below `_F32_EXACT_INT_LIMIT` is its int
    text, a non-integral one inside `_F32_POSITIONAL_BAND` numpy's Dragon4
    astype(str). Anything else, non-finite included, takes the encoder.
    """
    if arr.dtype.kind != "f" or arr.dtype.itemsize not in (4, 8):
        # The bit-pattern view needs a same-width unsigned type.
        raise TypeError(f"_json_scalar_column wants float32/float64, got {arr.dtype}")
    if arr.shape[0] > numpy.iinfo(numpy.int32).max:
        # The int32 inverse would wrap and point rows at wrong texts.
        raise ValueError(f"{arr.shape[0]:,} rows is more than an int32 index can address")
    ubits, inverse = numpy.unique(
        arr.view(numpy.dtype(f"u{arr.dtype.itemsize}")), return_inverse=True
    )
    # int32 halves what the caller holds per column until the write.
    inverse = pa.array(inverse.astype(numpy.int32))
    uniq = ubits.view(arr.dtype)
    if shorten:
        # An object array: a wide int text would truncate to a `<U` width.
        magnitude = numpy.abs(uniq.astype(numpy.float64))
        finite = numpy.isfinite(uniq)
        integral = finite & (uniq == numpy.trunc(uniq))
        low, high = _F32_POSITIONAL_BAND
        as_int = integral & (magnitude < _F32_EXACT_INT_LIMIT)
        as_dragon4 = finite & ~integral & (magnitude >= low) & (magnitude < high)
        texts = numpy.empty(uniq.shape[0], dtype=object)
        if as_dragon4.any():
            texts[as_dragon4] = uniq[as_dragon4].astype(str)
        if as_int.any():
            texts[as_int] = uniq[as_int].astype(numpy.int64).astype(str)
        per_value = ~(as_int | as_dragon4)
        if per_value.any():
            encode = _JSON.encode
            texts[per_value] = [encode(_f32_shortest(v))
                                for v in uniq[per_value].tolist()]
    else:
        encode = _JSON.encode
        texts = [encode(float(v)) for v in uniq.tolist()]
    return pa.array(texts, type=pa.string()), inverse


def _vec3(x, y, z) -> dict:
    """Build an {x, y, z} dict with integral components as ints, as
    System.Text.Json writes a double (0.0 as `0`)."""
    return {
        axis: (int(n) if float(n).is_integer() else n)
        for axis, n in zip(("x", "y", "z"), (x, y, z))
    }


def _parse_vector_or_none(val):
    """A "(x,y,z)" vector at full precision (the reference's damage direction
    is 0.055482650227362894), integral components as ints; None, never a zero
    vector, if unparseable. A dict passes through."""
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
    return _vec3(*nums)


def _parse_vector_or_zero(val, tally=None) -> dict:
    """A shot Location as {x, y, z} at full precision (shot_rays matches it
    only unrounded). An absent or unparseable one is the world origin, counted
    as `fabricated_shot_locations`: every effect RPC emits a shot, and an
    origin is indistinguishable from a real coordinate downstream."""
    parsed = _parse_vector_or_none(val)
    if parsed is None:
        _bump(tally, "fabricated_shot_locations")
        return {"x": 0, "y": 0, "z": 0}
    return parsed


def _parse_rotation(val, tally=None) -> dict:
    """A Rotation as {pitch, yaw, roll}; the (0,0,0) fallback is counted as
    `fabricated_shot_rotations`: spray_control reads it as the real aim."""
    if val is None:
        _bump(tally, "fabricated_shot_rotations")
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
                # Rust writes shortest f32 decimals; widening each through f32
                # matches the raw-wire decoder and the reference exactly.
                components = [
                    _struct.unpack("<f", _struct.pack("<f", float(part)))[0]
                    for part in parts
                ]
                return {"pitch": components[0],
                        "yaw": components[1],
                        "roll": components[2]}
            except ValueError:
                pass
    _bump(tally, "fabricated_shot_rotations")
    return {"pitch": 0, "yaw": 0, "roll": 0}


# ---------------------------------------------------------------------------
# NetGUID outer-chain resolution (weapon identity and fire mode)
# ---------------------------------------------------------------------------
def _has_alternate_marker(value) -> bool:
    if not value:
        return False
    lowered = str(value).lower()
    return any(marker in lowered for marker in ALTERNATE_FIRE_MARKERS)


def _resolve_fire_mode(firing_state_guid, source_id, guid_outer, guid_path):
    """``(fire_mode, evidence)`` from the NAME of the firing-state subobject:
    "FiringState" is primary, "ZoomedFiringState", "FiringStateBurst", ...
    alternate, "unknown" when no path resolved. Not burst_shot_number, which
    only indexes shots within a spray."""
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
    """Walk a GUID's outer chain to the equippable actor containing it:
    ``(owner_net_guid, name, category, class_path)`` or ``None``.

    Two lookups per hop: ``guid_class`` (actors.parquet: channel opens, with
    the spawn class path) and ``guid_path`` (net_guids.parquet: every
    registered GUID, subobjects included). A weapon appears in the first, its
    FiringState only in the second.
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
    """Read net_guids.parquet into (guid -> outer, guid -> path, row count);
    the count is the table's height, since both dicts drop rows."""
    table = read_parquet(export_dir / "net_guids.parquet")
    guids = table.column("net_guid").to_pylist()
    paths = table.column("path").cast("string").to_pylist()
    outers = table.column("outer_net_guid").to_pylist()
    guid_outer = {g: o for g, o in zip(guids, outers) if o is not None}
    guid_path = {g: p for g, p in zip(guids, paths) if p}
    return guid_outer, guid_path, len(table)


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

    def tell(self) -> int:
        return self._pos

    def read_bits(self, n: int) -> int:
        """`n` bits LSB first. A short read leaves the position at the end
        and raises EOFError; a declared length past the buffer raises
        IndexError (aborting the conversion) where a slice would pad zeros."""
        start = self._pos
        stop = min(start + n, self._bit_len)
        if stop > start and stop > len(self._data) * 8:
            raise IndexError("index out of range")
        if start + n > self._bit_len:
            self._pos = self._bit_len
            raise EOFError
        self._pos = start + n
        chunk = self._data[start >> 3:(start + n + 7) >> 3]
        return (int.from_bytes(chunk, "little") >> (start & 7)) & ((1 << n) - 1)

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
        return _struct.unpack('<d', _struct.pack('<Q', self.read_bits(64)))[0]

    def skip_bits(self, n: int):
        self._pos = min(self._pos + n, self._bit_len)


def _read_effect_vector(r: _BitReader):
    # Left to right: a short read on y or z stops where sequential reads would.
    return (r.read_f64(), r.read_f64(), r.read_f64())


class _EffectArraySpec(NamedTuple):
    """How to read one of the three effect value arrays: same wire shape,
    different tag/value handles (the containing struct's own, hence not
    contiguous) and value reader."""

    tag_handle: int
    value_handle: int
    read_value: object


_EFFECT_FLOATS = _EffectArraySpec(7, 8, _BitReader.read_f32)
_EFFECT_VECTORS = _EffectArraySpec(11, 12, _read_effect_vector)
_EFFECT_OBJECTS = _EffectArraySpec(15, 16, _BitReader.read_int_packed)


def _decode_effect_elements(data: bytes, bit_count: int, spec: _EffectArraySpec,
                            tally=None):
    """Decode one effect value array -> list of (tag_index, value) tuples.

    ``spec.read_value`` must raise on a short read, not return a sentinel:
    a failed read leaves the earlier value untouched. Each leaf reader ends
    at its declared payload boundary. An independent port of effect.rs over the raw
    blob: where Rust rejects a blob whole, this keeps what it decoded and
    counts the two shapes that fabricate values downstream, a half-read pair
    (dropped by `_decode_effect_blob`) and any bit left after the loop.
    """
    r = _BitReader(data, bit_count)
    try:
        count = r.read_int_packed()
    except (EOFError, ValueError):
        count = None
    # No count, or one past Rust's MAX_ARRAY_COUNT (256), still reaches the
    # residual check; count 0 enters the loop to consume Rust's terminator.
    framed = count is not None and count <= 256
    elements = [(None, None)] * count if framed else []
    while framed and not r.at_end():
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
            # Share the bytes and absolute bit offsets, but not the array's end.
            payload = _BitReader(data, r.tell() + payload_bits)
            payload.skip_bits(r.tell())
            r.skip_bits(payload_bits)
            if handle == spec.tag_handle:
                try:
                    tag = payload.read_int_packed()
                except (EOFError, ValueError):
                    pass
            elif handle == spec.value_handle:
                try:
                    val = spec.read_value(payload)
                except (EOFError, ValueError):
                    pass
        elements[idx] = (tag, val)
    # A declared slot missing either half, never visited or lost mid-read, is
    # a pair `_decode_effect_blob` drops.
    half_read = sum(1 for tag, val in elements if tag is None or val is None)
    if half_read:
        _bump(tally, "effect_half_read_pairs", half_read)
    # Any bit left is Rust's `ResidualBits`: bit_count is exact, not padded.
    if r.bits_remaining():
        _bump(tally, "effect_array_residual_bits")
    return elements


def _decode_effect_blob(blob: _EffectBlob | None, spec: _EffectArraySpec,
                        tag_table: dict, tally=None) -> dict:
    """Decode one effect blob into {tag_name: value}; an absent blob is {}.

    Half-read pairs are dropped, counted by `_decode_effect_elements`: a
    missing `FiringState.AttackVector.N` is how `_build_shot_event` notices.
    """
    if blob is None:
        return {}
    decoded = {}
    for tag_idx, val in _decode_effect_elements(blob.data, blob.bit_count, spec,
                                                tally):
        if tag_idx is not None and val is not None:
            decoded[tag_table.get(tag_idx, str(tag_idx))] = val
    return decoded


def _build_tag_table(manifest: dict) -> dict:
    """Build tag_index -> tag_name from the manifest's gameplay tag group."""
    groups = manifest.get("net_field_export_groups", [])
    for g in groups:
        if g.get("path") == "NetworkGameplayTagNodeIndex":
            return {f["handle"]: f["name"] for f in g.get("fields", [])}
    return {}


def _decode_rotation_short(data: bytes, bit_count: int):
    """A UE4 RotationShort: per axis a non-zero bit, then 16 bits of
    value * 360 / 65536 degrees. None on a short read, so the caller counts a
    fabricated rotation instead of publishing the unread axes as 0."""
    r = _BitReader(data, bit_count)
    try:
        return {axis: r.read_bits(16) * 360.0 / 65536.0 if r.read_bits(1) else 0.0
                for axis in ("pitch", "yaw", "roll")}
    except EOFError:
        return None


# ---------------------------------------------------------------------------
# Shot events
# ---------------------------------------------------------------------------
class _EffectBlob(NamedTuple):
    """One undecoded value array and the bit length the parser declared:
    len(data) * 8 would decode up to 7 padding bits as data."""

    data: bytes
    bit_count: int


class _EffectBlobs(NamedTuple):
    """The three undecoded value arrays a shot RPC may carry. Any may be absent."""

    floats: _EffectBlob | None = None
    objects: _EffectBlob | None = None
    vectors: _EffectBlob | None = None


class _ShotContext(NamedTuple):
    """Per-replay lookups every shot event needs; with empty GUID tables the
    equippable is null and fire_mode is "unknown" unless SourceID names
    alternate fire (a server-world effect's does not)."""

    tag_table: dict
    guid_outer: dict = {}
    guid_path: dict = {}
    guid_class: dict = {}


def _build_shot_event(
    ctx: _ShotContext,
    time_ms, packet_id, actor_net_guid, object_net_guid, channel_index,
    scalar_params: dict, blobs: _EffectBlobs, tally=None,
) -> dict:
    """A valorant_shot_received event from one effect RPC's params, always: a
    server-world effect (172 of 02d4d478's 2,647) has no firing state and
    comes out with a null equippable and fire_mode "unknown", as the
    reference emits it; valplay's affected sections guard on
    firing_player_state or attack_vectors."""
    tag_table = ctx.tag_table
    floats = _decode_effect_blob(blobs.floats, _EFFECT_FLOATS, tag_table, tally)
    objects = _decode_effect_blob(blobs.objects, _EFFECT_OBJECTS, tag_table, tally)
    vectors = _decode_effect_blob(blobs.vectors, _EFFECT_VECTORS, tag_table, tally)

    # Location and Rotation arrive under their bare handles "248"/"249"
    # (typed on 2,647 of 2,647 shots on 02d4d478). A {BitCount, Data} blob
    # there is a row the overlay did not type: decode the wire. Keep it.
    location = scalar_params.get("248")
    if isinstance(location, dict) and "Data" in location:
        raw = base64.b64decode(location["Data"])
        location = _vec3(*_struct.unpack_from("<3d", raw)) if len(raw) >= 24 else None
    rotation = scalar_params.get("249")
    if isinstance(rotation, dict) and "Data" in rotation:
        raw = base64.b64decode(rotation["Data"])
        rotation = _decode_rotation_short(raw, rotation.get("BitCount", len(raw) * 8))
    effect_id = scalar_params.get("EffectID")
    source_id = scalar_params.get("SourceID")
    start_time = scalar_params.get("StartMovementTime")
    is_local = scalar_params.get("bLocalEffect")
    is_transient = scalar_params.get("bTransient")
    wait_on = scalar_params.get("WaitOnReplicationActor")
    alliance = scalar_params.get("AllianceFilter")

    loc_obj = _parse_vector_or_zero(location, tally)
    rot_obj = _parse_rotation(rotation, tally)

    attack_keys = (f"FiringState.AttackVector.{i}" for i in range(1, 16))
    attack_vectors = [{"x": x, "y": y, "z": z} for x, y, z in
                      (vectors[key] for key in attack_keys if key in vectors)]

    burst = floats.get("FiringState.BurstShotNumber")
    yaw_switch = floats.get("FiringState.YawSwitch")

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

    # FiringState's outer is the gun; null, never guessed, when the chain does
    # not reach a known equippable.
    equippable = None
    if firing_state:
        hit = _resolve_equippable(firing_state, ctx.guid_outer, ctx.guid_path,
                                  ctx.guid_class)
        if hit is not None:
            owner_guid, name, category, class_path = hit
            equippable = {
                "net_guid": owner_guid,
                "name": name,
                "category": category,
                "class_path": class_path,
            }

    fire_mode, fire_mode_evidence = _resolve_fire_mode(
        firing_state, source_id, ctx.guid_outer, ctx.guid_path)

    # Only an int is an enum ordinal; anything else (the {BitCount, Data} blob
    # of an untyped field) passes through unchanged, like every shot param.
    alliance_str = alliance
    if isinstance(alliance, int):
        alliance_str = ALLIANCE_MAP.get(alliance, f"alliance_unknown_{alliance}")

    shot = {
        "effect_id": effect_id,
        # Float32 (as is random_seed): the reference prints the shortest
        # round-trip (12.780108), not the widened value.
        "start_movement_time": _f32_shortest(start_time)
        if isinstance(start_time, float) else start_time,
        "source_id": source_id,
        "is_local_effect": bool(is_local),
        "is_transient": True if is_transient is None else bool(is_transient),
        "wait_on_replication_actor": wait_on or 0,
        # Null when absent, as in the reference: a default merges two states.
        "alliance_filter": alliance_str,
        "location": loc_obj,
        "rotation": rot_obj,
        "ammo_remaining": ammo,
        # Null when absent: a default of 1 would rewrite a genuine 0, and
        # compute_metrics.py reads it with no default of its own.
        "num_projectiles": num_proj,
        "random_seed": _f32_shortest(random_seed)
        if isinstance(random_seed, float) else random_seed,
        "tracer_option": tracer_opt,
        "burst_shot_number": burst,
        "yaw_switch": yaw_switch,
        "firing_player_state": firing_player,
        "firing_state": firing_state,
        "attack_vectors": attack_vectors,
        # An equippable GUID in the blob: never populated (0 of 2,647 shots).
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
        # fields.parquet's values, as the reference has them.
        "object_net_guid": object_net_guid,
        "channel": channel_index,
        "shot": shot,
    }


# ---------------------------------------------------------------------------
# Combat report leaf labels
#
# The parser labels each flattened array leaf with the name the replay
# declares, wrong for this bundle twice over: compute_metrics.py reads the
# reference's names (the wire spells six differently, e.g. bDidKill and
# Riot's DamageRecieved), and declared names repeat within one element
# (HUDConfig at eight positions: keyed by name, 3,405 of 02d4d478's 20,298
# payload paths would merge). Resolve handles through this replay's declarations
# to the reference's member names; unknown members remain `_h{handle}`.
# ---------------------------------------------------------------------------
COMBAT_REPORT_GROUP = "CombatReportComponent"

# Legacy handle -> the member name the reference format uses. Leaf handles only: the
# containers (4 Reports, 10 Interactions, 26 DealtInteractions,
# 61 ReceivedInteractions, 44/79 Regions) keep the parser's schema names.
COMBAT_REPORT_REFERENCE_NAMES = {
    3: "RoundNumber",              # wire: RoundNum
    5: "RoundNumber",
    11: "Subject",                 # wire: ParticipantSubject
    12: "Team",                    # wire: ParticipantTeamName
    13: "CharacterIcon",           # wire: ParticipantCharacterIcon
    18: "DamageDealt",
    19: "HitsDealt",
    20: "DamageReceived",          # wire: DamageRecieved (Riot's typo)
    21: "HitsReceived",            # wire: HitsRecieved (Riot's typo)
    22: "DidKill",                 # wire: bDidKill
    23: "AssistType",
    24: "KillerPlayerState",       # wire: ParticipantsKillerState
    25: "WasKiller",               # wire: bWasKiller
    45: "Region",
    46: "Hits",
    47: "Damage",
    48: "IsWallPen",               # wire: bIsWallPen
    49: "IsKill",                  # wire: bIsKill
    50: "DestroyedArmor",
    80: "Region",
    81: "Hits",
    82: "Damage",
    83: "IsWallPen",               # wire: bIsWallPen
    84: "IsKill",                  # wire: bIsKill
    85: "DestroyedArmor",
    96: "CombatReportIndex",
    98: "ResurrectorPlayerState",
    103: "Died",                   # wire: bDied
}


# Sub-array container handles: such a row already carries the walker's schema
# name, the reference's spelling, and is left alone.
COMBAT_REPORT_CONTAINER_HANDLES = frozenset({4, 10, 26, 44, 61, 79})


COMBAT_REPORT_WIRE_NAMES = {name: name for name in COMBAT_REPORT_REFERENCE_NAMES.values()}
COMBAT_REPORT_WIRE_NAMES.update({
    "RoundNum": "RoundNumber", "ParticipantSubject": "Subject",
    "ParticipantTeamName": "Team", "ParticipantCharacterIcon": "CharacterIcon",
    "DamageRecieved": "DamageReceived", "HitsRecieved": "HitsReceived",
    "bDidKill": "DidKill", "ParticipantsKillerState": "KillerPlayerState",
    "bWasKiller": "WasKiller", "bIsWallPen": "IsWallPen", "bIsKill": "IsKill",
    "bDied": "Died",
})
COMBAT_REPORT_CONTAINER_NAMES = frozenset({
    "Reports", "Interactions", "DealtInteractions", "ReceivedInteractions", "Regions",
})


def _combat_report_declared_names(manifest: dict) -> dict[str, dict[int, str]]:
    """Use this replay's descriptor handles; repeated unknown names stay distinct.

    The legacy handle table remains only for exports without a declaration.
    Adding a member shifts later handles, including otherwise unchanged names.
    """
    return {
        group["path"]: {
            field["handle"]: COMBAT_REPORT_WIRE_NAMES.get(
                field.get("name"), f"_h{field['handle']}")
            for field in group.get("fields", [])
        }
        for group in manifest.get("net_field_export_groups") or []
        if COMBAT_REPORT_GROUP in group.get("path", "")
    }


def _combat_report_leaf_name(group_path: str, field_name: str, handle,
                             declared_names=None) -> str:
    """Normalize regional containers and resolve the leaf's reference name.

    Synthesized raw tails and depth-limit container rows keep their leaf label.
    """
    if not group_path or COMBAT_REPORT_GROUP not in group_path:
        return field_name
    if not field_name.startswith("Rounds["):
        return field_name
    # The schema may use the wire's container spelling; the bundle has always
    # called both regional arrays Regions, independently of their handles.
    regional_layout = ".RegionalDamageInteractions[" in field_name
    field_name = field_name.replace(".RegionalDamageInteractions[", ".Regions[")
    if field_name.endswith(".RegionalDamageInteractions"):
        field_name = field_name[:-len("RegionalDamageInteractions")] + "Regions"
    head, dot, leaf = field_name.rpartition(".")
    if not dot:
        return field_name
    if leaf == "_raw" or leaf in COMBAT_REPORT_CONTAINER_NAMES:
        return field_name
    if declared_names is not None and group_path in declared_names:
        name = declared_names[group_path].get(handle, f"_h{handle}")
    elif regional_layout:
        # These rows name a newer regional layout even when the manifest lacks
        # descriptors. Never assign an old handle's meaning to its shifted leaf.
        name = COMBAT_REPORT_WIRE_NAMES.get(leaf, f"_h{handle}")
    else:
        if handle in COMBAT_REPORT_CONTAINER_HANDLES:
            return field_name
        name = COMBAT_REPORT_REFERENCE_NAMES.get(handle, f"_h{handle}")
    return head + dot + name


# ---------------------------------------------------------------------------
# Field rows -> nested payloads
# ---------------------------------------------------------------------------
def _normalize_prop_field_name(field_name: str, is_bool: bool) -> str:
    """Strip the 'b' of a boolean property name ('bUltimateActive' ->
    'UltimateActive'), as the reference format does and compute_metrics.py expects."""
    if is_bool and field_name.startswith('b') and len(field_name) > 1 and field_name[1].isupper():
        return field_name[1:]
    return field_name


@lru_cache(maxsize=None)
def _parse_field_path_cached(path: str):
    """Parse one path: `(parts, unparsable_segment_count)`, memoised (a
    replay's ~520k parses cover a few thousand distinct paths). The caller
    bumps the tally on every call, so cache hits cannot under-count."""
    parts = []
    unparsable = 0
    for seg in path.split('.'):
        m = _PATH_RE.fullmatch(seg)
        if m:
            name = m.group(1)
            idx = int(m.group(2)) if m.group(2) is not None else None
            parts.append((name, idx))
        else:
            # Bare handle numbers ("248") and Blueprint names with spaces are
            # literal keys; a bracket means unread subscripts, and is counted.
            if '[' in seg or ']' in seg:
                unparsable += 1
            parts.append((seg, None))
    return tuple(parts), unparsable


@lru_cache(maxsize=None)
def _container_names(path: str) -> tuple:
    """Every prefix of `path` ending at a '[': the containers its row implies."""
    return tuple(path[:i] for i, c in enumerate(path) if c == '[')


def _parse_field_path(path: str, tally=None):
    """Parse a dot-separated field path with optional array indices."""
    parts, unparsable = _parse_field_path_cached(path)
    if unparsable:
        _bump(tally, "unparsable_path_segments", unparsable)
    return list(parts)


class _Node(dict):
    """A dict `_set_nested` built to hold rows' members. The type tells it
    from a row's own dict value (a raw blob, a vector): a row replacing
    members restructures the key, one replacing a value overwrites it."""

    __slots__ = ()


def _count_overwrite(tally, previous) -> None:
    """Count a row's value replacing `previous`, something already present,
    under the one reason that describes it: members `_set_nested` built (a
    `_Node` or an array) are a shape conflict, any row's own value, typed or
    a raw blob, a same-named overwrite. An empty `_Node` is an array filler
    and holds nothing."""
    if isinstance(previous, (_Node, list)):
        if previous:
            _bump(tally, "payload_shape_conflicts")
    else:
        _bump(tally, "property_key_collisions")


def _set_nested(root: dict, parts: list, value, tally=None):
    """Set `value` at `parts` (a parsed flat path such as
    'Rounds[0].Reports[0].DamageDealt') in a nested dict/list payload.

    Arrays grow with None/_Node fillers, and each element gets an 'Index'
    equal to its subscript, as in the reference (compute_metrics dedups on
    it). A replacement counts once: rows disagreeing about a key's shape
    ('Foo' scalar, 'Foo.Bar' nested) are `payload_shape_conflicts`, a value
    landing on another row's value (a raw blob included)
    `property_key_collisions`. A deeper row reaching a row's own dict value,
    or a real `Index` row replacing the injected one, loses nothing.
    """
    obj = root
    element = None  # the subscript `obj` sits at, when it is an array element
    last = len(parts) - 1
    for i, (name, idx) in enumerate(parts):
        is_last = i == last
        if idx is not None:
            arr = obj.setdefault(name, [])
            if not isinstance(arr, list):
                _bump(tally, "payload_shape_conflicts")
                arr = obj[name] = []
            while len(arr) <= idx:
                arr.append(None if is_last else _Node())
            if is_last:
                if arr[idx] is not None:
                    _count_overwrite(tally, arr[idx])
                arr[idx] = value
            else:
                if not isinstance(arr[idx], dict):
                    if arr[idx] is not None:
                        _bump(tally, "payload_shape_conflicts")
                    arr[idx] = _Node()
                arr[idx].setdefault("Index", idx)
                obj = arr[idx]
                element = idx
        elif is_last:
            # An Index equal to the element's subscript is the injected one.
            if name in obj and not (name == "Index" and obj[name] == element):
                _count_overwrite(tally, obj[name])
            obj[name] = value
        else:
            nxt = obj.setdefault(name, _Node())
            if not isinstance(nxt, dict):
                _bump(tally, "payload_shape_conflicts")
                nxt = obj[name] = _Node()
            obj = nxt
            element = None


def _drop_padding_elements(node):
    """Remove the empty `_Node` fillers `_set_nested` appends to reach a
    sparse index: compute_metrics sorts `{t["Index"]: t for t in teams}`,
    which a filler's None key breaks. A genuine element carries at least its
    `Index`; `None` fillers in scalar arrays stay (the position IS the index).
    """
    if isinstance(node, dict):
        for value in node.values():
            _drop_padding_elements(value)
    elif isinstance(node, list):
        node[:] = [e for e in node if e != {}]
        for value in node:
            _drop_padding_elements(value)
    return node


def _get_value(row_i64, row_f64, row_bool, row_str, row_raw, row_bits,
               tally=None):
    """A fields.parquet row's typed value, else its raw_bits blob:
    ``(value, is_raw)``. More than one typed column set keeps the first and
    is counted as `multi_typed_rows`; raw_bits rides beside a typed value by
    design."""
    # Bools summed, not a generator: this runs per row (1.4 M on a replay).
    if ((row_i64 is not None) + (row_f64 is not None)
            + (row_bool is not None) + (row_str is not None)) > 1:
        _bump(tally, "multi_typed_rows")
    if row_i64 is not None:
        return row_i64, False
    if row_f64 is not None:
        return row_f64, False
    if row_bool is not None:
        return row_bool, False
    if row_str is not None:
        return row_str, False
    if row_raw is not None:
        return _raw_blob(row_raw, row_bits), True
    return None, False


def _raw_blob(row_raw, row_bits) -> dict:
    """The {BitCount, Data} blob of one row's raw bits, as the reference format
    has it. The one builder, so the key order (part of the bytes) has one source;
    a caller that labels the blob adds `TypeName` after these two."""
    return {"BitCount": row_bits,
            "Data": base64.b64encode(row_raw).decode('ascii')}


def _count_unbuilt_blobs(blob_state, tally) -> None:
    """Count the raw-sourced fields one event needed and did not get.

    `blob_state` maps each raw-sourced field the event touched (container row
    or decoded member) to whether its blob was built. Counted per event and
    field, so members arriving without their blob count once.
    """
    unbuilt = sum(1 for built in blob_state.values() if not built)
    if unbuilt:
        tally.bump("raw_blobs_unavailable", unbuilt)


def _split_rpc_field(field_name: str):
    """Split "Rpc.Param" into (rpc_name, param_name); param_name is None when
    the field_name is the function itself (no dot)."""
    name, dot, param = field_name.partition('.')
    return name, (param if dot else None)


def _to_package_path(class_path: str) -> str:
    """Drop the `.ClassName_C` suffix, leaving the package path the
    reference's replication_class_path has: valplay's ability_usage and
    ability_detail key on `path.split("/")[-1]`."""
    slash = class_path.rfind("/")
    tail = class_path[slash + 1:]
    if "." not in tail:
        return class_path
    return class_path[: slash + 1] + tail.split(".", 1)[0]


# ---------------------------------------------------------------------------
# RPC parameter normalization
# ---------------------------------------------------------------------------
#: Damage RPC booleans the reference format spells without vrfkit's 'b' prefix.
_DAMAGE_PARAM_RENAMES = {
    "bDamageKilledTarget": "DamageKilledTarget",
    "bAliveAfterDamage": "AliveAfterDamage",
    "bIsWallPenetration": "IsWallPenetration",
    "bEquippableUsedZoomed": "EquippableUsedZoomed",
    "bEquippableUsedInFocusMode": "EquippableUsedInFocusMode",
}


def _normalize_rpc_param(rpc_name: str, param: str, value, is_raw: bool,
                         tally=None) -> dict | None:
    """Rename/reshape one RPC parameter to the reference format; None drops it.

    Only the damage RPCs are reshaped. Members of `LifeChangeEvents[]` and
    `LifeChangeBySection[]` are dropped: the parent blob carries them, and a
    member arriving without it is counted by `_build_rpc_events`.
    """
    if "[" in param and param.split("[", 1)[0] in (
        "LifeChangeEvents",
        "LifeChangeBySection",
    ):
        return None
    if rpc_name not in DAMAGE_RPC_NAMES:
        return {param: value}

    out_name = _DAMAGE_PARAM_RENAMES.get(param, param)
    if param == "RegionalDamage" and not is_raw:
        value = REGIONAL_DAMAGE_MAP.get(value, f"regional_damage__unknown_{value}")
    elif param in DAMAGE_VECTOR_PARAMS:
        # Quantized vectors as "(x,y,z)"; the reference emits {x, y, z}. An
        # unparseable one stays as it came: visibly absent beats a zero vector.
        parsed = _parse_vector_or_none(value)
        if parsed is not None:
            value = parsed
    elif param == "EquippableUsed":
        # The reference's ValorantEquippable shape, Name/ClassPath null (a
        # weapon instance has no NetGuidCache path; valplay resolves it from
        # actor_spawned); an undecoded value passes its bits through.
        if isinstance(value, int) and not is_raw:
            value = {
                "NetGuid": value,
                "Name": None,
                "ClassPath": None,
                "Category": "unknown",
            }
    elif param == "LifeChangeEvents" or param in DEATH_MONTAGE_BLOB_PARAMS:
        # The reference's labelled blob; the RPC loop builds it from raw_bits
        # typed or not, so `is_raw` means "the blob exists". Exact names: a
        # prefix match would also catch the 1-bit ...IsQueued bool.
        if is_raw and isinstance(value, dict):
            value["TypeName"] = param
    elif param == "DamagedBone" and is_raw:
        # Typed on all 632,906 _Point rows of the 1,018-export corpus. A raw
        # one is null and counted, never rendered from the bytes: valplay's
        # `_bone_region` files None under "other" but raises on a dict.
        _bump(tally, "damaged_bone_undecoded")
        value = None
    return {out_name: value}


# ---------------------------------------------------------------------------
# Conversion phases
#
# `events.sort` at the end is stable, so events tying on (packet_id, time_ms)
# keep the order the phases appended them in: keep the phase order (actors,
# properties, RPCs, server timeline) and the append order inside each phase.
# ---------------------------------------------------------------------------
#: The bundle manifest's shape version. valplay's gate is an equality check
#: (scripts/parse_replays.py ADAPTER_SCHEMA_VERSION), so a bump refuses every
#: existing bundle until valplay re-pins VRFKIT_REF: bump only when a consumer
#: that ignores the change would MISREAD a bundle. A new event type or key is
#: not that (valplay matches exact types and ignores unknown keys), nor is a
#: removed `losses` key (valplay forwards `losses` without reading its keys).
BUNDLE_SCHEMA_VERSION = 2


def _upstream_row_check(declared, observed) -> dict:
    """One declared-vs-observed row count. `agrees` is `None`, not `True`,
    when nothing was declared: a manifest without accounting must not
    certify itself."""
    return {
        "declared": declared,
        "observed": observed,
        "agrees": None if declared is None else declared == observed,
    }


def _public_level_names(value):
    """The level roots (the map URL) through a two-field allowlist: the
    neighbouring ``game_specific_data`` header can hold account subjects and
    loadouts, so nothing else of the header crosses."""
    if not isinstance(value, list):
        return None
    levels = []
    for row in value:
        if not isinstance(row, dict):
            continue
        name = row.get("name")
        time_ms = row.get("time_ms")
        if not isinstance(name, str) or not name:
            continue
        if isinstance(time_ms, bool) or not isinstance(time_ms, int):
            continue
        if not 0 <= time_ms <= 0xFFFF_FFFF:
            continue
        levels.append({"name": name, "time_ms": time_ms})
    return levels


def _write_manifest(manifest: dict, output_dir: Path, adapter: dict):
    """Write the bundle manifest (docs/USAGE.md, "What the bundle manifest
    carries"): `quality` and `net_field_export_groups` VERBATIM and null when
    absent, never a plausible zero; the public `level_names_and_times`; and
    `adapter`, this process's own measurements, kept apart from upstream ones.
    """
    quality = manifest.get("quality")
    groups = manifest.get("net_field_export_groups")
    levels = _public_level_names(manifest.get("level_names_and_times"))
    out_manifest = {
        "replay_version": manifest.get("replay_version", "unknown"),
        # No default: compute_metrics.py reads it without one, so a 0 would
        # pass for a real match length.
        "duration_ms": manifest.get("duration_ms"),
        "replay_build": manifest.get("replay_build", ""),
        "replay_changelist": manifest.get("replay_changelist", 0),
        "source_file": manifest.get("source_file", ""),
        "source_size_bytes": manifest.get("source_size_bytes"),
        "converter": "vrfkit/tools/to_valplay_bundle.py",
        "bundle_schema_version": BUNDLE_SCHEMA_VERSION,
        "quality": quality if isinstance(quality, dict) else None,
        "net_field_export_groups": groups if isinstance(groups, list) else None,
        "level_names_and_times": levels,
        "adapter": adapter,
    }
    # LF on every platform, as the extractors' receipts are.
    (output_dir / "manifest.json").write_text(
        json.dumps(out_manifest, indent=2), encoding='utf-8', newline='\n'
    )


def _dict_column_to_pylist(column):
    """A dictionary-encoded column as a list SHARING its string objects
    (`to_pylist` for any other column): one str per row costs a hundred-odd
    MB of duplicates on 02d4d478 and is 2.3x slower."""
    arr = column.combine_chunks()
    if not pa.types.is_dictionary(arr.type):
        return column.to_pylist()
    values = arr.dictionary.to_pylist()
    return [values[i] if i is not None else None
            for i in arr.indices.to_pylist()]


def _numeric_column_to_pylist(column):
    """A numeric column with NO nulls as a Python list, via numpy (~14x
    faster than `to_pylist`); numpy would widen a nullable integer column to
    float64, see `_nullable_numeric_to_pylist`."""
    return column.to_numpy(zero_copy_only=False).tolist()


def _nullable_numeric_to_pylist(column):
    """A nullable numeric column as a list of (value | None), equal to
    `to_pylist` and ~3x faster: nulls are filled with a type-matched sentinel
    so `to_numpy` keeps the type, then the validity mask puts None back."""
    arr = column.combine_chunks()
    n = len(arr)
    if n == 0:
        return []
    mask = arr.is_valid().to_numpy(zero_copy_only=False).tolist()
    t = arr.type
    if pa.types.is_boolean(t):
        fill = False
    elif pa.types.is_floating(t):
        fill = 0.0
    else:
        fill = 0
    values = pc.fill_null(arr, fill).to_numpy(zero_copy_only=False).tolist()
    return [v if m else None for v, m in zip(values, mask)]


class _FieldColumns(NamedTuple):
    """fields.parquet as one Python list per column (row iteration is slow)."""

    n_rows: int
    time_ms: list
    packet_id: list
    actor: list
    obj: list
    channel: list
    group_path: list
    handle: list
    field_name: list
    bit_count: list
    raw_bits: list
    value_i64: list
    value_f64: list
    value_bool: list
    value_str: list


def _load_field_columns(fields_path: Path, verbose: bool) -> _FieldColumns:
    """Read fields.parquet and extract every column we consume."""
    t0 = time.time()
    if verbose:
        print("Loading fields.parquet...")
    table = read_parquet(fields_path)
    n_rows = len(table)
    if verbose:
        print(f"  {n_rows:,} rows loaded in {time.time()-t0:.1f}s")

    t0 = time.time()
    if verbose:
        print("Extracting columns...")

    cols = _FieldColumns(
        n_rows=n_rows,
        time_ms=_numeric_column_to_pylist(table.column('time_ms')),
        packet_id=_numeric_column_to_pylist(table.column('packet_id')),
        actor=_numeric_column_to_pylist(table.column('actor_net_guid')),
        # Null for actor blocks, where the reference repeats the actor guid.
        obj=_nullable_numeric_to_pylist(table.column('object_net_guid')),
        channel=_numeric_column_to_pylist(table.column('channel_index')),
        group_path=_dict_column_to_pylist(table.column('group_path')),
        handle=_numeric_column_to_pylist(table.column('handle')),
        field_name=_dict_column_to_pylist(table.column('field_name')),
        bit_count=_numeric_column_to_pylist(table.column('bit_count')),
        raw_bits=table.column('raw_bits').to_pylist(),
        value_i64=_nullable_numeric_to_pylist(table.column('value_i64')),
        value_f64=_nullable_numeric_to_pylist(table.column('value_f64')),
        value_bool=_nullable_numeric_to_pylist(table.column('value_bool')),
        value_str=_dict_column_to_pylist(table.column('value_str')),
    )

    if verbose:
        print(f"  Columns extracted in {time.time()-t0:.1f}s")
    return cols


def _group_rows(cols: _FieldColumns):
    """Group field rows into property and RPC (group_path contains
    '_ClassNetCache') groups of row indices, in one pass: the dicts' insertion
    order decides how events tying on (packet_id, time_ms) are written."""
    prop_groups = defaultdict(list)
    rpc_groups = defaultdict(list)
    # Counted, to compare with quality.net.unresolved_rpc_payloads_preserved.
    unresolved_cnc_rows = 0
    for i, (pid, actor, obj, gp, handle, fn) in enumerate(zip(
            cols.packet_id, cols.actor, cols.obj, cols.group_path, cols.handle,
            cols.field_name)):
        if fn == UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME:
            unresolved_cnc_rows += 1
        elif CLASS_NET_CACHE_SUFFIX in gp:
            rpc_groups[(pid, actor, obj, gp, handle)].append(i)
        else:
            # Keyed by subobject too: a character's ItemSlot subobjects
            # would otherwise merge into one slot.
            prop_groups[(pid, actor, obj, gp)].append(i)
    return prop_groups, rpc_groups, unresolved_cnc_rows


#: `actors.event` -> the bundle event type (`open` is actor_spawned). `dormant`
#: suspends replication of a live actor, NOT a despawn: valplay pairs
#: spawn/close into a lifetime, so a settled wall published as a close would
#: read as destroyed. An unmapped value becomes `actor_lifecycle_unknown`.
_ACTOR_EVENT_TYPES = {
    "close": "actor_closed",
    "dormant": "actor_dormant",
}


# Event-chunk payload words per group. Mirrors vrf-container KNOWN_EVENT_GROUPS;
# adapter_contract.rs pins all three dicts. An unknown group crosses wordless.
_SERVER_TIMELINE_WORD_COUNTS = {
    "characterDeath": 2,
    "characterUltimateUsed": 1,
    "roundStarted": 1,
    "switchTeams": 1,
    "spikePlanted": 0,
    "spikeDefused": 0,
    "spikeExploded": 0,
}

# Public enum constants rechecked before `payload_name` may cross: tag, name
# and seconds cross only as a whole agreeing tuple.
_SERVER_TIMELINE_PAYLOAD_TAGS = {
    "characterDeath": 8,
    "characterUltimateUsed": 11,
    "roundStarted": 2,
    "switchTeams": 3,
    "spikePlanted": 4,
    "spikeDefused": 5,
    "spikeExploded": 6,
}
_SERVER_TIMELINE_PAYLOAD_NAMES = {
    "characterDeath": "EReplayEventGroup::CharacterDeath",
    "characterUltimateUsed": "EReplayEventGroup::CharacterUltimateUsed",
    "roundStarted": "EReplayEventGroup::RoundStart",
    "switchTeams": "EReplayEventGroup::SwitchTeams",
    "spikePlanted": "EReplayEventGroup::SpikePlanted",
    "spikeDefused": "EReplayEventGroup::SpikeDefused",
    "spikeExploded": "EReplayEventGroup::SpikeExploded",
}
EVENT_PAYLOAD_TIME_TOLERANCE_MS = 1.001


def _packets_at_or_before(cols: "_FieldColumns", times: list) -> list:
    """Order packet-less Event chunks among packet-ordered rows: each time
    maps to the largest packet id of a field row at or before it (0 before
    the first), a prefix maximum that stays monotone when a bad frame gives a
    later packet an earlier time. Ordering only, never published."""
    row_times = numpy.asarray(cols.time_ms, dtype=numpy.int64)
    order = numpy.argsort(row_times, kind="stable")
    packets = numpy.asarray(cols.packet_id, dtype=numpy.int64)[order]
    prefix = numpy.concatenate(([0], numpy.maximum.accumulate(packets)))
    return prefix[numpy.searchsorted(row_times[order], times, side="right")].tolist()


def _build_server_timeline_events(export_dir: Path, cols: "_FieldColumns",
                                  verbose: bool) -> tuple[list, int]:
    """Publish the Event-chunk timeline through a privacy-safe allowlist.

    events.parquet also holds a replay-scoped id, free-form metadata and raw
    payload bytes, any of which may identify an account or match, so none
    crosses. The payload FString crosses only when it equals the group's
    public enum constant and the tag/time tuple matches the measured layout.
    Returns ``(events, rows_read)``.
    """
    table = read_parquet(export_dir / "events.parquet")
    groups = _dict_column_to_pylist(table.column("group"))
    time1 = _numeric_column_to_pylist(table.column("time1"))
    time2 = _numeric_column_to_pylist(table.column("time2"))
    word0 = _nullable_numeric_to_pylist(table.column("word0"))
    word1 = _nullable_numeric_to_pylist(table.column("word1"))
    payload_tag = _nullable_numeric_to_pylist(table.column("payload_tag"))
    payload_name = table.column("payload_name").to_pylist()
    payload_seconds = _nullable_numeric_to_pylist(table.column("payload_seconds"))
    packet_keys = _packets_at_or_before(cols, time1)
    events = []

    for (group, first_time, second_time, first_word, second_word, tag,
         payload_enum_name, seconds, packet_key) in zip(
        groups, time1, time2, word0, word1, payload_tag, payload_name,
        payload_seconds, packet_keys
    ):
        event = {
            "type": "server_timeline_event",
            "time_ms": first_time,
            "time2_ms": second_time,
            "event_group": group,
        }
        word_count = _SERVER_TIMELINE_WORD_COUNTS.get(group, 0)
        if word_count >= 1 and first_word is not None:
            event["word0"] = first_word
        if word_count >= 2 and second_word is not None:
            event["word1"] = second_word
        expected_tag = _SERVER_TIMELINE_PAYLOAD_TAGS.get(group)
        expected_name = _SERVER_TIMELINE_PAYLOAD_NAMES.get(group)
        seconds_matches = (
            seconds is not None
            and math.isfinite(seconds)
            and abs(seconds * 1000.0 - first_time)
            <= EVENT_PAYLOAD_TIME_TOLERANCE_MS
        )
        if (
            expected_tag is not None
            and tag == expected_tag
            and payload_enum_name == expected_name
            and seconds_matches
        ):
            event["payload_tag"] = tag
            event["payload_name"] = payload_enum_name
            event["payload_seconds"] = _f32_shortest(seconds)
        events.append((packet_key, first_time, event))

    if verbose:
        print(f"  {len(events):,} server timeline events from events.parquet")
    return events, len(table)


def _spawn_axes(row: dict, axes: tuple):
    """Float32 spawn values written shortest; null when all are null (a
    static actor, which {0,0,0} would merge with real origin spawns)."""
    values = [row["spawn_" + axis] for axis in axes]
    if all(value is None for value in values):
        return None
    return {axis: 0 if value is None else _f32_shortest(value)
            for axis, value in zip(axes, values)}


def _build_actor_events(export_dir: Path, verbose: bool, tally: "_Tally"):
    """actor_spawned / actor_closed / actor_dormant events, and `guid_class`
    (actor GUID -> spawn class path) for the shots' weapon identity."""
    events = []
    guid_class = {}
    counts = Counter(dict.fromkeys(_ACTOR_EVENT_TYPES.values(), 0))
    rows = read_parquet(export_dir / "actors.parquet").to_pylist()
    for row in rows:
        raw_event = row["event"]
        event_type = ("actor_spawned" if raw_event == "open"
                      else _ACTOR_EVENT_TYPES.get(raw_event))
        if event_type is None:
            # A visible unknown carrying the raw value, never a plausible close.
            tally.bump("unknown_actor_lifecycle_events")
            event_type = "actor_lifecycle_unknown"
        event = {"type": event_type, "time_ms": row["time_ms"],
                 "actor_net_guid": row["actor_net_guid"],
                 "channel": row["channel_index"]}
        if raw_event != "open":
            counts[event_type] += 1
            event["actor_event"] = raw_event  # the wire's value, to audit the mapping
        else:
            class_path = row["class_path"]
            if class_path:
                # First open wins: a reused GUID keeps its first life's class.
                guid_class.setdefault(row["actor_net_guid"], class_path)
            # The event carries the reference's package path, guid_class the
            # object path the weapon lookup matches.
            event["replication_class_path"] = (
                _to_package_path(class_path) if class_path else None)
            event["archetype_path"] = row["archetype_path"]
            event["location"] = _spawn_axes(row, ("x", "y", "z"))
            event["rotation"] = _spawn_axes(row, ("pitch", "yaw", "roll"))
        events.append((row["packet_id"], row["time_ms"], event))

    if verbose:
        print(f"  {len(rows):,} actor lifecycle events from actors.parquet")
        for name, count in sorted(counts.items()):
            print(f"    {name}: {count:,}")
    return events, guid_class


def _build_property_events(cols: _FieldColumns, prop_groups: dict, tally: _Tally,
                           combat_report_names=None):
    """export_group_received events from the replicated-property groups. An
    unnamed row is dropped and counted; its group still emits an event."""
    col_time = cols.time_ms
    col_fn = cols.field_name
    col_handle = cols.handle
    col_bits = cols.bit_count
    col_raw = cols.raw_bits
    col_i64 = cols.value_i64
    col_f64 = cols.value_f64
    col_bool = cols.value_bool
    col_str = cols.value_str

    events = []
    for (pid, actor, obj, gp), row_indices in prop_groups.items():
        ms = col_time[row_indices[0]]

        # A container row ("Rounds", "Sel[1].Att") arrives beside its decoded
        # elements, before or after them; every prefix ending at a '[' names
        # one, and the second pass skips it at any depth.
        indexed_names = set()
        for ri in row_indices:
            fn = col_fn[ri]
            if fn and '[' in fn:
                indexed_names.update(_container_names(fn))

        payload = {}
        blob_state = {}  # {RAW_BLOB_PREFERRED name: blob built?}
        for ri in row_indices:
            fn = col_fn[ri]
            if fn is None:
                tally.bump("unnamed_property_rows")
                continue
            value, is_raw = _get_value(
                col_i64[ri], col_f64[ri], col_bool[ri], col_str[ri],
                col_raw[ri], col_bits[ri], tally
            )
            type_name = RAW_BLOB_PREFERRED.get(fn)
            if type_name is not None:
                # A second row of this name replaces the first one's blob or
                # stand-in below, or is dropped behind its blob.
                if fn in payload:
                    tally.bump("property_key_collisions")
                raw = col_raw[ri]
                if raw is not None:
                    blob = _raw_blob(raw, col_bits[ri])
                    blob["TypeName"] = type_name
                    payload[fn] = blob
                    blob_state[fn] = True
                else:
                    # Counted when the event is complete; a typed value goes
                    # in the blob's place (skipped visibly) unless one was built.
                    blob_state.setdefault(fn, False)
                    if value is not None and not blob_state[fn]:
                        payload[fn] = value
                continue
            # A container is skipped raw or typed: its elements carry the data.
            if fn in indexed_names or (value is None and not is_raw):
                continue

            # See "Combat report leaf labels".
            fn = _combat_report_leaf_name(gp, fn, col_handle[ri], combat_report_names)

            is_bool = col_bool[ri] is not None
            fn = _normalize_prop_field_name(fn, is_bool)

            if fn in VECTOR_PROPERTIES and isinstance(value, str):
                parsed_vec = _parse_vector_or_none(value)
                if parsed_vec is not None:
                    value = parsed_vec
            elif fn in JSON_OBJECT_PROPERTIES and isinstance(value, str):
                # A value that will not parse is a parser bug: it stops the run.
                value = json.loads(value)

            parts = _parse_field_path(fn, tally)
            if len(parts) == 1 and parts[0][1] is None:
                bare_name = parts[0][0]
                # Struct members and static-array elements share a name only
                # `handle` tells apart, so a repeat is a different property
                # (most of 02d4d478's 28,845 collisions); keyed by name anyway,
                # as valplay consumes names.
                if bare_name in payload:
                    _count_overwrite(tally, payload[bare_name])
                payload[bare_name] = value
            elif parts[0][0] in RAW_BLOB_PREFERRED:
                # A decoded member of a raw-blob field is not published (the
                # blob carries it), so one whose event built no blob is a loss.
                blob_state.setdefault(parts[0][0], False)
            else:
                _set_nested(payload, parts, value, tally)

        _count_unbuilt_blobs(blob_state, tally)
        _drop_padding_elements(payload)

        # Emitted even when empty: some events are existence signals.
        event = {
            "type": "export_group_received",
            "time_ms": ms,
            "export_group_path": gp,
            "actor_net_guid": actor,
            "object_net_guid": obj if obj is not None else actor,
            "payload": payload,
        }
        events.append((pid, ms, event))

    return events


def _build_rpc_events(cols: _FieldColumns, rpc_groups: dict,
                      shot_ctx: _ShotContext, tally: _Tally):
    """rpc_received events, the shot first for a shot RPC:
    ``(events, shot_count, resolved_weapon_count)``.

    Counted, not fixed: a function invoked TWICE on one object in one packet
    is one group whose second call overwrites the first's parameters (nothing
    marks an invocation's end), and a group of only unnamed rows cannot name
    its function, so it is dropped whole.
    """
    col_time = cols.time_ms
    col_fn = cols.field_name
    col_bits = cols.bit_count
    col_raw = cols.raw_bits
    col_i64 = cols.value_i64
    col_f64 = cols.value_f64
    col_bool = cols.value_bool
    col_str = cols.value_str

    events = []
    shot_count = 0
    resolved_weapon_count = 0

    for (pid, actor, obj, gp, handle), row_indices in rpc_groups.items():
        ms = col_time[row_indices[0]]

        rpc_name = None
        payload = {}
        effect_blobs = {}  # shot array parameter -> _EffectBlob
        blob_state = {}  # {raw-sourced parameter: blob built?}
        for ri in row_indices:
            fn = col_fn[ri]
            if fn is None:
                tally.bump("unnamed_rpc_rows")
                continue
            name, param = _split_rpc_field(fn)
            if rpc_name is None:
                rpc_name = name
            value, is_raw = _get_value(
                col_i64[ri], col_f64[ri], col_bool[ri], col_str[ri],
                col_raw[ri], col_bits[ri], tally
            )
            if param is None:
                # The function row itself: a zero-parameter RPC, or the whole
                # unbound parameter block as bits (608 rows on 02d4d478), kept
                # under the function's name (vrfkit-only), so "payload: null"
                # means only "no parameters".
                if value is not None:
                    if name in payload:
                        tally.bump("rpc_param_collisions")
                    payload[name] = value
                continue
            # The RAW_BLOB_PREFERRED rule for `_RAW_SOURCED_RPC_PARAMS`;
            # `_get_value` ran above, so multi_typed_rows still sees the row.
            sourced = _RAW_SOURCED_RPC_PARAMS.get(name)
            if sourced is not None:
                if param in sourced:
                    raw = col_raw[ri]
                    if raw is not None:
                        bits = col_bits[ri]
                        if name == "ReplayPlayContinuousEffectAtLocation":
                            effect_blobs[param] = _EffectBlob(bytes(raw), bits)
                        value = _raw_blob(raw, bits)
                        is_raw = True
                        blob_state[param] = True
                    else:
                        # Counted when the invocation is complete.
                        blob_state.setdefault(param, False)
                elif "[" in param:
                    member_of = param.split("[", 1)[0]
                    if member_of in sourced:
                        # A decoded member, dropped by `_normalize_rpc_param`
                        # because the blob carries it: the blob is expected.
                        blob_state.setdefault(member_of, False)
            if value is None and not is_raw:
                continue
            param_out = _normalize_rpc_param(rpc_name, param, value, is_raw, tally)
            if param_out is not None:
                for k, v in param_out.items():
                    if k in payload:
                        tally.bump("rpc_param_collisions")
                    payload[k] = v

        _count_unbuilt_blobs(blob_state, tally)

        if rpc_name is None:
            tally.bump("unnamed_rpc_invocations")
            continue

        if rpc_name == "ReplayPlayContinuousEffectAtLocation":
            # No blob guard: an effect with only scalar params (7 of 02d4d478's
            # 2,647) is still a shot, as in the reference.
            first = row_indices[0]
            shot_event = _build_shot_event(
                shot_ctx, ms, pid, actor,
                obj,
                cols.channel[first], payload,
                _EffectBlobs(*map(effect_blobs.get, (
                    "FloatValues", "ObjectValues", "VectorValues"))),
                tally=tally,
            )
            events.append((pid, ms, shot_event))
            shot_count += 1
            if shot_event["shot"]["equippable"] is not None:
                resolved_weapon_count += 1

        event = {
            "type": "rpc_received",
            "time_ms": ms,
            "function_name": rpc_name,
            "actor_net_guid": actor,
            "object_net_guid": obj,
            "payload": payload if payload else None,
        }
        events.append((pid, ms, event))

    return events, shot_count, resolved_weapon_count


def _write_events(events: list, output_dir: Path, verbose: bool) -> tuple[int, int]:
    """Stable-sort on (packet_id, time_ms) and write events.ndjson:
    ``(events_written, time_ms_regressions)``.

    packet_id is the wire's only total order; ties keep the phase order
    ("Conversion phases"). time_ms is not monotonic (a non-finite frame gives
    0), so a regression is counted, never repaired away from packet order.
    """
    t0 = time.time()
    if verbose:
        print("Sorting and writing events.ndjson...")
    events.sort(key=lambda x: (x[0], x[1]))

    events_written = 0
    regressions = 0
    previous_ms = None
    encode = _JSON.encode
    with open(output_dir / "events.ndjson", 'w', encoding='utf-8') as f:
        write = f.write
        for _, time_ms, evt in events:
            if previous_ms is not None and time_ms < previous_ms:
                regressions += 1
            previous_ms = time_ms
            write(encode(evt))
            write('\n')
            events_written += 1

    if verbose:
        print(f"  {events_written:,} events written in {time.time()-t0:.1f}s")
        if regressions:
            print(f"  {regressions:,} time_ms regressions in written order")
    return events_written, regressions


#: Every movement.parquet column `_write_movement` reads. The export declares
#: all of them non-null (vrf-export/src/tables/movement.rs).
_MOVEMENT_COLUMNS = ("time_ms", "packet_id", "character_net_guid",
                     "pos_x", "pos_y", "pos_z", "vel_x", "vel_y", "vel_z",
                     "yaw", "pitch")


def _string_bytes(arr) -> memoryview:
    """A pa.string() array's values as one span of bytes: the data buffer
    between its first and last offsets, not the whole buffer (a slice starts
    earlier, and Arrow may allocate more than it fills)."""
    if arr.type != pa.string():
        raise RuntimeError(f"expected a string array, got {arr.type}")
    offsets = numpy.frombuffer(arr.buffers()[1], dtype=numpy.int32,
                               count=len(arr) + 1, offset=arr.offset * 4)
    return memoryview(arr.buffers()[2])[offsets[0]:offsets[-1]]


def _write_movement(movement_path: Path, output_dir: Path, verbose: bool) -> tuple:
    """Write movement.ndjson, keeping the last sub-move per (packet, character):
    ``(rows_read, rows_written, non_finite_rows)``.

    Only rows_read compares with the export's declared count. non_finite_rows
    counts WRITTEN rows spelling Infinity/NaN: a strict parser (valplay's
    orjson) rejects the line and recounts fewer rows than were written.
    """
    t0 = time.time()
    if verbose:
        print("Converting movement.parquet...")
    mv_table = read_parquet(movement_path, columns=list(_MOVEMENT_COLUMNS))
    n_mv = len(mv_table)

    # Declared non-null, and load-bearing: `to_numpy` turns a uint32 column
    # with a null into float64 NaN (`1000.0`-style times, `nan`), and a null
    # join input makes a NULL line that adds no bytes yet counts as written.
    for name in _MOVEMENT_COLUMNS:
        nulls = mv_table.column(name).null_count
        if nulls:
            raise ValueError(f"movement.parquet column {name!r} carries "
                             f"{nulls:,} nulls; the export declares it non-null")
    mv = {name: mv_table.column(name).to_numpy(zero_copy_only=False)
          for name in _MOVEMENT_COLUMNS}

    # The last sub-move per (packet, character), as the reference keeps: a
    # packet's move chain shares one time_ms, and valplay's posture.py counts
    # a dt=0 leg short (distance_m 3.1-5.2 m low). Keyed on the PACKET, never
    # the millisecond, which also merges two packets landing in one ms. The
    # first index of each (packet << 32 | char) key in the reversed array is
    # its last in the original; both are uint32, so the pack cannot collide.
    key64 = ((mv["packet_id"].astype(numpy.uint64) << numpy.uint64(32))
             | mv["character_net_guid"].astype(numpy.uint64))
    first_in_rev = numpy.unique(key64[::-1], return_index=True)[1]
    keep = numpy.sort((n_mv - 1) - first_in_rev)
    del key64, first_in_rev
    movement_collapsed = n_mv - len(keep)

    # Positions and velocities are float32 written shortest (349.99, not
    # 349.989990234375); yaw and pitch widened, as the reference writes them.
    # One column at a time: all eight at once cost 35-83 MB more peak.
    non_finite = numpy.zeros(len(keep), dtype=bool)
    float_texts = []
    for name in _MOVEMENT_COLUMNS[3:]:
        kept = mv.pop(name)[keep]
        non_finite |= ~numpy.isfinite(kept)
        float_texts.append(_json_scalar_column(kept, shorten=name[:4] in ("pos_", "vel_")))
        del kept
    non_finite_rows = int(numpy.count_nonzero(non_finite))
    movement_written = len(keep)
    # Nearly all distinct, so not deduplicated; Arrow's cast writes the digits
    # the JSON encoder would.
    int_slots = (pa.array(mv["time_ms"][keep]), pa.array(mv["character_net_guid"][keep]))
    # Dropped now, to stay out of the write loop's peak.
    mv.clear()
    del mv_table, keep, non_finite

    # Assembled in Arrow one block at a time, byte for byte the per-row join
    # (MovementLineAssemblyTests), in binary mode with os.linesep endings.
    fragments = [pa.scalar(text, type=pa.string()) for text in
                 _MOVEMENT_LINE.replace("\n", os.linesep).split("%s")]
    with open(output_dir / "movement.ndjson", "wb") as f:
        for start in range(0, movement_written, _MOVEMENT_BLOCK_ROWS):
            length = min(_MOVEMENT_BLOCK_ROWS, movement_written - start)
            slots = [pc.cast(values.slice(start, length), pa.string())
                     for values in int_slots]
            slots += [texts.take(inverse.slice(start, length))
                      for texts, inverse in float_texts]
            pieces = [fragments[0]]
            for slot, fragment in zip(slots, fragments[1:], strict=True):
                pieces += (slot, fragment)
            f.write(_string_bytes(pc.binary_join_element_wise(*pieces, "")))

    if verbose and movement_collapsed:
        print(f"  {movement_collapsed:,} intra-packet sub-moves collapsed "
              f"(kept in movement.parquet)")

    if verbose:
        print(f"  {movement_written:,} movement rows written in {time.time()-t0:.1f}s")
    return n_mv, movement_written, non_finite_rows


# ---------------------------------------------------------------------------
# Main conversion
# ---------------------------------------------------------------------------
def _convert_into(export_dir: Path, output_dir: Path, *, verbose: bool = False):
    """Read vrfkit Parquet export and write valplay-compatible bundle."""
    tally = _Tally()
    manifest = json.loads((export_dir / "manifest.json").read_text(encoding='utf-8'))
    upstream_quality = manifest.get("quality")
    if not isinstance(upstream_quality, dict):
        # Published as `"quality": null` ("nobody counted"), never a zero.
        upstream_quality = None

    cols = _load_field_columns(export_dir / "fields.parquet", verbose)

    t0 = time.time()
    if verbose:
        print("Grouping rows into events...")
    prop_groups, rpc_groups, unresolved_cnc_rows = _group_rows(cols)
    if verbose:
        print(f"  {len(prop_groups):,} property events, {len(rpc_groups):,} RPC invocations")
        print(f"  Grouped in {time.time()-t0:.1f}s")

    t0 = time.time()
    if verbose:
        print("Building event records...")

    # The phases run in "Conversion phases" order: actors, properties, RPCs,
    # timeline.
    guid_outer, guid_path, net_guid_rows_read = _load_net_guids(export_dir)

    events, guid_class = _build_actor_events(export_dir, verbose, tally)

    events += _build_property_events(
        cols, prop_groups, tally, _combat_report_declared_names(manifest))

    shot_ctx = _ShotContext(_build_tag_table(manifest), guid_outer, guid_path, guid_class)
    rpc_events, shot_count, resolved_weapon_count = _build_rpc_events(
        cols, rpc_groups, shot_ctx, tally
    )
    events += rpc_events

    timeline_events, server_timeline_rows_read = _build_server_timeline_events(
        export_dir, cols, verbose
    )
    events += timeline_events

    # An empty tag table costs only a replay with shots.
    if shot_count and not shot_ctx.tag_table:
        tally.bump("empty_gameplay_tag_table")

    if verbose:
        print(f"  {len(events):,} total events built in {time.time()-t0:.1f}s")
        print(f"  {shot_count:,} valorant_shot_received events")
        pct = 100 * resolved_weapon_count / shot_count if shot_count else 0
        print(f"  {resolved_weapon_count:,} with a resolved weapon ({pct:.2f}%)")

    events_written, time_ms_regressions = _write_events(events, output_dir, verbose)
    if time_ms_regressions:
        # Not a loss (every event is written), but it breaks the consumer's
        # (time_ms, line index) tie-break, so it is surfaced with the losses.
        tally.bump("events_time_ms_regressions", time_ms_regressions)

    movement_rows_read, movement_written, non_finite_movement_rows = _write_movement(
        export_dir / "movement.parquet", output_dir, verbose
    )
    # Not a loss either, but a strict parser rejects the line.
    tally.bump("non_finite_movement_rows", non_finite_movement_rows)

    # Declared vs read back, only for exact table heights: `quality.net.fields`
    # is not one (fields.parquet adds leaves), and a permanent false alarm is
    # how a real one stops being read.
    declared = upstream_quality or {}
    upstream_row_counts = {
        "movement_rows": _upstream_row_check(
            declared.get("movement_rows"), movement_rows_read
        ),
        "net_guid_rows": _upstream_row_check(
            declared.get("net_guid_rows"), net_guid_rows_read
        ),
        "event_rows": _upstream_row_check(
            declared.get("event_rows"), server_timeline_rows_read
        ),
    }
    disagreements = sum(
        1 for check in upstream_row_counts.values() if check["agrees"] is False
    )
    if disagreements:
        # Which side is wrong is not knowable here: counted, never repaired.
        tally.bump("upstream_row_count_disagreement", disagreements)

    adapter_accounting = {
        "schema_version": BUNDLE_SCHEMA_VERSION,
        "events_written": events_written,
        "events_time_ms_regressions": time_ms_regressions,
        "movement_rows_read": movement_rows_read,
        "movement_rows_written": movement_written,
        "net_guid_rows_read": net_guid_rows_read,
        "server_timeline_rows_read": server_timeline_rows_read,
        "server_timeline_events_written": len(timeline_events),
        "field_rows_read": cols.n_rows,
        "field_rows_unresolved_class_net_cache": unresolved_cnc_rows,
        "upstream_row_counts": upstream_row_counts,
        # Every counter, zeros included.
        "losses": dict(tally),
    }
    _write_manifest(manifest, output_dir, adapter_accounting)

    return {
        "events_written": events_written,
        "movement_written": movement_written,
        "tally": tally,
    }


def _print_summary(output_dir: Path, result: dict) -> None:
    """The console summary, from `convert` after the publish commit, so it
    names the final path; "complete" only when nothing was lost."""
    tally = result["tally"]
    if tally.total:
        print(f"\nConversion finished WITH LOSSES: {output_dir}")
    else:
        print(f"\nConversion complete: {output_dir}")
    print(f"  events.ndjson:   {result['events_written']:,} lines")
    print(f"  movement.ndjson: {result['movement_written']:,} lines")
    print("  manifest.json:   written")
    for line in tally.lines():
        print(line)


def _validate_separate_trees(export_dir: Path, output_dir: Path) -> tuple[Path, Path]:
    """Reject equal, nested, and aliased input/output trees before any write."""
    source = export_dir.resolve()
    destination = output_dir.resolve()
    # is_relative_to holds for equal paths too.
    if source.is_relative_to(destination) or destination.is_relative_to(source):
        raise ValueError(
            f"export and output directories must not overlap: {source} / {destination}"
        )
    return source, destination


def _publish_bundle(staging: Path, output_dir: Path) -> None:
    """Publish a verified staged bundle, rolling back an existing bundle."""
    parent = output_dir.parent.resolve()
    require_descendant(staging, parent)
    require_descendant(output_dir, parent)
    backup = Path(tempfile.mkdtemp(prefix=f".{output_dir.name}.backup.", dir=parent))
    # mkdtemp reserves a collision-free name; os.replace requires it absent.
    backup.rmdir()
    moved_old = False
    try:
        if output_dir.exists():
            os.replace(output_dir, backup)
            moved_old = True
        os.replace(staging, output_dir)
    except BaseException:
        if moved_old and backup.exists() and not output_dir.exists():
            os.replace(backup, output_dir)
        raise
    if backup.exists():
        try:
            remove_tree(backup, parent)
        except OSError as exc:
            # Past the commit point, failure would be false and invite a retry
            # over the new bundle; the old one is left recoverable.
            print(
                f"warning: published {output_dir}, but could not remove old "
                f"backup {backup}: {exc}",
                file=sys.stderr,
            )


def convert(export_dir: Path, output_dir: Path, *, verbose: bool = False):
    """Transactionally convert one export without modifying either old tree."""
    export_dir, output_dir = _validate_separate_trees(export_dir, output_dir)
    # `vrfkit export` writes all six in one transaction: an export missing one
    # is regenerated, never converted with stand-ins.
    for name in ("manifest.json", "fields.parquet", "actors.parquet",
                 "net_guids.parquet", "events.parquet", "movement.parquet"):
        if not (export_dir / name).is_file():
            raise FileNotFoundError(f"{name} not found in {export_dir}")

    output_dir.parent.mkdir(parents=True, exist_ok=True)
    parent = output_dir.parent.resolve()
    require_descendant(output_dir, parent)
    staging = Path(tempfile.mkdtemp(prefix=f".{output_dir.name}.", dir=parent))
    try:
        result = _convert_into(export_dir, staging, verbose=verbose)
        _publish_bundle(staging, output_dir)
    except BaseException:
        if staging.exists():
            remove_tree(staging, parent)
        raise
    _print_summary(output_dir, result)
    return result


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------
def main():
    # ~680k acyclic event dicts live until the sort; the cyclic collector's
    # rescans cost ~17% of a run. Here, not at import: the tests import this.
    gc.disable()

    parser = argparse.ArgumentParser(
        description="Convert vrfkit Parquet export to valplay NDJSON bundle"
    )
    parser.add_argument("export_dir", type=Path,
                        help="vrfkit export directory")
    parser.add_argument("-o", "--output", type=Path, default=None,
                        help="Output bundle directory (default: out/valplay_bundle/<stem>)")
    parser.add_argument("-v", "--verbose", action="store_true",
                        help="Print progress messages")
    args = parser.parse_args()

    export_dir = args.export_dir.resolve()
    if args.output:
        output_dir = args.output.resolve()
    else:
        manifest = json.loads((export_dir / "manifest.json").read_text(encoding='utf-8'))
        source = manifest.get("source_file")
        stem = PureWindowsPath(source).stem if source else export_dir.name
        output_dir = Path(__file__).resolve().parent.parent / "out" / "valplay_bundle" / stem

    convert(export_dir, output_dir, verbose=args.verbose)


if __name__ == "__main__":
    main()
