#!/usr/bin/env python3
"""Data assembly for the 2D replay viewer.

Playback and measurement are deliberately separate passes. Movement arrives at
125 Hz (median 8 ms gap); playback renders at 20 Hz because nothing needs more.
But a teleport can fall between two rendered frames, so every check in this
module reads the FULL-RATE stream. A viewer that downsampled before measuring
would look correct while hiding the defect it exists to find.
"""
from __future__ import annotations

import json
import math
import re
from pathlib import Path
from typing import NamedTuple

import pyarrow.parquet as pq

import extract_active_effects
import viewer_projection as vp

PLAYBACK_HZ = 20


class Round(NamedTuple):
    """One round, half-open: `start_ms` inclusive, `end_ms` exclusive."""

    index: int
    start_ms: int
    end_ms: int


def rounds_from_events(times, groups, match_end_ms: int) -> list[Round]:
    """Round boundaries from `events.roundStarted`.

    A replay with no round events becomes one round covering the match, so the
    viewer stays usable; the caller reports the round count, which is what
    makes an empty set visible.
    """
    starts = sorted(t for t, g in zip(times, groups) if g == "roundStarted")
    if not starts:
        return [Round(0, 0, match_end_ms)]
    bounds = starts + [match_end_ms]
    return [Round(i, bounds[i], bounds[i + 1]) for i in range(len(starts))]


def downsample(samples, hz: int = PLAYBACK_HZ):
    """Keep the first sample in each 1/hz bucket. PLAYBACK ONLY.

    Never feed the result to a check. See the module docstring.
    """
    if not samples:
        return []
    step = 1000 // hz
    kept = []
    next_at = None
    for sample in samples:
        t = sample[0]
        if next_at is None or t >= next_at:
            kept.append(sample)
            next_at = t + step
    return kept


# Measured from the reference replay's own distribution rather than invented:
# p90 of inter-sample speed is 659 cm/s, which matches VALORANT's 675 cm/s run
# speed. 3000 is 4.4x that -- above Jett's dash and Raze's satchel (roughly
# 1600-1800 cm/s) and below a real teleport. It leaves 391 of 1,773,814
# samples, 0.022%.
TELEPORT_CM_PER_S = 3000.0

# hypot(x1-x0, y1-y0) above is blind to a pure-vertical displacement -- two
# rows with identical x/y report zero horizontal speed no matter the z gap --
# so it needs a companion check, not a fold-in. Folding z into one distance
# is not safe either way: Abyss has no floor, players genuinely fall off it,
# and every real fall would then false-positive as a teleport.
#
# There is no measured vertical-speed distribution to calibrate against the
# way TELEPORT_CM_PER_S was (that used a real p90 over 1.7M samples). Reusing
# the same 3000 cm/s bound is a deliberate, conservative stand-in: legitimate
# vertical impulses (jump apex, Jett updraft, Raze satchel/boombot launch)
# are the same order of magnitude as the horizontal ability bursts the
# horizontal bound already clears (roughly 1600-1800 cm/s), so 3000 should
# clear them too. It is a placeholder for real calibration, not a measurement,
# and is flagged as such rather than presented as one.
VERTICAL_TELEPORT_CM_PER_S = 3000.0

# A round start moves every player to spawn. That is a teleport in the data and
# not a defect, so displacements this soon after a round boundary are excused.
RESPAWN_GRACE_MS = 3000

DEATH_POSITION_WINDOW_MS = 2000

CHECK_KINDS = (
    "teleport",
    "vertical_teleport",
    "off_map",
    "parked",
    "absent_player",
    "death_without_position",
    "unknown_guid",
    "effect_off_map",
    "unexplained_heal",
)


class Finding(NamedTuple):
    """One anomaly, seekable from the page."""

    kind: str
    time_ms: int
    subject: str
    detail: str


def _in_unit_square(u: float, v: float) -> bool:
    return 0.0 <= u <= 1.0 and 0.0 <= v <= 1.0


