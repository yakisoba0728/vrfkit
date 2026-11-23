"""Measure each class's `ReplicatedMovement` location level against its spawn.

usage: python tools/check_rep_movement_levels.py EXPORT_OR_PARENT [...]

The wire packs `round(world * scale)` and says only "scaled", so a wrong level
decodes with no error (docs/DATA.md#replicatedmovementlocation-is-world-units-at-a-per-class-level).
Per class, the main `fields.parquet` rows are read from `raw_bits` by
validate_type_evidence's reader:

- join: an `actors.parquet` `open` row with |spawn| >= 50 and the first row of
  the same actor, channel and `time_ms`; ratio = |packed| / |spawn| on scaled
  rows. The level is the scale (1, 10, 100) nearest the median ratio; it is
  clean when every join reads within rounding (0.5 / scale + 0.06) of the spawn.
- speed: consecutive rows of one actor, 0 < dt <= 0.2 s, |velocity| >= 200:
  median of (|step| / dt) / |velocity| at the measured level.
- rotator: rows consumed exactly as byte only / short only / either / neither.

A class the overlay types (`table.rs`, `scoped_types.rs`) must measure clean
at its declared level, with no row only the other rotator width consumes and
a typed value (`value_str`) on every row. A typed class with no row in the
input is counted, not failed -- one replay holds few of them -- unless
`--require-all` is given. Exit 1 otherwise, 2 when there is no export or no
`ReplicatedMovement` row.
"""
from __future__ import annotations

import argparse
import json
import math
import re
import statistics
import sys
from collections import Counter
from pathlib import Path

import pyarrow.compute as pc
from wire_bits import read_parquet

from export_scan import discover_exports
from overlay_mirror import overlay_entries, scoped_entries
from validate_type_evidence import decode_exact

FIELD = "ReplicatedMovement"
SRC = Path(__file__).resolve().parents[1] / "crates" / "vrf-decode" / "src"
LEVELS = {1: "RoundWholeNumber", 10: "RoundOneDecimal", 100: "RoundTwoDecimals"}
WIDTHS = {"ByteComponents": "RepMovementByte", "ShortComponents": "RepMovementShort"}
MIN_SPAWN, MIN_SPEED, MAX_DT = 50.0, 200.0, 0.2
#: The spawn itself is sent to a tenth of a unit, plus f32 slack.
SPAWN_SLACK = 0.06
ACTOR_COLUMNS = ["time_ms", "channel_index", "actor_net_guid", "class_path", "event",
                 "spawn_x", "spawn_y", "spawn_z"]
FIELD_COLUMNS = ["time_ms", "channel_index", "actor_net_guid", "group_path",
                 "compatible_checksum", "bit_count", "raw_bits", "value_str"]


def declared_types(src: Path = SRC) -> dict[str, tuple[str, str] | str]:
    """group -> (rotator, level) for a RepMovement entry, else the type text."""
    table = overlay_entries((src / "table.rs").read_text(encoding="utf-8"))
    scoped = scoped_entries((src / "scoped_types.rs").read_text(encoding="utf-8"))
    declared = {}
    for group, field_type in [(g, t) for g, n, t in table if n == FIELD] + [
            (g, t) for n, g, _c, t in scoped if n == FIELD]:
        m = re.search(r"RotatorQuantization::(\w+), location: VectorQuantization::(\w+)",
                      field_type)
        declared[group] = m.group(1, 2) if m else field_type.removeprefix("FieldType::")
    return declared


class ClassStats:
    def __init__(self):
        self.rows = self.typed = 0
        self.rotator = Counter()
        self.checksums, self.builds, self.join_builds = Counter(), set(), set()
        self.joins = []  # (packed, spawn) on scaled rows
        self.unscaled = 0
        self.steps = []  # (|packed step| / dt, |velocity|)


def decode(raw: bytes | None, bits: int):
    """(location, velocity, width) or None; width is "either" when both
    rotator widths consume the payload exactly."""
    results = {}
    for name in WIDTHS.values():
        try:
            results[name] = decode_exact(raw, bits, name)
        except ValueError:
            pass
    if not results:
        return None
    width = "either" if len(results) == 2 else next(iter(results))
    value = next(iter(results.values()))
    return value["location"], value["linear_velocity"], width


def read_export(export: Path, stats: dict[str, ClassStats], unjoined: Counter) -> int:
    build = json.loads((export / "manifest.json").read_text(encoding="utf-8")).get(
        "replay_build", "?")
    actors = read_parquet(export / "actors.parquet", columns=ACTOR_COLUMNS)
    actors = actors.filter(pc.and_(pc.equal(actors["event"], "open"),
                                   pc.is_valid(actors["spawn_x"])))
    spawns = {(c, g, t): (cls, (x, y, z)) for t, c, g, cls, x, y, z in zip(
        *(actors[n].to_pylist() for n in ACTOR_COLUMNS if n != "event"))
        if math.hypot(x, y, z) >= MIN_SPAWN}
    fields = read_parquet(export / "fields.parquet", columns=FIELD_COLUMNS,
                           filters=[("field_name", "==", FIELD)])
    last = {}
    for t, c, g, group, checksum, bits, raw, text in zip(
            *(fields[n].to_pylist() for n in FIELD_COLUMNS)):
        s = stats.setdefault(group, ClassStats())
        s.rows += 1
        s.typed += text is not None
        s.checksums[checksum] += 1
        s.builds.add(build)
        decoded = decode(raw, bits)
        s.rotator[decoded[2] if decoded else "neither"] += 1
        if decoded is None:
            continue
        location, velocity, _width = decoded
        packed = location.get("packed") if location.get("scaled") else None
        _cls, spawn = spawns.pop((c, g, t), (None, None))
        if spawn is not None:
            if packed is None:
                s.unscaled += 1
            else:
                s.joins.append((packed, spawn))
                s.join_builds.add(build)
        previous = last.get((c, g))
        if previous and packed is not None and previous[1] is not None:
            dt, speed = (t - previous[0]) / 1000, math.hypot(*previous[2])
            if 0 < dt <= MAX_DT and speed >= MIN_SPEED:
                step = math.dist(packed, previous[1])
                s.steps.append((step / dt, speed))
        last[(c, g)] = (t, packed, velocity)
    unjoined.update(cls for cls, _spawn in spawns.values())
    return fields.num_rows


