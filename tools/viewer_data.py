#!/usr/bin/env python3
"""Data assembly for the 2D replay viewer.

Playback and measurement are deliberately separate passes. Movement arrives at
125 Hz (median 8 ms gap); playback renders at 20 Hz because nothing needs more.
But a teleport can fall between two rendered frames, so every check in this
module reads the FULL-RATE stream. A viewer that downsampled before measuring
would look correct while hiding the defect it exists to find.
"""
from __future__ import annotations

from typing import NamedTuple

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
