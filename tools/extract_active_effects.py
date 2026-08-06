#!/usr/bin/env python3
"""Derive a table of persistent ability effects from an export.

VALORANT's persistent abilities -- smokes, walls, slows, traps, molly/decay
zones, recon bolts, ult orbs -- spawn an actor with a known class, a spawn
location, and an open/close lifetime. `actors.parquet` already carries all of
that; this script filters the effect actors out, pairs each actor's open and
close events into a lifetime, classifies the effect, and writes one row per
effect instance.

This is a *derived* view over the raw export, not a new wire decode: the data
is already in `actors.parquet` (class + spawn xyz + open/close). vrfkit itself
exports raw tables; analytical joins live here so the parser stays focused.

Position note: `spawn_x/y/z` are the actor's spawn transform, which for a
placed effect (smoke, wall segment, trap) is its world location. For the few
effects that relocate, `fields.parquet` carries the live `ReplicatedMovement`
(quantized x100) or `MulticastAddSmokeScreenPoint.Translation`.

Usage:
    python tools/extract_active_effects.py --export <out_dir> --out active_effects.parquet
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

# Substrings that mark a class as a persistent ability effect. Matched
# case-insensitively against the full class_path. The list is intentionally
# broad: a missed effect simply does not appear, and a false positive is a
# short-lived actor whose open/close still reads sensibly.
EFFECT_KEYWORDS = (
    "smoke",
    "smokezone",
    "wall",
    "barrier",
    "toxicscreen",
    "slowfield",
    "slow",
    "trap",
    "cage",
    "molotov",
    "fire",
    "decayexplosion",
    "decaynade",
    "orb",
    "drone",
    "scout",
    "recon",
    "nanoswarm",
    "alarmbot",
)

# Map a class to a coarse effect family. Order matters: check the more
# specific tokens first so "SlowField" is a slow, not a field.
def classify(class_path: str) -> str:
    c = class_path.lower()
    if "smoke" in c or "smokezone" in c:
        return "smoke"
    if "wall" in c or "barrier" in c or "toxicscreen" in c:
        return "wall"
    if "slow" in c:
        return "slow"
    if "trap" in c or "cage" in c:
        return "trap"
    if "molotov" in c or "fire" in c or "decay" in c or "nanoswarm" in c:
        return "damage_zone"
    if "orb" in c:
        return "orb"
    if "drone" in c or "scout" in c or "recon" in c or "alarmbot" in c:
        return "recon"
    return "other"


# Internal agent codename, when the class lives under /Game/Characters/<name>/.
# These are VALORANT's internal names (Smonk = Brimstone, Pandemic = Viper,
# ...); they are left as-is rather than mapped to display names, which
# `equippable_table.py` already owns.
AGENT_RE = re.compile(r"/Game/Characters/(\w+)/")


def agent_codename(class_path: str) -> str:
    m = AGENT_RE.search(class_path)
    return m.group(1) if m else ""


def is_effect_class(class_path: str) -> bool:
    if not class_path:
        return False
    c = class_path.lower()
    if not any(k in c for k in EFFECT_KEYWORDS):
        return False
    # Exclude ability *controllers*: classes whose leaf starts with "Ability_"
    # are the ability actor itself (or a post-death variant), which lives across
    # the whole match. The transient effect instance is the GameObject_ /
    # Projectile_ / Patch_ actor it spawns, and that is what we want here.
    leaf = class_path.rsplit("/", 1)[-1].lower()
    if leaf.startswith("ability_"):
        return False
    return True


def build(out_dir: Path) -> list[dict]:
    """The effect instances alone. See [`build_with_tally`] for what was lost."""
    return build_with_tally(out_dir)[0]


def build_with_tally(out_dir: Path) -> tuple[list[dict], dict]:
    actors_path = out_dir / "actors.parquet"
    if not actors_path.exists():
        raise SystemExit(f"no actors.parquet in {out_dir} -- run `vrfkit export` first")

    table = pq.read_table(actors_path)
    cols = {c: table.column(c).to_pylist() for c in table.column_names}
    guid = cols["actor_net_guid"]
    event = cols["event"]
    time_ms = cols["time_ms"]
    class_path = cols["class_path"]
    sx = cols["spawn_x"]
    sy = cols["spawn_y"]
    sz = cols["spawn_z"]

    # Actor NetGUIDs are recycled across rounds, so the same GUID can carry
    # several open/close lifetimes. Collect events per GUID, then pair each
    # open with the close that follows it -- not first-open to last-close,
    # which would span unrelated rounds and report absurd durations.
    events: dict[int, list[tuple]] = {}
    for i in range(len(guid)):
        cp = class_path[i]
        if not is_effect_class(cp):
            continue
        events.setdefault(guid[i], []).append(
            (time_ms[i], event[i], sx[i], sy[i], sz[i], cp)
        )

    tally = {"went_dormant": 0}
    rows: list[dict] = []
    for g, evs in events.items():
        evs.sort(key=lambda e: e[0])
        pending = None  # (open_ms, sx, sy, sz, class_path) of the current open instance
        for t, ev, x, y, z, cp in evs:
            if ev == "open":
                if pending is not None:
                    # Reopened before closing: the prior instance never closed
                    # in this export. Emit it open-ended so it is not lost.
                    rows.append(_row(g, pending, None))
                pending = (t, x, y, z, cp)
            elif ev == "close":
                if pending is not None:
                    rows.append(_row(g, pending, t))
                    pending = None
                # A close with no pending open is an orphan (actor opened before
                # the export window); drop it rather than invent an open time.
            elif ev == "dormant":
                # Dormancy is NOT destruction. The actor stopped replicating --
                # which for a settled smoke or wall is its normal steady state --
                # so ending the instance here would make persistent effects
                # vanish early in anything built on this table. The instance
                # stays pending and, absent a later close, ends up open-ended.
                #
                # That is also what the code did before `dormant` existed as a
                # value, purely because `elif ev == "close"` did not match it.
                # The behaviour was right and unstated, which is the same shape
                # as the bug this whole pass was fixing: an open-ended row
                # because the actor went dormant and an open-ended row because
                # the export window ended are indistinguishable in the table.
                # Hence the tally.
                tally["went_dormant"] += 1
        if pending is not None:
            rows.append(_row(g, pending, None))

    rows.sort(key=lambda r: (r["open_ms"] if r["open_ms"] is not None else -1, r["actor_net_guid"]))
    return rows, tally


def _row(guid: int, open_rec: tuple, close_ms):
    open_ms, x, y, z, cp = open_rec
    return {
        "actor_net_guid": guid,
        "class_path": cp,
        "effect_type": classify(cp),
        "agent": agent_codename(cp),
        "spawn_x": x,
        "spawn_y": y,
        "spawn_z": z,
        "open_ms": open_ms,
        "close_ms": close_ms,
        "duration_ms": (close_ms - open_ms) if close_ms is not None else None,
    }


SCHEMA = pa.schema([
    pa.field("actor_net_guid", pa.int32()),
    pa.field("class_path", pa.string()),
    pa.field("effect_type", pa.string()),
    pa.field("agent", pa.string()),
    pa.field("spawn_x", pa.float32()),
    pa.field("spawn_y", pa.float32()),
    pa.field("spawn_z", pa.float32()),
    pa.field("open_ms", pa.int64()),
    pa.field("close_ms", pa.int64()),
    pa.field("duration_ms", pa.int64()),
])


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--export", type=Path, required=True,
                    help="directory written by `vrfkit export` (must hold actors.parquet)")
    ap.add_argument("--out", type=Path, required=True,
                    help="output active_effects.parquet path")
    args = ap.parse_args()

    rows, tally = build_with_tally(args.export)
    cols = {name: [r[name] for r in rows] for name in SCHEMA.names}
    table = pa.Table.from_pydict(cols, schema=SCHEMA)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    pq.write_table(table, args.out, compression="zstd")

    from collections import Counter
    by_type = Counter(r["effect_type"] for r in rows)
    print(f"wrote {args.out} ({len(rows)} effect instances)")
    for t, n in sorted(by_type.items()):
        print(f"  {t:12s} {n}")
    # Printed with its zero. An open-ended row can mean "the actor went dormant"
    # or "the export window ended first", and the table cannot tell them apart.
    open_ended = sum(1 for r in rows if r["close_ms"] is None)
    print(f"  {'open-ended':12s} {open_ended} ({tally['went_dormant']} actor(s) went dormant)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