def percentile(values: list[float], q: float) -> float:
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(q * len(ordered)))]


def measure(s: ClassStats) -> dict:
    """The level, its ratio percentiles and worst join, speed and rotator width."""
    out = {"level": None, "clean": False, "ratio": None, "worst": None, "speed": None}
    ratios = [math.hypot(*p) / math.hypot(*sp) for p, sp in s.joins]
    if ratios:
        median = statistics.median(ratios)
        scale = min(LEVELS, key=lambda k: abs(math.log(max(median, 1e-9) / k)))
        worst = max(max(abs(p / scale - q) for p, q in zip(packed, spawn))
                    for packed, spawn in s.joins)
        out.update(level=LEVELS[scale], worst=worst,
                   clean=worst <= 0.5 / scale + SPAWN_SLACK,
                   ratio=tuple(percentile(ratios, q) for q in (0.01, 0.5, 0.99)))
        if s.steps:
            out["speed"] = statistics.median(v / scale / speed for v, speed in s.steps)
    rotator = s.rotator
    only = [name for name in WIDTHS.values() if rotator[name]]
    out["width"] = (only[0] if len(only) == 1 and not rotator["neither"]
                    else "either" if not only and not rotator["neither"] else None)
    return out


def verdict(declared, m: dict, s: ClassStats) -> tuple[str, bool]:
    """(text, fails)."""
    if not isinstance(declared, tuple):
        state = "untyped" if declared is None else f"declared {declared}"
        if m["level"] is None:
            return f"{state}, no joins", False
        level = m["level"] if m["clean"] else "not clean"
        return f"{state}, {level}, rotator {m['width'] or 'inconsistent'}", False
    rotator, level = declared
    other = next(n for k, n in WIDTHS.items() if k != rotator)
    if s.rotator[other] or s.rotator["neither"]:
        return f"FAILED: {rotator} does not consume every row", True
    if s.typed < s.rows:
        return f"FAILED: {s.rows - s.typed} of {s.rows} rows have no typed value", True
    if m["level"] is None:
        return "typed, no joins", False
    if not m["clean"]:
        return f"FAILED: declared {level}, measured not clean", True
    if m["level"] != level:
        return f"FAILED: declared {level}, measured {m['level']}", True
    return "typed, agrees", False


def fmt(value, spec: str) -> str:
    return "?" if value is None else format(value, spec)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("roots", nargs="+", type=Path)
    parser.add_argument("--require-all", action="store_true",
                        help="fail when a class the overlay types has no row in the input")
    args = parser.parse_args(argv)
    try:
        exports = discover_exports(args.roots)
    except ValueError as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        return 2
    declared = declared_types()
    stats: dict[str, ClassStats] = {}
    unjoined = Counter()
    rows = sum(read_export(export, stats, unjoined) for export in exports)
    builds = set().union(*(s.builds for s in stats.values())) if stats else set()
    print(f"exports {len(exports)}, builds {len(builds)}, {FIELD} rows {rows:,}, "
          f"classes {len(stats)}")
    if not rows:
        print(f"FAILED: no {FIELD} row", file=sys.stderr)
        return 2
    print("class | rows | typed | joins (builds) | opens unjoined | unscaled | ratio p1/p50/p99 | "
          "worst | speed (pairs) | rotator byte/short/either/neither | checksums | verdict | group")
    failed = 0
    for group in sorted(stats):
        s, m = stats[group], measure(stats[group])
        text, fails = verdict(declared.get(group), m, s)
        failed += fails
        ratio = "/".join(fmt(r, ".4f") for r in m["ratio"]) if m["ratio"] else "?"
        rot = "/".join(str(s.rotator[k]) for k in (*WIDTHS.values(), "either", "neither"))
        sums = ",".join(f"{c}" for c, _n in s.checksums.most_common())
        print(f"{group.rsplit('.', 1)[-1]} | {s.rows} | {s.typed} | {len(s.joins)} "
              f"({len(s.join_builds)}) | {unjoined[group]} | {s.unscaled} | {ratio} | "
              f"{fmt(m['worst'], '.4f')} | {fmt(m['speed'], '.2f')} ({len(s.steps)}) | {rot} | "
              f"{sums} | {text} | {group}")
    missing = sorted(g for g, d in declared.items() if isinstance(d, tuple) and g not in stats)
    print(f"typed classes with no row: {len(missing)}; failed: {failed}")
    if args.require_all and missing:
        print(f"FAILED: --require-all, {len(missing)} typed classes with no row: "
              + ", ".join(g.rsplit(".", 1)[-1] for g in missing), file=sys.stderr)
        return 1
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
