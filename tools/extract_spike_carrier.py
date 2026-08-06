#!/usr/bin/env python3
"""Derive a spike (bomb) custody timeline from an export.

Who was holding the spike, and when. The answer lives on the spike's own actor
channel: `BombEquippable_C.Owner` is re-replicated every time custody changes,
so a round reads as a sequence of owner intervals -- ground pickup, player,
dropped projectile, ground pickup, next player, and so on. This script pairs
consecutive `Owner` writes into intervals, resolves each owner NetGUID to a
class, and joins players through to their manifest `subject`.

This is a *derived* view over the raw export, not a new wire decode: every
value comes out of `fields.parquet` / `actors.parquet` / `manifest.json`.
vrfkit itself exports raw tables; analytical joins live here so the parser
stays focused.

Three signals overlap; `Owner` is the one used, the others qualify it:

  Owner                  the custody signal. A superset of the other two --
                         it covers "carrying it in the backpack", not just
                         "holding it".
  NewCharacter           `MulticastPlayBombPickedUpAudio`, fires only on a
                         pickup and always agrees with the `Owner` written in
                         the same tick. Not read here; it is the cross-check
                         that established `Owner`.
  NewCurrentEquippable   the `AresInventory` side, i.e. the spike is actually
                         *in hand*. Kept as the `in_hand` flag.

`Owner` is not always a player. VALORANT hands the spike to whatever actor is
physically holding it, which includes:

  EquippableGroundPickup_C     -- lying on the floor (round start, or dropped)
  EquippablePickupProjectile_C -- mid-air, between a drop and its landing
  Pawn_Aggrobot_SeekerNade_C   -- Gekko's Wingman, which really does carry and
                                  plant the spike

Rather than allowlisting proxy classes, an owner that is not a manifest
character is asked for its own `Instigator` -- Wingman's is the Gekko player --
and that is reported as `carrier_pawn_guid` with `via_proxy_class` set.

NetGUID note: `Owner` used to arrive untyped on this group, so an earlier
version of this script unpacked `SerializeIntPacked` out of `raw_bits` by hand.
The overlay now resolves `Owner`/`Instigator`/`AttachParent`/`Controller` by
name for any group, so `value_i64` is populated and that decoder is gone. An
export written before that change will not work here.

Usage:
    python tools/extract_spike_carrier.py --export <out_dir> --out spike_carrier.parquet
"""

from __future__ import annotations

import argparse
import bisect
import json
import sys
from collections import Counter
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

BOMB_CLASS = "BombEquippable.BombEquippable_C"

#: Holder kinds that mean somebody is actually carrying the spike.
HELD_KINDS = ("player", "proxy")

#: Owner classes that mean "nobody is carrying it". GroundPickup is the actor
#: the spike lives inside while it sits on the floor; PickupProjectile is the
#: short arc between a drop and the landing. Both are matched on the leaf name.
LOOSE_CLASSES = ("EquippableGroundPickup_C", "EquippablePickupProjectile_C")


def leaf(class_path: str | None) -> str:
    """Last path segment of a class path, or "" when the class is unknown."""
    return class_path.rsplit("/", 1)[-1] if class_path else ""


def classify_owner(owner: int, owner_class: str | None,
                   pawn_subject: dict, instigator: dict):
    """Who, if anyone, is carrying the spike for this `Owner` value.

    Returns `(kind, carrier_pawn_guid, via_proxy_class)`. A manifest character
    is the carrier itself; a ground pickup or drop projectile means nobody has
    it; anything else is asked for its own `Instigator`, which walks a proxy
    such as Gekko's Wingman back to the player that spawned it. An owner that
    is none of those stays `unknown` rather than being guessed at.
    """
    if owner in pawn_subject:
        return "player", owner, ""
    name = leaf(owner_class)
    if any(k in name for k in LOOSE_CLASSES):
        return "loose", None, ""
    source = instigator.get(owner)
    if source in pawn_subject:
        return "proxy", source, name
    return "unknown", None, ""


def carrier_at(held, t_ms: int):
    """The custody interval covering `t_ms`, or None if nobody held it then.

    `to_ms` is None for the last interval of a bomb whose actor never closed,
    and that interval runs to the end of the replay.
    """
    covering = [r for r in held
                if r["from_ms"] <= t_ms and (r["to_ms"] is None or t_ms <= r["to_ms"])]
    return covering[-1] if covering else None


