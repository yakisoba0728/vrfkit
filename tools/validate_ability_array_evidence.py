"""Independently inspect two observed ability DynamicArray wire payloads.

Usage: python tools/validate_ability_array_evidence.py EXPORT_DIR [...]
The input directories contain fields.parquet. Every matching parent is checked
for explicit array/element terminators, exact bit consumption, known member
handles, and the member widths declared by the pinned C# descriptors.
"""

from __future__ import annotations

import argparse
from collections import Counter
import json
import math
from pathlib import Path
import struct

import pyarrow as pa
import pyarrow.parquet as pq


ROUTES = {
    ("/Script/ShooterGame.BlindManagerComponent", "ActiveBlinds", 3853965310): {
        3: ("BlindId", None),
        4: ("EffectID", None),
        5: ("SourceID", None),
        6: ("bLocalEffect", 1),
        7: ("bTransient", 1),
        8: ("InitialDuration", 32),
        9: ("StartNetMovementTime", 32),
        10: ("BlindConfig", None),
        11: ("CausingActor", None),
    },
    (
        "/Script/ShooterGame.PrecalculatedProjectileMovementComponent_ClassNetCache",
        "MulticastSetPath.NetworkedProjectilePath",
        2930105559,
    ): {
        1: ("ElapsedSeconds", 32),
        2: ("Location", 192),
        3: ("Velocity", 192),
    },
}

BLIND_DECLARATIONS = {
    2: ("ActiveBlinds", 3853965310),
    3: ("BlindId", 2836858544),
    4: ("EffectID", 3321413110),
    5: ("SourceID", 4130766059),
    6: ("bLocalEffect", 2802682995),
    7: ("bTransient", 815378154),
    8: ("InitialDuration", 1370668337),
    9: ("StartNetMovementTime", 2358118895),
    10: ("BlindConfig", 4121438116),
    11: ("CausingActor", 2370661694),
}
PATH_GROUP = "/Script/ShooterGame.PrecalculatedProjectileMovementComponent:MulticastSetPath"


class Bits:
    def __init__(self, data: bytes, length: int):
        assert len(data) * 8 >= length
        self.data = data
        self.length = length
        self.pos = 0

    def read(self, width: int) -> int:
        if self.pos + width > self.length:
            raise ValueError(f"short read at bit {self.pos}: wanted {width}")
        value = 0
        for shift in range(width):
            value |= ((self.data[self.pos >> 3] >> (self.pos & 7)) & 1) << shift
            self.pos += 1
        return value

    def packed(self) -> int:
        value = 0
        for shift in range(0, 35, 7):
            byte = self.read(8)
            value |= (byte >> 1) << shift
            if byte & 1 == 0:
                return value
        raise ValueError("IntPacked overflow")