def run_checks(context: dict) -> tuple[list[Finding], dict[str, int]]:
    """Every check, over the FULL-RATE movement stream.

    Returns the findings and a per-kind count that always carries all eight
    keys, zeros included. A count that only appears when non-zero cannot
    distinguish a clean replay from a check that stopped running.
    """
    findings: list[Finding] = []
    counts = {kind: 0 for kind in CHECK_KINDS}

    def report(kind, time_ms, subject, detail):
        findings.append(Finding(kind, time_ms, subject, detail))
        counts[kind] += 1

    movement = context["movement"]
    rounds = context["rounds"]
    players = context["players"]
    pawns = context["pawn_classes"]
    k = context["constants"]
    # Respawn grace is owed only at an actual round TRANSITION: a jump needs a
    # position carried over from a previous round to be "excused" as a
    # respawn. The earliest round has no previous round -- when it comes from
    # rounds_from_events' no-events fallback it is not even a real boundary,
    # just Round(0, 0, match_end_ms) invented to keep the viewer usable -- so
    # granting it the same grace would blind the check for the first
    # RESPAWN_GRACE_MS of the recording. Excluded on purpose.
    respawn_boundaries = sorted(r.start_ms for r in rounds)[1:]

    live: dict[int, list[tuple]] = {}
    for time_ms, guid, x, y, z, vel_z in movement:
        if vp.is_parked(x, z):
            report("parked", time_ms, str(guid), "hidden-actor park slot")
            continue
        if guid not in players and guid not in pawns:
            report("unknown_guid", time_ms, str(guid),
                   "not a manifest player and not a known pawn class")
        u, v = vp.project(x, y, k)
        if not _in_unit_square(u, v):
            report("off_map", time_ms, str(guid), f"projects to ({u:.3f}, {v:.3f})")
        live.setdefault(guid, []).append((time_ms, x, y, z, vel_z))

    for guid, rows in live.items():
        rows.sort()
        for (t0, x0, y0, z0, vz0), (t1, x1, y1, z1, vz1) in zip(rows, rows[1:]):
            dt = t1 - t0
            if dt <= 0:
                continue
            in_grace = any(0 <= t1 - s <= RESPAWN_GRACE_MS for s in respawn_boundaries)

            speed = math.hypot(x1 - x0, y1 - y0) / dt * 1000.0
            if speed > TELEPORT_CM_PER_S and not in_grace:
                report("teleport", t1, players.get(guid, str(guid)),
                       f"{speed:.0f} cm/s over {dt} ms")

            # A downward move with negative vel_z at arrival is a fall, not a
            # defect (see VERTICAL_TELEPORT_CM_PER_S above). Everything else
            # that crosses the bound -- upward, or downward with vel_z >= 0,
            # which no genuine fall produces -- is the anomaly.
            vspeed = abs(z1 - z0) / dt * 1000.0
            falling = z1 < z0 and vz1 < 0
            if vspeed > VERTICAL_TELEPORT_CM_PER_S and not falling and not in_grace:
                report("vertical_teleport", t1, players.get(guid, str(guid)),
                       f"{vspeed:.0f} cm/s vertical over {dt} ms")

    for guid, label in players.items():
        for r in rounds:
            if not any(r.start_ms <= t < r.end_ms for t, *_ in live.get(guid, [])):
                report("absent_player", r.start_ms, label,
                       f"no movement rows in round {r.index}")

    for time_ms, victim in context["deaths"]:
        near = [t for t, *_ in live.get(victim, [])
                if abs(t - time_ms) <= DEATH_POSITION_WINDOW_MS]
        if not near:
            report("death_without_position", time_ms, str(victim),
                   "no movement row within 2 s of the death")

    for effect in context["effects"]:
        u, v = vp.project(effect["spawn_x"], effect["spawn_y"], k)
        if not _in_unit_square(u, v):
            report("effect_off_map", effect["open_ms"], effect["effect_type"],
                   f"spawn projects to ({u:.3f}, {v:.3f})")

    # Grouped by (guid, section, instance), NOT (guid, section): a section
    # CLASS like "AttachedDamageSection" is reused across every armour item a
    # player ever owns, so grouping by class alone conflates an old instance
    # destroyed in combat with a fresh purchase that replaced it -- "old
    # armour ended at 0, new armour started at 23" is not a rise, it is two
    # different physical components sharing one class name. `instance` (the
    # raw ChangedComponent NetGUID) tells them apart. Section still matters
    # on top of that: one damage call can report several sections (e.g. the
    # always-zero ShieldDamageSection alongside the real HealthDamageSection
    # reading) in the same millisecond, and comparing across sections as if
    # they were one quantity misreads "shield 0, health 61" as a 0 -> 61
    # rise. See health_series and docs/DATA.md, "Health is absolute, not a
    # subtraction". Respawn grace mirrors the teleport checks above: every
    # player's health resets to full within a millisecond of every round
    # start, and that reset is not a defect any more than the position reset
    # at the same moment is -- but a reset can also happen mid-round (e.g. a
    # revive), which is a genuine within-instance rise this check should
    # still surface; grace only excuses rises near a round boundary.
    by_player_section_instance: dict[tuple[int, str, object], list[tuple]] = {}
    for time_ms, guid, life, is_heal, section, instance in context["health"]:
        by_player_section_instance.setdefault(
            (guid, section, instance), []).append((time_ms, life, is_heal))
    for (guid, section, instance), rows in by_player_section_instance.items():
        # Stable sort keyed on time_ms ALONE, not the bare tuple: sorting the
        # bare (time_ms, life, is_heal) tuple breaks a time_ms tie on life
        # ascending, which is wrong -- several hits landing in the same
        # millisecond are already in true call order from health_series
        # (packet_id, a global wire sequence number), and re-sorting by
        # value here would undo that and misread a real, decreasing damage
        # sequence as an ascending "staircase" of unexplained rises.
        rows.sort(key=lambda r: r[0])
        for (t0, l0, _), (t1, l1, heal1) in zip(rows, rows[1:]):
            in_grace = any(0 <= t1 - s <= RESPAWN_GRACE_MS for s in respawn_boundaries)
            if l1 > l0 and not heal1 and not in_grace:
                report("unexplained_heal", t1, players.get(guid, str(guid)),
                       f"{section}: {l0:.0f} -> {l1:.0f} with no heal RPC")

    findings.sort(key=lambda f: (f.time_ms, f.kind))
    return findings, counts


