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

import json
import urllib.request
from pathlib import Path
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


MAPS_API = "https://valorant-api.com/v1/maps"


class ConstantsUnavailable(SystemExit):
    """The map transform could not be obtained, so nothing can be projected."""


def _fetch(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=30) as response:
        return response.read()


def load_constants(map_url: str, cache_dir: Path, fetch=None) -> MapConstants:
    """The published transform for `map_url`, fetched once and cached.

    Raises `ConstantsUnavailable` rather than returning a default. A guessed
    scale would render a plausible wrong picture, and a wrong picture in a
    verification instrument is worse than a missing one.
    """
    fetch = fetch or _fetch
    cache_dir.mkdir(parents=True, exist_ok=True)
    cached = cache_dir / "maps.json"
    if not cached.is_file():
        try:
            cached.write_bytes(fetch(MAPS_API))
        except Exception as error:
            raise ConstantsUnavailable(
                f"could not fetch {MAPS_API}: {error}\n"
                f"the minimap transform is not in the replay; without it "
                f"nothing can be projected"
            ) from error
    published = json.loads(cached.read_text(encoding="utf-8"))
    for entry in published.get("data") or []:
        if entry.get("mapUrl") == map_url:
            return MapConstants(
                map_url=map_url,
                x_multiplier=entry["xMultiplier"],
                y_multiplier=entry["yMultiplier"],
                x_scalar=entry["xScalarToAdd"],
                y_scalar=entry["yScalarToAdd"],
                display_icon_url=entry["displayIcon"],
            )
    raise ConstantsUnavailable(f"no published transform for map {map_url}")


def load_minimap_png(k: MapConstants, cache_dir: Path, fetch=None) -> bytes:
    """The map's minimap image, fetched once and cached beside the constants."""
    fetch = fetch or _fetch
    cache_dir.mkdir(parents=True, exist_ok=True)
    name = k.map_url.strip("/").replace("/", "_") + ".png"
    cached = cache_dir / name
    if not cached.is_file():
        try:
            cached.write_bytes(fetch(k.display_icon_url))
        except Exception as error:
            raise ConstantsUnavailable(
                f"could not fetch minimap {k.display_icon_url}: {error}"
            ) from error
    return cached.read_bytes()
