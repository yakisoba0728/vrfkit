#!/usr/bin/env python3
"""Render one `vrfkit export` directory into a self-contained 2D viewer page.

This is a verification instrument, not a product. It exists to answer whether
the parsed data actually describes a game of VALORANT, so every choice favours
making a wrong value visible over looking good: playback is downsampled to
20 Hz but the checks run over the full 125 Hz stream, every filtered row is
counted, and a missing map transform fails the build rather than drawing on a
blank square at a guessed scale.

Usage:
    python tools/build_replay_viewer.py --export out/myreplay --out replay.html
"""
from __future__ import annotations

import argparse
import base64
import json
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import viewer_data as vd
import viewer_projection as vp

TEMPLATE = Path(__file__).resolve().parent / "viewer_template.html"


def build(export_dir: Path, out_path: Path, cache_dir: Path, fetch=None) -> dict:
    """Write the page. Returns the per-check counts, zeros included."""
    context = vd.read_export(export_dir)
    map_url = (context["manifest"].get("level_names_and_times") or [{}])[0].get("name", "")
    constants = vp.load_constants(map_url, cache_dir, fetch=fetch)
    png = vp.load_minimap_png(constants, cache_dir, fetch=fetch)

    context["constants"] = constants
    findings, counts = vd.run_checks(context)

    payload = _build_payload(context, constants, findings, counts)

    html = TEMPLATE.read_text(encoding="utf-8")
    html = html.replace("/*__VIEWER_PAYLOAD__*/",
                        "const PAYLOAD = " + json.dumps(payload) + ";")
    html = html.replace("__MINIMAP_DATA_URI__",
                        "data:image/png;base64," + base64.b64encode(png).decode("ascii"))
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(html, encoding="utf-8")
    return counts


def _build_payload(context: dict, constants: vp.MapConstants,
                   findings: list, counts: dict) -> dict:
    """Everything the page's script reads out of `PAYLOAD`.

    Two fields the page's own contract (see `viewer_template.html`) demands
    and that a naive re-derivation of the plan's brief silently drops:

    - `constants`: effects carry raw world `spawn_x`/`spawn_y`, unlike
      `frames`, which are pre-projected into u/v space by `_pack_frames`
      below. Without `x_multiplier`/`y_multiplier`/`x_scalar`/`y_scalar` the
      page's `drawEffects` has nothing to project them with and the effects
      layer never renders, silently.
    - `health` as JSON OBJECTS, not bare tuples: the page reads `row.section`,
      `row.guid`, `row.time_ms`, `row.life_result` BY NAME (`indexHealth` in
      the template). A bare tuple serialises to a JSON array, every named
      read comes back `undefined`, and the health layer quietly shows
      nothing while the rest of the page looks fine.
    """
    return {
        "playbackHz": vd.PLAYBACK_HZ,
        "rounds": [r._asdict() for r in context["rounds"]],
        "players": {str(g): label for g, label in context["players"].items()},
        "counts": counts,
        "findings": [f._asdict() for f in findings],
        "frames": _pack_frames(context, constants),
        "effects": context["effects"],
        "events": [{"time_ms": t, "group": g, "word0": w0, "word1": w1}
                   for t, g, w0, w1 in context["events"]],
        "health": [{"time_ms": t, "guid": g, "life_result": life, "is_heal": is_heal,
                    "section": section, "instance": instance}
                   for t, g, life, is_heal, section, instance in context["health"]],
        "constants": constants._asdict(),
    }


def _pack_yaw_byte(yaw_deg) -> int:
    """One byte encoding a heading in degrees, matching the page's
    `decodeFrames` (see the comment above it in `viewer_template.html`): the
    page decodes `yaw_deg = byte * 360 / 256`, so packing must be the exact
    inverse, `byte = round((yaw_deg % 360.0) / 360.0 * 256.0) % 256`.

    Naively packing `int(yaw_deg) % 256` -- the plan's own sketch -- does not
    round-trip: 300 deg and 44 deg both land on byte 44 (`int(300) % 256 ==
    int(44) % 256 == 44`), so a build using that formula produces a
    perfectly valid-looking payload while every arrow whose true heading
    exceeds 255 deg points in an unrelated, wrong direction, with no test
    that only checks "a byte was written" able to tell the difference.

    `yaw_deg % 360.0` is always in [0, 360) for any float input in Python
    (`%` follows the sign of the divisor), so a negative UE yaw needs no
    special handling.
    """
    if yaw_deg is None:
        return 0
    return round((yaw_deg % 360.0) / 360.0 * 256.0) % 256


def _pack_frames(context: dict, constants: vp.MapConstants) -> dict:
    """Downsampled playback positions per GUID, as base64 uint16 pairs.

    PLAYBACK ONLY. `run_checks` has already read the full-rate stream; see the
    note in `viewer_data`.
    """
    by_guid: dict[int, list] = {}
    for (time_ms, guid, x, y, z, _vel_z), yaw in zip(context["movement"], context["yaws"]):
        if vp.is_parked(x, z):
            continue
        by_guid.setdefault(guid, []).append((time_ms, x, y, yaw))

    packed = {}
    for guid, rows in by_guid.items():
        # Keyed on time_ms ALONE, not the bare tuple: time_ms is
        # non-decreasing but not strictly increasing (docs/DATA.md), so real
        # ties exist, and sorting the bare tuple would break a tie on x
        # ascending instead of preserving the stream's own order --
        # health_series documents the identical hazard for the same reason.
        rows.sort(key=lambda r: r[0])
        kept = vd.downsample(rows, vd.PLAYBACK_HZ)
        blob = bytearray()
        for time_ms, x, y, yaw in kept:
            u, v = vp.project(x, y, constants)
            blob += struct.pack(
                "<IHHB", time_ms,
                max(0, min(65535, int(u * 65535))),
                max(0, min(65535, int(v * 65535))),
                _pack_yaw_byte(yaw))
        packed[str(guid)] = {
            "kind": vd.classify_guid(guid, context["players"], context["actor_classes"]),
            "class_path": context["actor_classes"].get(guid, ""),
            "data": base64.b64encode(bytes(blob)).decode("ascii"),
            "count": len(kept),
        }
    return packed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--export", type=Path, required=True,
                        help="directory written by `vrfkit export`")
    parser.add_argument("--out", type=Path, required=True, help="output .html path")
    parser.add_argument("--cache", type=Path, default=Path("out/mapcache"),
                        help="where the fetched map transform and image are kept")
    args = parser.parse_args()

    counts = build(args.export, args.out, args.cache)
    print(f"wrote {args.out}")
    for kind in vd.CHECK_KINDS:
        print(f"  {kind:24s} {counts[kind]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
