"""Validate proposed overlay types directly against exported raw field bits.

An evidence tool, not type inference. Usage:
    python tools/validate_type_evidence.py EXPORT_OR_PARENT EVIDENCE.json [--compare-typed]

EVIDENCE is ``[{"group": ..., "field": ..., "type": ...[, "checksum": u32]}]``
or ``scoped_type_evidence.json`` (see `load_specifications`). Every matching
payload must be consumed exactly, carry zero padding above ``bit_count`` and
hold finite floats; the report gives widths and value ranges so a reviewer
can reject a decodable but implausible type. UInt32 is read unsigned: an
Int32 reading of the same bits passes the width check and is still wrong.
EnumByte takes its 1..8-bit width from the payload. A ReplicatedMovement
location's scale is not on the wire, so ``--compare-typed`` accepts it at /100
or /1 and reports which (``location_scales``); the level each class must have
is pinned by the Rust test
``every_rep_movement_entry_carries_its_measured_location_level``.

The decoders follow Unreal's wire layouts, not the Rust readers, so the two
can disagree. A directory is searched recursively, skipping the leftovers of
an interrupted ``vrfkit export`` (see export_scan.py) and refusing a table
with no ``manifest.json`` beside it; a table file or ``--export-id`` is read
as given.

The report's ``status`` is ``pass`` or ``fail``, matching the exit code. With
``--output``, a run that does not finish leaves ``incomplete`` or ``error``
there, with no counts. An existing ``--output`` is replaced only if it parses
as one of this tool's reports.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import struct
import sys
from collections import Counter, defaultdict
from pathlib import Path

import pyarrow as pa
import pyarrow.compute as pc
import pyarrow.parquet as pq

from atomic_io import atomic_write_text, refuse_input_path
from export_scan import generated_ancestor, leftover_note, skipped_report


def _signed(value: int, width: int) -> int:
    """Two's-complement sign extension of a `width`-bit field."""
    sign = 1 << (width - 1)
    return (value ^ sign) - sign