def unresolved(rows, events) -> list[str]:
    """Everything this extraction failed to resolve, as readable lines.

    The command returned 0 whatever it could not answer. Two answers it cannot
    fail to have:

    - Custody at all. Zero rows writes a valid, empty Parquet, prints
      "0 custody intervals" and exits 0 -- indistinguishable, to anything
      reading `$?`, from a replay whose spike changed hands forty times.
    - A carrier for every plant. The module docstring already says what a
      `NO CARRIER` plant means: the chain between the `Owner` log and the
      `spikePlanted` event dropped something. It was printed as one more line
      of output.
    """
    problems = []
    if not rows:
        problems.append(
            "no custody intervals at all: no BombEquippable_C.Owner writes "
            "were found, so an empty table was written")
    held = [r for r in rows if r["holder_kind"] in HELD_KINDS]
    for group, t1 in zip(events.get("group", []), events.get("time1", [])):
        if group == "spikePlanted" and carrier_at(held, t1) is None:
            problems.append(
                f"plant at t={t1} resolves to NO CARRIER: no player or proxy "
                f"held the spike at that moment")
    return problems


def load(out_dir: Path):
    for name in ("fields.parquet", "actors.parquet", "manifest.json"):
        if not (out_dir / name).exists():
            raise SystemExit(f"no {name} in {out_dir} -- run `vrfkit export` first")

    fields = pq.read_table(out_dir / "fields.parquet")
    f = {c: fields.column(c).to_pylist() for c in
         ("time_ms", "actor_net_guid", "group_path", "field_name",
          "value_i64", "raw_bits")}

    actors = pq.read_table(out_dir / "actors.parquet")
    a = {c: actors.column(c).to_pylist() for c in
         ("time_ms", "actor_net_guid", "event", "class_path")}

    manifest = json.loads((out_dir / "manifest.json").read_text(encoding="utf-8"))

    events = {}
    ev_path = out_dir / "events.parquet"
    if ev_path.exists():
        et = pq.read_table(ev_path)
        events = {c: et.column(c).to_pylist() for c in ("group", "time1", "metadata")}
    return f, a, manifest, events


def build(out_dir: Path):
    f, a, manifest, events = load(out_dir)

    # actor_net_guid -> class_path. Dynamic actors (the spike, pawns) are not in
    # net_guids.parquet at all, so actors.parquet is the only resolver for them.
    # No GUID on the sample export carries two distinct class paths, so a flat
    # map is safe; a future export that recycles GUIDs would need time scoping.
    guid_class: dict[int, str] = {}
    for g, cp in zip(a["actor_net_guid"], a["class_path"]):
        if cp:
            guid_class.setdefault(g, cp)

    # Bomb actor lifetimes -- the close bounds the last custody interval.
    bomb_close: dict[int, int] = {}
    bombs: set[int] = set()
    for t, g, ev, cp in zip(a["time_ms"], a["actor_net_guid"],
                            a["event"], a["class_path"]):
        if cp and BOMB_CLASS in cp:
            bombs.add(g)
            if ev == "close":
                bomb_close.setdefault(g, t)

    pawn_subject = {p["character_net_guid"]: p["subject"]
                    for p in manifest.get("players", [])}

    # Round boundaries from the replay's own roundStarted events.
    round_starts: list[tuple[int, int]] = []
    for grp, t1, meta in zip(events.get("group", []), events.get("time1", []),
                             events.get("metadata", [])):
        if grp == "roundStarted":
            try:
                round_starts.append((t1, int(meta)))
            except (TypeError, ValueError):
                round_starts.append((t1, len(round_starts)))
    round_starts.sort()
    round_ts = [t for t, _ in round_starts]

    def round_of(ms: int):
        if not round_ts:
            return None
        i = bisect.bisect_right(round_ts, ms) - 1
        return round_starts[i][1] if i >= 0 else None

    # An actor's own Instigator, used to walk a proxy carrier (Wingman) back to
    # the player that spawned it.
    instigator: dict[int, int] = {}
    for i, name in enumerate(f["field_name"]):
        if name == "Instigator" and f["value_i64"][i]:
            instigator.setdefault(f["actor_net_guid"][i], f["value_i64"][i])

    # Every Owner write on a bomb channel, in time order: the custody log.
    owner_log: dict[int, list[tuple[int, int]]] = {}
    for i, grp in enumerate(f["group_path"]):
        if (BOMB_CLASS in grp and f["field_name"][i] == "Owner"
                and f["value_i64"][i] is not None):
            owner_log.setdefault(f["actor_net_guid"][i], []).append(
                (f["time_ms"][i], f["value_i64"][i]))

    # AresInventory side: (pawn, bomb) pairs seen in hand, with timestamps.
    in_hand: dict[tuple[int, int], list[int]] = {}
    for i, grp in enumerate(f["group_path"]):
        if (grp.endswith("AresInventory")
                and f["field_name"][i] in ("CurrentEquippable",
                                           "NewCurrentEquippable")
                and f["value_i64"][i] in bombs):
            in_hand.setdefault(
                (f["actor_net_guid"][i], f["value_i64"][i]), []
            ).append(f["time_ms"][i])

    rows: list[dict] = []
    for bomb, log in sorted(owner_log.items()):
        log.sort()
        for n, (t, owner) in enumerate(log):
            end = log[n + 1][0] if n + 1 < len(log) else bomb_close.get(bomb)
            cls = guid_class.get(owner)
            kind, carrier, proxy = classify_owner(
                owner, cls, pawn_subject, instigator)
            held = in_hand.get((owner, bomb), [])
            rows.append({
                "round_number": round_of(t),
                "bomb_net_guid": bomb,
                "from_ms": t,
                "to_ms": end,
                "duration_ms": (end - t) if end is not None else None,
                "owner_net_guid": owner,
                "owner_class": leaf(cls),
                "holder_kind": kind,
                "carrier_pawn_guid": carrier,
                "carrier_subject": pawn_subject.get(carrier, ""),
                "via_proxy_class": proxy,
                "in_hand": any(t <= h and (end is None or h <= end)
                               for h in held),
            })

    rows.sort(key=lambda r: (r["from_ms"], r["bomb_net_guid"]))
    return rows, events