# Post-death spectator cameras replicate movement like a character does. They
# are not positions: drawing one renders a dead player apparently walking
# around the map. Classified apart so the page can default the layer off.
POSTDEATH_MARKER = "_PostDeath_"


def classify_guid(guid: int, players: dict, actor_classes: dict) -> str:
    """One of "player", "pawn", "postdeath", "unknown"."""
    if guid in players:
        return "player"
    class_path = actor_classes.get(guid)
    if not class_path:
        return "unknown"
    if POSTDEATH_MARKER in class_path:
        return "postdeath"
    return "pawn"


def _column(table, name):
    return table.column(name).to_pylist()


# Both spellings. `MulticastNotifyHeal` and `MulticastNotifyOverhealDecay` name
# their array `LifeChangeBySection`; the damage RPCs name it
# `LifeChangeEvents`. Filtering on one alone silently drops more than half the
# calls, which is how this was found (docs/DATA.md, "Health is absolute, not
# a subtraction"). One array element carries several sibling members --
# LifeResult and ChangedComponent both matched here, by the same
# fn/array/index prefix, so a LifeResult row can be paired with the
# ChangedComponent row for the SAME element.
_ARRAY_ELEMENT_RE = re.compile(
    r"^(?P<fn>[A-Za-z_]+)\.(?P<arr>LifeChangeEvents|LifeChangeBySection)"
    r"\[(?P<idx>\d+)\]\.(?P<member>LifeResult|ChangedComponent)$")
HEAL_FUNCTIONS = ("MulticastNotifyHeal", "MulticastNotifyOverhealDecay")

# A ChangedComponent that cannot be resolved to a section -- absent from
# net_guids.parquet, net_guids.parquet itself absent, or no ChangedComponent
# row decoded for that element -- is labelled with this sentinel rather than
# dropped or folded into another section. Dropping would silently shrink the
# series with no visible trace; folding it into, say, "health" would let an
# unresolved reading masquerade as a real one. Neither is acceptable given
# this module's own pattern elsewhere (parked rows, unknown movement GUIDs)
# of surfacing an anomaly rather than hiding it.
UNKNOWN_SECTION = "unknown"


