#!/usr/bin/env python3
"""Data assembly for the 2D replay viewer.

Playback and measurement are deliberately separate passes. Movement arrives at
125 Hz (median 8 ms gap); playback renders at 20 Hz because nothing needs more.
But a teleport can fall between two rendered frames, so every check in this
module reads the FULL-RATE stream. A viewer that downsampled before measuring
would look correct while hiding the defect it exists to find.
"""
from __future__ import annotations

import math
from typing import NamedTuple

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

# A round start moves every player to spawn. That is a teleport in the data and
# not a defect, so displacements this soon after a round boundary are excused.
RESPAWN_GRACE_MS = 3000

DEATH_POSITION_WINDOW_MS = 2000

CHECK_KINDS = (
    "teleport",
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
    for time_ms, guid, x, y, z in movement:
        if vp.is_parked(x, z):
            report("parked", time_ms, str(guid), "hidden-actor park slot")
            continue
        if guid not in players and guid not in pawns:
            report("unknown_guid", time_ms, str(guid),
                   "not a manifest player and not a known pawn class")
        u, v = vp.project(x, y, k)
        if not _in_unit_square(u, v):
            report("off_map", time_ms, str(guid), f"projects to ({u:.3f}, {v:.3f})")
        live.setdefault(guid, []).append((time_ms, x, y))

    for guid, rows in live.items():
        rows.sort()
        for (t0, x0, y0), (t1, x1, y1) in zip(rows, rows[1:]):
            dt = t1 - t0
            if dt <= 0:
                continue
            speed = math.hypot(x1 - x0, y1 - y0) / dt * 1000.0
            if speed <= TELEPORT_CM_PER_S:
                continue
            if any(0 <= t1 - s <= RESPAWN_GRACE_MS for s in respawn_boundaries):
                continue
            report("teleport", t1, players.get(guid, str(guid)),
                   f"{speed:.0f} cm/s over {dt} ms")

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

    by_player: dict[int, list[tuple]] = {}
    for time_ms, guid, life, is_heal in context["health"]:
        by_player.setdefault(guid, []).append((time_ms, life, is_heal))
    for guid, rows in by_player.items():
        rows.sort()
        for (t0, l0, _), (t1, l1, heal1) in zip(rows, rows[1:]):
            if l1 > l0 and not heal1:
                report("unexplained_heal", t1, players.get(guid, str(guid)),
                       f"{l0:.0f} -> {l1:.0f} with no heal RPC")

    findings.sort(key=lambda f: (f.time_ms, f.kind))
    return findings, counts
