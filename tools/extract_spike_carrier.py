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

NetGUID note: `Owner` on `BombEquippable_C` has no entry in vrfkit's overlay
type table, so `value_i64` is null and only `raw_bits` survives; the group is
one of the bare ones the table does not reach. `unpack_netguid` below reads the
UE `SerializeIntPacked` encoding those bits carry. Typing the field in the
overlay table would make this function unnecessary and is the better fix, but
it moves the export baseline, so it is left as follow-up work.

Usage:
    python tools/extract_spike_carrier.py --export <out_dir> --out spike_carrier.parquet
"""

from __future__ import annotations

import argparse
import bisect
import json
from collections import Counter
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

BOMB_CLASS = "BombEquippable.BombEquippable_C"

#: Owner classes that mean "nobody is carrying it". GroundPickup is the actor
#: the spike lives inside while it sits on the floor; PickupProjectile is the
#: short arc between a drop and the landing. Both are matched on the leaf name.
LOOSE_CLASSES = ("EquippableGroundPickup_C", "EquippablePickupProjectile_C")


def unpack_netguid(raw: bytes) -> int | None:
    """Decode UE's `SerializeIntPacked`, which is how a NetGUID reaches the wire.

    Each byte carries 7 payload bits in its high bits and a "more follows" flag
    in bit 0; groups are little-endian. Returns None for an empty payload.

    Verified against 4,601 `Owner`/`Instigator`/`Controller`/`AttachParent`
    rows on groups where vrfkit does populate `value_i64` -- zero mismatches.
    """
    if not raw:
        return None
    value = 0
    shift = 0
    for byte in raw:
        value |= (byte >> 1) << shift
        shift += 7
        if not byte & 1:
            break
    return value


def leaf(class_path: str | None) -> str:
    """Last path segment of a class path, or "" when the class is unknown."""
    return class_path.rsplit("/", 1)[-1] if class_path else ""


def netguid_at(values: list, raws: list, i: int) -> int | None:
    """The NetGUID at row `i`, from the typed column when present."""
    return values[i] if values[i] is not None else unpack_netguid(raws[i] or b"")


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
    # the player that spawned it. Typed on pawn groups, so value_i64 is usually
    # populated; fall back to the packed bits otherwise.
    instigator: dict[int, int] = {}
    for i, name in enumerate(f["field_name"]):
        if name == "Instigator":
            g = netguid_at(f["value_i64"], f["raw_bits"], i)
            if g:
                instigator.setdefault(f["actor_net_guid"][i], g)

    # Every Owner write on a bomb channel, in time order: the custody log.
    owner_log: dict[int, list[tuple[int, int]]] = {}
    for i, grp in enumerate(f["group_path"]):
        if BOMB_CLASS in grp and f["field_name"][i] == "Owner":
            g = netguid_at(f["value_i64"], f["raw_bits"], i)
            if g is not None:
                owner_log.setdefault(f["actor_net_guid"][i], []).append(
                    (f["time_ms"][i], g))

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
            kind, carrier, proxy = "unknown", None, ""
            if owner in pawn_subject:
                kind, carrier = "player", owner
            elif any(k in leaf(cls) for k in LOOSE_CLASSES):
                kind = "loose"
            else:
                src = instigator.get(owner)
                if src in pawn_subject:
                    kind, carrier, proxy = "proxy", src, leaf(cls)

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

    held = [r for r in rows if r["holder_kind"] in ("player", "proxy")]
    print(f"  rounds {len({r['round_number'] for r in rows})}, "
          f"with a carrier {len({r['round_number'] for r in held})}, "
          f"bombs {len({r['bomb_net_guid'] for r in rows})}")

    # The carrier at plant time. `spikePlanted` carries no planter identity in
    # its payload, so this is the join answering rather than a check -- but a
    # plant with no carrier would mean the chain dropped something.
    for grp, t1 in zip(events.get("group", []), events.get("time1", [])):
        if grp != "spikePlanted":
            continue
        at = [r for r in held
              if r["from_ms"] <= t1 and (r["to_ms"] is None or t1 <= r["to_ms"])]
        who = at[-1] if at else None
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
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