def health_series(export_dir: Path, players: dict) -> list[tuple]:
    """`(time_ms, guid, life_result, is_heal, section, instance)` for the
    manifest players.

    `LifeResult` is the ABSOLUTE value after the change, so nothing has to be
    accumulated (docs/DATA.md). `section` is the element's `ChangedComponent`
    resolved through net_guids.parquet to its component identity --
    "HealthDamageSection", "AttachedDamageSection" (real armour),
    "ShieldDamageSection" (an always-zero decoy shell, docs/DATA.md
    convention 2), or others (OverhealDamageSection, ability-specific
    sections) -- reported as-is rather than normalized into a fixed enum,
    since docs/DATA.md is explicit that component names move with the game
    and inventing a mapping here would need the same upkeep.

    `instance` is the raw `ChangedComponent` NetGUID itself -- the SAME value
    `section` was resolved from, not a separate lookup -- or None when it
    could not be read (see UNKNOWN_SECTION). A section CLASS like
    "AttachedDamageSection" is reused across every armour item a player ever
    owns: an old instance destroyed in one round and a fresh purchase next
    round are different physical components sharing one class name, and
    comparing across that reuse misreads "old armour ended at 0, new armour
    started at 23" as a rise. A caller must split per (player, section,
    instance) -- not (player, section) alone -- before comparing values over
    time: one damage call can also report several sections for the SAME
    actor in the SAME element burst (e.g. shield then health), and treating
    those as one flattened series makes an always-zero shield reading look
    like a "rise" into the real health value that landed beside it. See
    `docs/DATA.md`, "Health is absolute, not a subtraction".

    Grouped by (actor_net_guid, packet_id, channel_index, fn, array, index),
    not by time_ms: measured on the real export, the SAME function can be
    invoked twice in the SAME millisecond, each call reusing array index 0
    (docs/DATA.md's round-reset broadcast fires once per section, and both
    calls can land in one tick) -- a time_ms-keyed join silently drops or
    corrupts one of the two readings. packet_id + channel_index identify the
    one wire call an element belongs to.

    net_guids.parquet is optional, matching fields.parquet's own optionality
    in `read_export`: if it is missing, every section resolves to
    `UNKNOWN_SECTION`.
    """
    fields_path = export_dir / "fields.parquet"
    table = pq.read_table(
        fields_path, columns=["time_ms", "actor_net_guid", "packet_id",
                               "channel_index", "field_name", "value_f64", "value_i64"])

    net_guids_path = export_dir / "net_guids.parquet"
    section_of: dict[int, str] = {}
    if net_guids_path.is_file():
        ng = pq.read_table(net_guids_path, columns=["net_guid", "path"])
        section_of = dict(zip(_column(ng, "net_guid"), _column(ng, "path")))

    life_by_key: dict[tuple, tuple[int, int, str, float]] = {}
    changed_component_by_key: dict[tuple, int] = {}

    for time_ms, guid, packet_id, channel, name, life, changed_component in zip(
        _column(table, "time_ms"), _column(table, "actor_net_guid"),
        _column(table, "packet_id"), _column(table, "channel_index"),
        _column(table, "field_name"), _column(table, "value_f64"),
        _column(table, "value_i64"),
    ):
        if guid not in players or not name:
            continue
        matched = _ARRAY_ELEMENT_RE.match(name)
        if not matched:
            continue
        key = (guid, packet_id, channel, matched.group("fn"),
               matched.group("arr"), matched.group("idx"))
        if matched.group("member") == "LifeResult":
            if life is not None:
                life_by_key[key] = (time_ms, guid, matched.group("fn"), life)
        elif changed_component is not None:
            changed_component_by_key[key] = changed_component

    # Sorted by (time_ms, packet_id), NOT plain tuple order: packet_id is a
    # global, monotonically increasing wire sequence number, so it is the
    # true call order within a tied millisecond. Plain tuple order would
    # break a time_ms tie on the next field instead (life_result, ascending)
    # -- measured on the real export: several separate hits landing in the
    # SAME millisecond (e.g. automatic-weapon fire) produce several calls to
    # the same section, and sorting those by value re-labels a real
    # DECREASING health sequence as an ascending "staircase", which then
    # misreads as consecutive unexplained rises. packet_id is dropped from
    # the returned tuple; only its ordering effect is kept.
    ordered = []
    for key, (time_ms, guid, fn, life) in life_by_key.items():
        changed_component = changed_component_by_key.get(key)
        section = (section_of.get(changed_component) or UNKNOWN_SECTION
                   if changed_component is not None else UNKNOWN_SECTION)
        packet_id = key[1]
        ordered.append((time_ms, packet_id, guid, life, fn in HEAL_FUNCTIONS,
                         section, changed_component))
    ordered.sort(key=lambda r: r[:2])
    return [(time_ms, guid, life, is_heal, section, instance)
            for time_ms, packet_id, guid, life, is_heal, section, instance in ordered]