class Bits:
    """LSB-first reader over exactly ``bit_count`` bits of one payload.

    Reading past ``bit_count`` raises, and so does nonzero padding above it:
    the exporter zero-fills it, so a set bit means the window is not the one
    the type describes.
    """

    def __init__(self, raw: bytes, bit_count: int):
        if len(raw) != (bit_count + 7) // 8:
            raise ValueError(f"raw_bits holds {len(raw)} bytes, bit_count {bit_count} needs {(bit_count + 7) // 8}")
        self._value = int.from_bytes(raw, "little")
        if self._value >> bit_count:
            raise ValueError("nonzero padding above bit_count")
        self._end = bit_count
        self.pos = 0

    def bits(self, count: int) -> int:
        if self.pos + count > self._end:
            raise ValueError("truncated payload: read past the end of the payload")
        value = (self._value >> self.pos) & ((1 << count) - 1)
        self.pos += count
        return value

    def bit(self) -> bool:
        return bool(self.bits(1))

    def remaining(self) -> int:
        return self._end - self.pos

    def ieee(self, size: int) -> float:
        """A little-endian float of 32 or 64 bits."""
        return struct.unpack("<f" if size == 32 else "<d", self.bits(size).to_bytes(size // 8, "little"))[0]

    def int_packed(self) -> int:
        """``SerializeIntPacked``: 7 payload bits per byte, low bit continues,
        at most 32 bits."""
        value = 0
        for index in range(5):
            byte = self.bits(8)
            chunk = byte >> 1
            if index == 4:
                if byte & 1:
                    raise ValueError("runaway IntPacked")
                if chunk > 15:
                    raise ValueError("overflowing IntPacked")
            value |= chunk << (index * 7)
            if not byte & 1:
                return value
        raise AssertionError("unreachable IntPacked loop end")

    def serialized_int(self, max_value: int) -> int:
        """``FBitReader::SerializeInt``: floor(log2(max)) bits, then one more
        only while the value could still be raised without reaching max."""
        width = max_value.bit_length() - 1
        value = self.bits(width)
        mask = 1 << width
        if value + mask < max_value and self.bit():
            value |= mask
        return value

    def fstring(self) -> str:
        length = _signed(self.bits(32), 32)
        if length == 0:
            return ""
        unit = 1 if length > 0 else 2
        data = bytes(self.bits(8) for _ in range(abs(length) * unit))
        if data[-unit:] != b"\0" * unit:
            raise ValueError("FString lacks its terminator")
        return data[:-unit].decode("utf-8" if unit == 1 else "utf-16-le")


def fname(reader: Bits) -> str:
    """An isHardcoded bit, then an IntPacked index (its decimal string) or an
    FString and an instance number N (N > 0 renders ``name_{N-1}``)."""
    if reader.bit():
        return str(reader.int_packed())
    name = reader.fstring()
    number = _signed(reader.bits(32), 32)
    if number < 0:
        raise ValueError("negative FName instance number")
    return name if number == 0 else f"{name}_{number - 1}"


#: Nesting limit shared with the Rust tree reader; deeper input is refused.
FTEXT_MAX_DEPTH = 16
#: A format history's argument count must lie in 0..=this, as in Rust.
FTEXT_MAX_ARGUMENTS = 128
#: The four digit members of FNumberFormattingOptions, in wire order after its
#: two bools and rounding byte.
FTEXT_DIGITS = ("minimum_integral_digits", "maximum_integral_digits",
                "minimum_fractional_digits", "maximum_fractional_digits")


def _ftext_bool(reader: Bits) -> bool:
    """An archive bool: a whole u32 that must be 0 or 1."""
    value = reader.bits(32)
    if value not in (0, 1):
        raise ValueError(f"FText bool is {value}, not 0 or 1")
    return bool(value)


def _ftext_tree(reader: Bits, depth: int = 0) -> dict:
    """``FText``: u32 flags, a history byte and the history (255 empty, 11
    string table, 3 argument format, 4 as-number with a double source), in
    the shape of the exporter's JSON."""
    if depth >= FTEXT_MAX_DEPTH:
        raise ValueError("FText nesting exceeds the depth limit")
    flags = reader.bits(32)
    history = reader.bits(8)
    if history == 255:
        if flags != 0 or reader.bits(32) != 0:
            raise ValueError("FText empty history must have zero flags and zero length")
        return {"flags": flags, "history": 255, "kind": "empty"}
    if history == 11:
        if reader.bit():
            raise ValueError("FText string table name is not inline")
        name = reader.fstring()
        number = _signed(reader.bits(32), 32)
        if number < 0:
            raise ValueError("negative FText table name number")
        key = reader.fstring()
        return {"flags": flags, "history": 11, "kind": "string_table",
                "table": {"name": name, "number": number}, "key": key}
    if history == 3:
        source = _ftext_tree(reader, depth + 1)
        count = _signed(reader.bits(32), 32)
        if not 0 <= count <= FTEXT_MAX_ARGUMENTS:
            raise ValueError(f"FText argument count {count}")
        arguments = []
        for _ in range(count):
            name = reader.fstring()
            tag = reader.bits(8)
            if tag == 0:
                value = {"bits_u64": str(reader.bits(64))}
            elif tag == 4:
                value = _ftext_tree(reader, depth + 1)
            else:
                raise ValueError(f"FText argument tag {tag}")
            arguments.append({"name": name, "tag": tag, "value": value})
        return {"flags": flags, "history": 3, "kind": "format", "source": source,
                "arguments": arguments}
    if history == 4:
        tag = reader.bits(8)
        if tag != 3:
            raise ValueError(f"FText number source tag {tag}")
        number = reader.ieee(64)
        if not math.isfinite(number):
            raise ValueError("FText number is non-finite")
        options = None
        if _ftext_bool(reader):
            options = {"always_sign": _ftext_bool(reader), "use_grouping": _ftext_bool(reader),
                       "rounding_mode": _signed(reader.bits(8), 8)}
            for member in FTEXT_DIGITS:
                options[member] = _signed(reader.bits(32), 32)
        culture = reader.fstring()
        return {"flags": flags, "history": 4, "kind": "as_number",
                "source": {"tag": 3, "double": number}, "format": options, "culture": culture}
    raise ValueError(f"FText history {history}")


def _packed_vector(reader: Bits) -> dict:
    """``ReadPackedVector``: a SerializeInt(128) header whose low six bits are
    the component width and whose seventh says "scaled"; width 0 falls back to
    three floats, doubles when scaled. Returned unscaled (``{"packed": ints,
    "scaled": flag}`` or ``{"floats": values}``): the scale is the reader's.
    """
    header = reader.serialized_int(1 << 7)
    width, scaled = header & 63, bool(header >> 6)
    if width:
        return {"packed": tuple(_signed(reader.bits(width), width) for _ in range(3)),
                "scaled": scaled}
    parts = tuple(reader.ieee(64 if scaled else 32) for _ in range(3))
    if not all(math.isfinite(p) for p in parts):
        raise ValueError("non-finite packed vector component")
    return {"floats": parts}


def _unit_scale(vector: dict) -> tuple[float, float, float]:
    """A packed vector in whole units."""
    if "floats" in vector:
        return vector["floats"]
    return tuple(float(p) for p in vector["packed"])


def _vector_net_quantize100(reader: Bits) -> tuple[float, float, float]:
    """``FVector_NetQuantize100``: the type names its scale, hundredths."""
    vector = _packed_vector(reader)
    if vector.get("scaled"):
        return tuple(p / 100 for p in vector["packed"])
    return _unit_scale(vector)


def _rotator(reader: Bits, width: int) -> tuple[float, float, float]:
    """A rotator: one presence bit per component, then `width` bits if set."""
    return tuple(
        reader.bits(width) * 360.0 / (1 << width) if reader.bit() else 0.0
        for _ in range(3)
    )


def _rep_movement(reader: Bits, rotation_bits: int) -> dict:
    """``FRepMovement::NetSerialize``, in wire order."""
    sleep, physics, frame, handle = (reader.bit() for _ in range(4))
    location = _packed_vector(reader)
    rotation = tuple(map(_f32, _rotator(reader, rotation_bits)))
    velocity = _unit_scale(_packed_vector(reader))
    return {
        "linear_velocity": velocity,
        "angular_velocity": _unit_scale(_packed_vector(reader)) if physics else None,
        "location": location,
        "rotation": rotation,
        "simulated_physics_sleep": sleep,
        "rep_physics": physics,
        "server_frame": reader.int_packed() if frame else None,
        "server_physics_handle": reader.int_packed() if handle else None,
    }


def _f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", value))[0]


def _exported_rep_movement(text: str) -> dict:
    """The exporter's ``value_str`` JSON in the shape ``_rep_movement`` returns."""
    obj = json.loads(text)

    def vector(v):
        return None if v is None else (v["x"], v["y"], v["z"])

    rotation = obj["rotation"]
    return {
        "linear_velocity": vector(obj["linear_velocity"]),
        "angular_velocity": vector(obj["angular_velocity"]),
        "location": vector(obj["location"]),
        "rotation": tuple(_f32(rotation[k]) for k in ("pitch", "yaw", "roll")),
        "simulated_physics_sleep": obj["simulated_physics_sleep"],
        "rep_physics": obj["rep_physics"],
        "server_frame": obj["server_frame"],
        "server_physics_handle": obj["server_physics_handle"],
    }


def _enum_byte(reader: Bits) -> int:
    if not 1 <= reader.remaining() <= 8:
        raise ValueError("EnumByte is not 1..8 bits")
    return reader.bits(reader.remaining())


def _enum_remaining_bits(reader: Bits) -> int:
    if reader.remaining() > 32:
        raise ValueError("EnumRemainingBits wider than 32 bits")
    return reader.bits(reader.remaining())


#: Byte-aligned types: exact width, struct format, exported column.
FIXED = {"Byte": (8, "<B", "value_i64"), "Int32": (32, "<i", "value_i64"),
         "UInt32": (32, "<I", "value_i64"), "Float": (32, "<f", "value_f64"),
         "Double": (64, "<d", "value_f64"), "VectorDouble": (192, "<3d", "value_str")}
#: Types read with `Bits`: decoder, exported column.
BIT_LEVEL_TYPES = {
    "Bool": (Bits.bit, "value_bool"),
    "FString": (Bits.fstring, "value_str"),
    "ObjectNetGuid": (Bits.int_packed, "value_i64"),
    "EnumByte": (_enum_byte, "value_i64"),
    "EnumRemainingBits": (_enum_remaining_bits, "value_i64"),
    "FName": (fname, "value_str"),
    "FTextTree": (_ftext_tree, "value_str"),
    "RotationShort": (lambda reader: _rotator(reader, 16), "value_str"),
    "VectorNetQuantize100": (_vector_net_quantize100, "value_str"),
    "RepMovementByte": (lambda reader: _rep_movement(reader, 8), "value_str"),
    "RepMovementShort": (lambda reader: _rep_movement(reader, 16), "value_str"),
}
#: Every supported evidence type, and the column its exported value lives in.
TYPED_COLUMNS = {name: spec[-1] for table in (FIXED, BIT_LEVEL_TYPES) for name, spec in table.items()}


def decode_exact(raw: bytes | None, bit_count: int, type_name: str):
    """Decode one value while requiring the declared payload width."""
    raw = raw or b""  # a zero-bit payload is exported as null
    reader = Bits(raw, bit_count)
    if type_name in FIXED:
        width, fmt, _column = FIXED[type_name]
        if bit_count != width:
            raise ValueError(f"{type_name} is not {width} bits")
        value = struct.unpack(fmt, raw)
        if not all(map(math.isfinite, value)):
            raise ValueError(f"{type_name} is non-finite")
        return value if len(value) > 1 else value[0]
    if type_name not in BIT_LEVEL_TYPES:
        raise ValueError(f"unsupported evidence type {type_name!r}")
    value = BIT_LEVEL_TYPES[type_name][0](reader)
    if reader.remaining():
        raise ValueError(f"{type_name} leaves {reader.remaining()} residual bits")
    return value


#: Location scales a RepMovement comparison accepts, in the order tried.
LOCATION_SCALES = (100, 1)


def rep_movement_location_scale(decoded: dict, exported) -> str | None:
    """How `exported` renders `decoded`'s location, or None if it does not.

    Every other member must be equal exactly; the location may be the packed
    integers divided by one scale in LOCATION_SCALES, the same on all three
    components. A float fallback or an unscaled packing has one rendering.
    """
    if not isinstance(exported, dict) or exported.keys() != decoded.keys():
        return None
    if any(decoded[k] != exported[k] for k in decoded if k != "location"):
        return None
    location, got = decoded["location"], exported["location"]
    if "floats" in location:
        return "raw floats" if location["floats"] == got else None
    if not location["scaled"]:
        return "unscaled" if tuple(float(p) for p in location["packed"]) == got else None
    for scale in LOCATION_SCALES:
        if tuple(p / scale for p in location["packed"]) == got:
            return f"/{scale}"
    return None


def _parse_triple(text, prefix: str):
    if not isinstance(text, str) or not text.startswith(prefix) or not text.endswith(")"):
        return None
    parts = text[len(prefix):-1].split(",")
    if len(parts) != 3:
        return None
    try:
        return tuple(float(part) for part in parts)
    except ValueError:
        return None


def exported_matches(type_name: str, exported, decoded) -> bool:
    """Whether the exported column holds the independently decoded value.
    Geometry strings are parsed back into numbers; rotator components are
    compared as f32, whose shortest spelling does not parse back exactly."""
    if type_name in {"VectorNetQuantize100", "VectorDouble"}:
        return _parse_triple(exported, "(") == decoded
    if type_name == "FTextTree":
        try:
            return json.loads(exported) == decoded
        except (TypeError, ValueError):
            return False
    if type_name == "RotationShort":
        parsed = _parse_triple(exported, "rot(")
        return parsed is not None and all(
            _f32(a) == _f32(b) for a, b in zip(parsed, decoded))
    return exported == decoded


def _summary(type_name: str, value):
    """The numbers a range check can use: a scalar, or a vector's components.
    None for ReplicatedMovement, whose location has more than one scale."""
    if type_name.startswith("RepMovement"):
        return None
    if isinstance(value, tuple):
        return value
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return value
    return None


def values_match(type_name: str, decoded, exported) -> tuple[bool, str | None]:
    """`(matches, detail)`; detail is a RepMovement row's location scale."""
    if type_name.startswith("RepMovement"):
        scale = rep_movement_location_scale(decoded, exported)
        return scale is not None, scale
    return exported_matches(type_name, exported, decoded), None


def exported_value(row: dict, type_name: str):
    """The exported typed value, in the shape `decode_exact` returns. A
    RepMovement `value_str` that does not parse becomes a marker equal to no
    decoded value: a mismatch, not a stopped run."""
    value = row[TYPED_COLUMNS[type_name]]
    if type_name.startswith("RepMovement") and value is not None:
        try:
            return _exported_rep_movement(value)
        except (ValueError, KeyError, TypeError):
            return {"unparseable value_str": value}
    return value


def parquet_files(root: Path, export_ids=None, skipped=None):
    """Yield the field tables to read below `root`, filtering only a recursive
    search (see the module docstring); skipped directories go into `skipped`
    when one is given."""
    if root.is_file():
        yield root
        return
    if export_ids:
        for export_id in export_ids:
            tables = [root / export_id / name for name in ("fields.parquet", "checkpoint_fields.parquet")
                      if (root / export_id / name).exists()]
            if not tables:
                raise ValueError(f"--export-id {export_id}: no field table in {root / export_id}")
            yield from tables
        return
    for name in ("fields.parquet", "checkpoint_fields.parquet"):
        for path in root.rglob(name):
            leftover = generated_ancestor(path, root)
            if leftover is not None:
                if skipped is not None:
                    skipped.add(leftover)
                continue
            if not (path.parent / "manifest.json").is_file():
                raise ValueError(
                    f"{path} has no manifest.json beside it; vrfkit writes the manifest last, "
                    "so this is not a finished export (an interrupted export or a partial copy)")
            yield path


def spec_rows(table: pa.Table, groups: pa.Array, fields: pa.Array) -> pa.Table:
    """The rows whose group is in `groups` AND whose field is in `fields`, in
    order: a superset of the specified pairs, which `validate` then resolves
    exactly, that keeps a specified group's other fields out of Python."""
    return table.filter(pc.and_(
        pc.is_in(pc.cast(table["group_path"], pa.string()), value_set=groups),
        pc.is_in(pc.cast(table["field_name"], pa.string()), value_set=fields),
    ))


def check_specifications(specifications: list[dict]) -> dict:
    """`{(group, field, checksum or None): type}`; a malformed, duplicate or
    scoped-beside-unscoped entry raises."""
    if not specifications:
        raise ValueError("evidence specification is empty")
    for spec in specifications:
        if set(spec) not in ({"group", "field", "type"}, {"group", "field", "type", "checksum"}) or not spec["group"] or not spec["field"]:
            raise ValueError(f"invalid evidence specification: {spec!r}")
        if "checksum" in spec and (type(spec["checksum"]) is not int or not 0 < spec["checksum"] <= 0xFFFFFFFF):
            raise ValueError("checksum scope must be a nonzero u32")
        if spec["type"] not in TYPED_COLUMNS:
            raise ValueError(f"unsupported evidence type: {spec['type']!r}")
    expected = {(s["group"], s["field"], s.get("checksum")): s["type"] for s in specifications}
    if len(expected) != len(specifications):
        raise ValueError("duplicate group/field specification")
    for group, field, checksum in expected:
        if checksum is not None and (group, field, None) in expected:
            raise ValueError("overlapping scoped and unscoped specification")
    return expected


def validate(export_root: Path, specifications: list[dict], export_ids=None, compare_typed=False) -> dict:
    if not export_root.exists():
        raise ValueError(f"export root does not exist: {export_root}")
    expected = check_specifications(specifications)

    def label_for(key):
        group, field, checksum = key
        suffix = f"::checksum={checksum}" if checksum is not None else ""
        return f"{group}::{field}{suffix}"
    counts = Counter()
    widths = defaultdict(Counter)
    wire_keys = defaultdict(Counter)
    minima, maxima = {}, {}
    failures = []
    failure_count = 0
    failure_counts = Counter()
    typed_mismatch_count = 0
    typed_mismatch_examples = []
    location_scales = defaultdict(Counter)
    skipped = set()
    paths = list(parquet_files(export_root, export_ids, skipped))
    if not paths:
        raise ValueError(f"no field parquet files below {export_root}{leftover_note(skipped)}")
    wanted_groups = pa.array(sorted({group for group, _field, _checksum in expected}))
    wanted_fields = pa.array(sorted({field for _group, field, _checksum in expected}))
    columns = ["group_path", "field_name", "handle", "compatible_checksum", "bit_count", "raw_bits"]
    if compare_typed:
        columns += ["value_str", "value_i64", "value_f64", "value_bool"]
    for path in paths:
        with pq.ParquetFile(path, pre_buffer=False) as source:
            for batch in source.iter_batches(batch_size=131072, columns=columns, use_threads=False):
                table = spec_rows(pa.Table.from_batches([batch]), wanted_groups, wanted_fields)
                for row in table.to_pylist():
                    key = (row["group_path"], row["field_name"], row["compatible_checksum"])
                    if key not in expected:
                        key = (row["group_path"], row["field_name"], None)
                    type_name = expected.get(key)
                    if type_name is None:
                        continue
                    label = label_for(key)
                    counts[label] += 1
                    widths[label][row["bit_count"]] += 1
                    wire_keys[label][
                        f"handle={row['handle']},checksum={row['compatible_checksum']},bits={row['bit_count']}"
                    ] += 1
                    try:
                        value = decode_exact(row["raw_bits"], row["bit_count"], type_name)
                        if compare_typed:
                            exported = exported_value(row, type_name)
                            matched, detail = values_match(type_name, value, exported)
                            if detail is not None:
                                location_scales[label][detail] += 1
                            if not matched:
                                typed_mismatch_count += 1
                                if len(typed_mismatch_examples) < 32:
                                    typed_mismatch_examples.append({
                                        "file": str(path), "field": label,
                                        "decoded": value,
                                        "exported": row[TYPED_COLUMNS[type_name]],
                                    })
                        summary = _summary(type_name, value)
                        if isinstance(summary, tuple):
                            low = minima.get(label, summary)
                            high = maxima.get(label, summary)
                            minima[label] = tuple(map(min, low, summary))
                            maxima[label] = tuple(map(max, high, summary))
                        elif summary is not None:
                            minima[label] = min(minima.get(label, summary), summary)
                            maxima[label] = max(maxima.get(label, summary), summary)
                    except ValueError as exc:
                        failure_count += 1
                        failure_counts[label] += 1
                        if len(failures) < 32:
                            failures.append({"file": str(path), "field": label, "error": str(exc)})
    missing = [label_for(key) for key in expected if not counts[label_for(key)]]
    return {
        "fields": {
            label: {
                "rows": count,
                "widths": dict(sorted(widths[label].items())),
                "wire_keys": dict(sorted(wire_keys[label].items())),
                **({"min": minima[label], "max": maxima[label]} if label in minima else {}),
                **({"location_scales": dict(sorted(location_scales[label].items()))}
                   if label in location_scales else {}),
            }
            for label, count in sorted(counts.items())
        },
        "missing": missing,
        "failure_count": failure_count,
        "failure_counts": dict(sorted(failure_counts.items())),
        "failure_examples": failures,
        "typed_mismatch_count": typed_mismatch_count,
        "typed_mismatch_examples": typed_mismatch_examples,
        "skipped_generated_dirs": skipped_report(skipped),
        "parquet_files": len(paths),
    }


def load_specifications(path: Path) -> list[dict]:
    """A specification list, or `scoped_type_evidence.json` (`{"entries":
    [...]}`), whose RPC parameter group `Class:Function` is exported as group
    `Class_ClassNetCache`, field `Function.field`."""
    document = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        return document
    specifications = []
    for entry in document["entries"]:
        group, field = entry["group"], entry["field"]
        if ":" in group:
            group, function = group.split(":", 1)
            group, field = f"{group}_ClassNetCache", f"{function}.{field}"
        specifications.append({"group": group, "field": field,
                               "checksum": entry["checksum"], "type": entry["type"]})
    return specifications


REPORT_NAME = "validate_type_evidence"
# Every report written before the ``report`` marker carried these keys.
LEGACY_REPORT_KEYS = frozenset({"fields", "missing", "failure_count", "provenance"})


def is_own_report(path: Path) -> bool:
    """Whether `path` holds a report this tool wrote, so replacing it loses
    nothing a rerun cannot regenerate."""
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return False
    return isinstance(document, dict) and (document.get("report") == REPORT_NAME
                                           or LEGACY_REPORT_KEYS <= document.keys())


def unfinished_report(status: str, error: str) -> str:
    """A report with no counts, so a consumer reading `failure_count` fails."""
    return json.dumps({"report": REPORT_NAME, "status": status, "error": error},
                      indent=2, ensure_ascii=True) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("export_root", type=Path)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--export-id", action="append", dest="export_ids")
    parser.add_argument("--compare-typed", action="store_true")
    parser.add_argument("--allow-missing", action="store_true",
                        help="list specified identities with no row under `missing` without failing; "
                             "observing none still fails")
    args = parser.parse_args(argv)
    if args.output:
        if args.output.suffix.lower() == ".parquet" or args.output.name == "manifest.json":
            parser.error("--output must not name an export table or manifest")
        try:
            refuse_input_path(args.output, (args.evidence, Path(__file__)))
        except ValueError as exc:
            parser.error(str(exc))
        if args.output.exists() and not is_own_report(args.output):
            parser.error(f"--output {args.output} exists and is not a {REPORT_NAME} report; "
                         "refusing to replace it")
        # Before reading any input: a run that raises must leave this behind,
        # never an earlier pass.
        atomic_write_text(args.output, unfinished_report("incomplete", "validation did not finish"))
    try:
        specifications = load_specifications(args.evidence)
        report = validate(args.export_root, specifications, args.export_ids, args.compare_typed)
    except Exception as exc:
        if args.output:
            atomic_write_text(args.output, unfinished_report("error", f"{type(exc).__name__}: {exc}"))
        raise
    failed = bool((report["missing"] and not args.allow_missing) or not report["fields"]
                  or report["failure_count"] or report["typed_mismatch_count"])
    report = {"report": REPORT_NAME, "status": "fail" if failed else "pass", **report}
    report["provenance"] = {
        "export_root": str(args.export_root.resolve()),
        "parquet_files": report.pop("parquet_files"),
        "export_ids": sorted(args.export_ids) if args.export_ids else None,
        "specification": str(args.evidence.resolve()),
        "specification_sha256": hashlib.sha256(args.evidence.read_bytes()).hexdigest(),
    }
    rendered = json.dumps(report, indent=2, ensure_ascii=True) + "\n"
    if args.output:
        atomic_write_text(args.output, rendered)
    print(rendered, end="")
    if not report["fields"]:
        print("no specified identity was observed", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
