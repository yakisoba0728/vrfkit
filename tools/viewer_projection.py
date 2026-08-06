#!/usr/bin/env python3
"""World-to-minimap projection for the 2D replay viewer.

The transform is not in the replay. It is published per map by
valorant-api.com and joined on `manifest.level_names_and_times[0].name`.

The axes cross:

    u = pos_y * xMultiplier + xScalarToAdd
    v = pos_x * yMultiplier + yScalarToAdd

Of the four sign/order variants only this one holds up. It puts 100% of live
positions inside the unit square on eleven of twelve maps, while feeding
`pos_x` to `u` collapses to 0.9% on Haven and 3.1% on Fracture. Measured over
12 maps, 69 replays, 121,672,885 live movement rows on build 13.02.
"""
from __future__ import annotations

from typing import NamedTuple

# Hidden actors are parked far outside the map. BOTH axes must qualify:
# filtering on z alone misclassifies a real player falling off Abyss, which
# has no floor.
PARK_LIMIT = -40000.0


class MapConstants(NamedTuple):
    """One map's published minimap transform."""

    map_url: str
    x_multiplier: float
    y_multiplier: float
    x_scalar: float
    y_scalar: float
    display_icon_url: str


def is_parked(pos_x: float, pos_z: float) -> bool:
    """True when the actor is in the engine's hidden-actor park slot."""
    return pos_x < PARK_LIMIT and pos_z < PARK_LIMIT


def project(pos_x: float, pos_y: float, k: MapConstants) -> tuple[float, float]:
    """World centimetres to normalised minimap coordinates. Axes cross."""
    return (
        pos_y * k.x_multiplier + k.x_scalar,
        pos_x * k.y_multiplier + k.y_scalar,
    )