_REQUIRED_EXPORT_FILES = ("movement.parquet", "actors.parquet", "events.parquet", "manifest.json")


def read_export(export_dir: Path) -> dict:
    """Everything the viewer needs, read from one export directory.

    A missing table fails by name rather than yielding an empty layer that
    would look like a quiet match.
    """
    for name in _REQUIRED_EXPORT_FILES:
        path = export_dir / name
        if not path.is_file():
            raise SystemExit(f"{path} is missing; run `vrfkit export` first")

    manifest = json.loads((export_dir / "manifest.json").read_text(encoding="utf-8"))
    players = {p["character_net_guid"]: p["subject"][:8]
               for p in manifest.get("players") or []
               if p.get("character_net_guid") is not None}

    actors = pq.read_table(export_dir / "actors.parquet",
                           columns=["actor_net_guid", "class_path"])
    actor_classes = {}
    for guid, path in zip(_column(actors, "actor_net_guid"), _column(actors, "class_path")):
        # class_path is nullable (actors.rs writes null when the GUID cache had
        # no mapping for that row, e.g. an orphan close for an actor opened
        # before the export window). NetGUIDs are recycled across rounds, so
        # the same numeric GUID can later carry a real, resolvable open. A
        # plain setdefault would pin the first row's null forever and a real
        # pawn or camera would classify as "unknown" for the whole match.
        if path:
            actor_classes.setdefault(guid, path)

    mv = pq.read_table(export_dir / "movement.parquet",
                       columns=["time_ms", "character_net_guid",
                                "pos_x", "pos_y", "pos_z", "vel_z", "yaw"])
    movement = list(zip(_column(mv, "time_ms"), _column(mv, "character_net_guid"),
                        _column(mv, "pos_x"), _column(mv, "pos_y"),
                        _column(mv, "pos_z"), _column(mv, "vel_z")))
    yaws = _column(mv, "yaw")

    ev = pq.read_table(export_dir / "events.parquet",
                       columns=["time1", "group", "word0", "word1"])
    times, groups = _column(ev, "time1"), _column(ev, "group")
    word0s, word1s = _column(ev, "word0"), _column(ev, "word1")
    # characterDeath carries (word0, word1) = (killer, killed) NetGUID (see
    # vrf-export/src/record.rs and docs/USAGE.md); word1 is the victim.
    deaths = [(t, w) for t, g, w in zip(times, groups, word1s) if g == "characterDeath"]

    # The last timestamp seen anywhere in the export, not manifest duration_ms:
    # if the manifest's stated duration outran the last real movement/event
    # row, rounds_from_events would stretch the final round into a stretch
    # with no data in it, and every player would report "absent" for time that
    # was never recorded. The +1 keeps that last sample inside the half-open
    # Round it belongs to; the trailing +[0] keeps max() from raising on an
    # empty export.
    match_end_ms = max([t for t, *_ in movement] + times + [0]) + 1
    rounds = rounds_from_events(times, groups, match_end_ms)
    effects, effect_tally = extract_active_effects.build_with_tally(export_dir)

    pawn_classes = {g: c for g, c in actor_classes.items()
                    if classify_guid(g, players, actor_classes) in ("pawn", "postdeath")}

    return {
        "manifest": manifest,
        "movement": movement,
        "yaws": yaws,
        "rounds": rounds,
        "players": players,
        "actor_classes": actor_classes,
        "pawn_classes": pawn_classes,
        "deaths": deaths,
        "events": list(zip(times, groups, word0s, word1s)),
        "effects": effects,
        "effect_tally": effect_tally,
        "health": (health_series(export_dir, players)
                   if (export_dir / "fields.parquet").is_file() else []),
        "match_end_ms": match_end_ms,
    }