SCHEMA = pa.schema([
    pa.field("round_number", pa.int32()),
    pa.field("bomb_net_guid", pa.int64()),
    pa.field("from_ms", pa.int64()),
    pa.field("to_ms", pa.int64()),
    pa.field("duration_ms", pa.int64()),
    pa.field("owner_net_guid", pa.int64()),
    pa.field("owner_class", pa.string()),
    pa.field("holder_kind", pa.string()),
    pa.field("carrier_pawn_guid", pa.int64()),
    pa.field("carrier_subject", pa.string()),
    pa.field("via_proxy_class", pa.string()),
    pa.field("in_hand", pa.bool_()),
])


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--export", type=Path, required=True,
                    help="directory written by `vrfkit export`")
    ap.add_argument("--out", type=Path, required=True,
                    help="output spike_carrier.parquet path")
    ap.add_argument("--print", dest="show", action="store_true",
                    help="also print the timeline, carried intervals only")
    args = ap.parse_args()

    rows, events = build(args.export)
    cols = {name: [r[name] for r in rows] for name in SCHEMA.names}
    table = pa.Table.from_pydict(cols, schema=SCHEMA)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    pq.write_table(table, args.out, compression="zstd")

    by_kind = Counter(r["holder_kind"] for r in rows)
    print(f"wrote {args.out} ({len(rows)} custody intervals)")
    for k, n in sorted(by_kind.items()):
        print(f"  {k:8s} {n}")

    held = [r for r in rows if r["holder_kind"] in HELD_KINDS]
    print(f"  rounds {len({r['round_number'] for r in rows})}, "
          f"with a carrier {len({r['round_number'] for r in held})}, "
          f"bombs {len({r['bomb_net_guid'] for r in rows})}")

    # The carrier at plant time. `spikePlanted` carries no planter identity in
    # its payload, so this is the join answering rather than a check -- but a
    # plant with no carrier means the chain dropped something, and `unresolved`
    # below turns that into a nonzero exit rather than a line of output.
    for grp, t1 in zip(events.get("group", []), events.get("time1", [])):
        if grp != "spikePlanted":
            continue
        who = carrier_at(held, t1)
        tag = "NO CARRIER" if who is None else (
            f"{who['carrier_subject'][:8]} pawn={who['carrier_pawn_guid']}"
            + (f" via {who['via_proxy_class']}" if who["via_proxy_class"] else ""))
        print(f"  plant t={t1:>8}  round {who['round_number'] if who else '?'}  {tag}")

    if args.show:
        print()
        for r in held:
            print("  r%-3s %8d-%-8s %-6s %-8s pawn=%-5s %s%s" % (
                r["round_number"], r["from_ms"], r["to_ms"], r["holder_kind"],
                r["carrier_subject"][:8], r["carrier_pawn_guid"],
                "in-hand " if r["in_hand"] else "",
                "" if r["duration_ms"] is None
                else f"({r['duration_ms'] / 1000:.1f}s)"))

    problems = unresolved(rows, events)
    if problems:
        print(f"\nFAILED: {len(problems)} thing(s) this export could not "
              f"resolve", file=sys.stderr)
        for line in problems:
            print(f"    {line}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