def member_value(handle: int, data: bytes, width: int, path_point: bool):
    bits = Bits(data, width)
    if path_point:
        if handle == 1:
            value = struct.unpack("<f", bits.read(32).to_bytes(4, "little"))[0]
            result = ("value_f64", value)
        else:
            result = (
                "value_str",
                tuple(struct.unpack("<d", bits.read(64).to_bytes(8, "little"))[0] for _ in range(3)),
            )
    elif handle in (3, 4):
        result = ("value_i64", bits.read(width))
    elif handle == 5:
        if bits.read(1):
            name = str(bits.packed())
        else:
            length_raw = bits.read(32)
            length = length_raw - (1 << 32) if length_raw >= 1 << 31 else length_raw
            if abs(length) > 65536:
                raise ValueError("FName string too large")
            unit_count = abs(length)
            unit_width = 16 if length < 0 else 8
            text = b"".join(bits.read(unit_width).to_bytes(unit_width // 8, "little") for _ in range(unit_count))
            name = text.decode("utf-16-le" if length < 0 else "utf-8").rstrip("\0")
            number = bits.read(32)
            if number >= 1 << 31:
                raise ValueError("negative FName instance")
            if number:
                name += f"_{number - 1}"
        result = ("value_str", name)
    elif handle in (6, 7):
        result = ("value_bool", bool(bits.read(1)))
    elif handle in (8, 9):
        result = ("value_f64", struct.unpack("<f", bits.read(32).to_bytes(4, "little"))[0])
    else:
        result = ("value_i64", bits.packed())
    if bits.pos != width:
        raise ValueError(f"member {handle} consumed {bits.pos} of {width} bits")
    if result[0] == "value_f64" and not math.isfinite(result[1]):
        raise ValueError(f"member {handle} is non-finite")
    if path_point and handle in (2, 3) and not all(math.isfinite(v) for v in result[1]):
        raise ValueError(f"vector member {handle} is non-finite")
    return result


def inspect(row: dict, spec: dict) -> tuple[int, Counter, dict]:
    bits = Bits(row["raw_bits"], row["bit_count"])
    capacity = bits.packed()
    if capacity > 4096:
        raise ValueError(f"array capacity {capacity} exceeds decoder limit")
    observations = Counter()
    children = {}
    path_point = row["field_name"].startswith("MulticastSetPath.")
    seen = 0
    while True:
        encoded_index = bits.packed()
        if encoded_index == 0:
            break
        if encoded_index - 1 >= capacity:
            raise ValueError(f"array index {encoded_index - 1} >= capacity {capacity}")
        seen += 1
        if seen > 4096:
            raise ValueError("too many array elements")
        fields = 0
        while True:
            encoded_handle = bits.packed()
            if encoded_handle == 0:
                break
            handle = encoded_handle - 1
            width = bits.packed()
            name, expected = spec.get(handle, (None, None))
            if name is None:
                raise ValueError(f"unknown member handle {handle}")
            if expected is not None and width != expected:
                raise ValueError(f"{name} width {width}, expected {expected}")
            if not path_point and handle in (3, 4, 5, 10, 11):
                allowed = {3: (32,), 4: (64,), 5: (297,), 10: (16,), 11: (8, 16, 24)}[handle]
                if width not in allowed:
                    raise ValueError(f"{name} width {width}, expected {allowed}")
            start = bits.pos
            if start + width > bits.length:
                raise ValueError(f"{name} extends past parent")
            raw = bits.read(width).to_bytes((width + 7) // 8, "little")
            child_name = f"{row['field_name']}[{encoded_index - 1}].{name}"
            if child_name in children:
                raise ValueError(f"duplicate child {child_name}")
            column, value = member_value(handle, raw, width, path_point)
            children[child_name] = (handle, width, raw, column, value)
            observations[(handle, width)] += 1
            fields += 1
            if fields > 128:
                raise ValueError("too many member fields")
    # ActiveBlinds is delta-replicated: a frame can update only selected
    # members, or change no elements and include one additional zero trailer.
    # Projectile path points still require all three members and no trailer.
    if not path_point and seen == 0 and bits.length - bits.pos == 8:
        if bits.packed() != 0:
            raise ValueError("nonzero empty-array trailer")
    if bits.pos != bits.length:
        raise ValueError(f"{bits.length - bits.pos} unconsumed bits")
    if path_point and len(children) != seen * 3:
        raise ValueError(f"{len(children)} members for {seen} elements")
    return seen, observations, children


def check_declarations(export: Path, rows: Counter) -> None:
    groups = {
        group["path"]: group["fields"]
        for group in json.loads((export / "manifest.json").read_text(encoding="utf-8"))[
            "net_field_export_groups"
        ]
    }
    if rows["ActiveBlinds"]:
        fields = groups["/Script/ShooterGame.BlindManagerComponent"]
        observed = {field["handle"]: (field["name"], field["compatible_checksum"]) for field in fields}
        for handle, declaration in BLIND_DECLARATIONS.items():
            if observed.get(handle) != declaration:
                raise ValueError(f"BlindManager handle {handle}: {observed.get(handle)} != {declaration}")
    if rows["MulticastSetPath.NetworkedProjectilePath"]:
        fields = groups[PATH_GROUP]
        observed = {field["handle"]: (field["name"], field["compatible_checksum"]) for field in fields}
        wanted = ("NetworkedProjectilePath", 2930105559)
        if observed.get(0) != wanted:
            raise ValueError(f"MulticastSetPath handle 0: {observed.get(0)} != {wanted}")


def compare_children(parent: dict, expected: dict, emitted: list[dict]) -> None:
    if len(expected) != len(emitted):
        raise ValueError(f"{parent['field_name']}: {len(emitted)} emitted children, expected {len(expected)}")
    scope = ("time_ms", "packet_id", "channel_index", "actor_net_guid", "object_net_guid", "group_path")
    path_point = parent["field_name"].startswith("MulticastSetPath.")
    for (name, (handle, width, raw, column, value)), child in zip(expected.items(), emitted):
        if child["field_name"] != name:
            raise ValueError(f"child path {child['field_name']} != {name}")
        if any(child[key] != parent[key] for key in scope):
            raise ValueError(f"{name}: child context differs from parent")
        if child["handle"] != (0 if path_point else handle):
            raise ValueError(f"{name}: wrong exported handle {child['handle']}")
        if child["compatible_checksum"] is not None:
            raise ValueError(f"{name}: nested child has a top-level checksum")
        if child["bit_count"] != width or child["raw_bits"] != raw:
            raise ValueError(f"{name}: child raw window differs from parent")
        if value is None or child[column] is None:
            raise ValueError(f"{name}: null typed value")
        if column == "value_str" and isinstance(value, tuple):
            text = child[column]
            if not (text.startswith("(") and text.endswith(")")):
                raise ValueError(f"{name}: malformed vector {text!r}")
            actual = tuple(float(part) for part in text[1:-1].split(","))
            if actual != value:
                raise ValueError(f"{name}: vector {actual} != {value}")
        elif child[column] != value:
            raise ValueError(f"{name}: value {child[column]!r} != {value!r}")
        if any(child[key] is not None for key in ("value_i64", "value_f64", "value_bool", "value_str") if key != column):
            raise ValueError(f"{name}: multiple typed columns")


def relevant_rows(export: Path, compare_typed: bool):
    columns = ["group_path", "field_name", "compatible_checksum", "bit_count", "raw_bits"]
    if compare_typed:
        columns += [
            "time_ms", "packet_id", "channel_index", "actor_net_guid", "object_net_guid",
            "handle", "value_i64", "value_f64", "value_bool", "value_str",
        ]
    parent_names = {key[1] for key in ROUTES}
    for batch in pq.ParquetFile(export / "fields.parquet").iter_batches(batch_size=65536, columns=columns):
        names = batch["field_name"].to_pylist()
        indices = [
            i for i, name in enumerate(names)
            if name in parent_names or (
                compare_typed and isinstance(name, str) and any(name.startswith(parent + "[") for parent in parent_names)
            )
        ]
        if indices:
            yield from batch.take(pa.array(indices, type=pa.int32())).to_pylist()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exports", nargs="+", type=Path)
    parser.add_argument("--require-routes", action="store_true", help="fail if either route has zero rows across all exports")
    parser.add_argument("--compare-typed", action="store_true", help="compare emitted child rows to an independent decode of parent raw bits")
    args = parser.parse_args()
    failed = False
    total_rows = Counter()
    for export in args.exports:
        rows = Counter({key[1]: 0 for key in ROUTES})
        elements = Counter()
        members = Counter()
        typed = Counter({key[1]: 0 for key in ROUTES})
        pending = {key[1]: [] for key in ROUTES}
        for row in relevant_rows(export, args.compare_typed):
            if args.compare_typed:
                child_route = next(
                    (key[1] for key in ROUTES if row["group_path"] == key[0] and row["field_name"] is not None
                     and row["field_name"].startswith(key[1] + "[")),
                    None,
                )
                if child_route:
                    pending[child_route].append(row)
                    continue
            key = (row["group_path"], row["field_name"], row["compatible_checksum"])
            spec = ROUTES.get(key)
            if spec is None:
                continue
            try:
                count, observed, expected_children = inspect(row, spec)
                if args.compare_typed:
                    compare_children(row, expected_children, pending[key[1]])
                    typed[key[1]] += len(expected_children)
            except ValueError as exc:
                failed = True
                print(f"{export}: {key[1]} {row['bit_count']} bits: {exc}")
                pending[key[1]].clear()
                continue
            pending[key[1]].clear()
            rows[key[1]] += 1
            elements[key[1]] += count
            members.update({(key[1], handle, width): count for (handle, width), count in observed.items()})
        if args.compare_typed and any(pending.values()):
            failed = True
            print(f"{export}: orphan children: { {name: len(children) for name, children in pending.items()} }")
        try:
            check_declarations(export, rows)
        except (KeyError, ValueError) as exc:
            failed = True
            print(f"{export}: declaration mismatch: {exc}")
        total_rows.update(rows)
        print(f"{export}: rows={dict(rows)} elements={dict(elements)} typed_children={dict(typed)} members={dict(members)}")
    if args.require_routes and any(total_rows[key[1]] == 0 for key in ROUTES):
        failed = True
        print(f"missing observed route: {dict(total_rows)}")
    if failed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
