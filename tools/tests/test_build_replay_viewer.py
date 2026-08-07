"""End to end: a synthetic export directory in, one self-contained page out.

The plan's brief for this test predates several later tasks in this same
plan and would not catch what it looks like it catches:

- `movement.parquet` must carry `vel_z` -- `read_export` reads it as a
  required column (Task 4's `vertical_teleport` check needs it) and pyarrow
  raises before any assertion here runs if it is absent. The brief's own
  synthetic table omits it.
- the packed yaw byte is NOT `int(yaw) % 256` -- see `_pack_yaw_byte`'s
  docstring in `build_replay_viewer.py` and the comment above `decodeFrames`
  in `viewer_template.html`. A test using the wrong formula could not tell a
  correct packer from a broken one.
- `PAYLOAD.health` rows must be JSON OBJECTS, not bare tuples -- the page
  reads `row.section` by name -- and `PAYLOAD` must carry `constants`
  (effects are raw world coordinates, unlike frames, which are
  pre-projected). Both are pinned below rather than assumed.
- there are NINE check kinds, not eight (Task 4 added `vertical_teleport`).
  `counts` must pass through as-is, zeros included.
"""
import base64
import json
import re
import struct
import sys
import tempfile
import unittest
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import build_replay_viewer as builder  # noqa: E402
import viewer_data as vd  # noqa: E402
import viewer_projection as vp  # noqa: E402

MAPS_JSON = json.dumps({"data": [{
    "mapUrl": "/Game/Maps/Ascent/Ascent",
    "xMultiplier": 7.2e-05, "yMultiplier": -7.2e-05,
    "xScalarToAdd": 0.5, "yScalarToAdd": 0.5,
    "displayIcon": "https://example.invalid/ascent.png",
}]}).encode()
PNG = bytes.fromhex("89504e470d0a1a0a")  # enough to be embedded verbatim


def fetch(url):
    return PNG if url.endswith(".png") else MAPS_JSON


def _write_minimal_export(export_dir: Path) -> None:
    (export_dir / "manifest.json").write_text(json.dumps({
        "level_names_and_times": [{"name": "/Game/Maps/Ascent/Ascent", "time_ms": 0}],
        "players": [{"character_net_guid": 1, "subject": "aaaaaaaa-0000"}],
    }), encoding="utf-8")
    pq.write_table(pa.table({
        "time_ms": pa.array([0, 8, 16], pa.uint32()),
        "character_net_guid": pa.array([1, 1, 1], pa.uint32()),
        "pos_x": pa.array([0.0, 1.0, 2.0], pa.float32()),
        "pos_y": pa.array([0.0, 1.0, 2.0], pa.float32()),
        "pos_z": pa.array([100.0] * 3, pa.float32()),
        "vel_z": pa.array([0.0, 0.0, 0.0], pa.float32()),
        "yaw": pa.array([0.0, 90.0, 180.0], pa.float32()),
    }), export_dir / "movement.parquet")
    pq.write_table(pa.table({
        "time_ms": pa.array([0], pa.uint32()),
        "actor_net_guid": pa.array([1], pa.uint32()),
        "event": pa.array(["open"], pa.string()),
        "class_path": pa.array(["/G/TestCharacter.TestCharacter_C"], pa.string()),
        "spawn_x": pa.array([0.0], pa.float32()),
        "spawn_y": pa.array([0.0], pa.float32()),
        "spawn_z": pa.array([100.0], pa.float32()),
    }), export_dir / "actors.parquet")
    pq.write_table(pa.table({
        "time1": pa.array([0], pa.uint32()),
        "group": pa.array(["roundStarted"], pa.string()),
        "word0": pa.array([0], pa.uint32()),
        "word1": pa.array([0], pa.uint32()),
    }), export_dir / "events.parquet")


class BuildTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.export = self.tmp / "export"
        self.export.mkdir()
        _write_minimal_export(self.export)

    def test_the_page_is_written_and_contains_no_placeholder(self):
        out = self.tmp / "replay.html"
        builder.build(self.export, out, self.tmp / "cache", fetch=fetch)
        html = out.read_text(encoding="utf-8")
        self.assertNotIn("/*__VIEWER_PAYLOAD__*/", html)
        self.assertNotIn("__MINIMAP_DATA_URI__", html)
        self.assertIn("data:image/png;base64,", html)

    def test_every_check_count_reaches_the_page(self):
        out = self.tmp / "replay.html"
        counts = builder.build(self.export, out, self.tmp / "cache", fetch=fetch)
        html = out.read_text(encoding="utf-8")
        for kind in counts:
            self.assertIn(kind, html, f"{kind} never reached the page")

    def test_counts_carries_all_nine_kinds_zeros_included(self):
        """Task 4 added `vertical_teleport`; a count dict silently missing a
        kind would be indistinguishable on the page from a check that never
        ran at all."""
        counts = builder.build(self.export, self.tmp / "replay.html",
                                self.tmp / "cache", fetch=fetch)
        self.assertEqual(set(counts), set(vd.CHECK_KINDS))
        self.assertEqual(len(vd.CHECK_KINDS), 9)

    def test_a_missing_table_fails_by_name(self):
        (self.export / "movement.parquet").unlink()
        with self.assertRaises(SystemExit) as caught:
            builder.build(self.export, self.tmp / "x.html", self.tmp / "cache", fetch=fetch)
        self.assertIn("movement.parquet", str(caught.exception))

    def test_an_unavailable_transform_fails_the_build(self):
        def dead(url):
            raise OSError("no network")

        with self.assertRaises(SystemExit):
            builder.build(self.export, self.tmp / "x.html", self.tmp / "cache", fetch=dead)

    def test_build_measures_at_full_rate_while_playback_stays_downsampled(self):
        """The load-bearing invariant, pinned where it actually composes.

        `run_checks` reads the full-rate movement stream and `_pack_frames`
        downsamples for playback -- but nothing before this test exercised
        that separation THROUGH `build()` itself. A plausible refactor
        ("`_pack_frames` already downsamples per GUID, reuse it for both")
        can insert a downsample call ahead of `run_checks` inside `build()`
        without touching `downsample()` or `run_checks()` in isolation, so
        `test_downsampling_does_not_hide_a_teleport` (which pins `downsample`
        alone) and `test_a_teleport_is_reported` (which pins that
        `run_checks` does not downsample internally) both stay green while
        the composition in `build()` silently hides the defect.

        Two movement rows 8 ms apart -- both inside the same 50 ms playback
        bucket -- carry a horizontal jump far past TELEPORT_CM_PER_S.
        `counts["teleport"]` must be 1: the measurement pass saw it at full
        rate. The guid's packed frame blob in the rendered page must be
        exactly 9 bytes -- one <IHHB record -- because playback correctly
        collapsed both rows into the single bucket they share.
        """
        export = self.tmp / "teleport_export"
        export.mkdir()
        (export / "manifest.json").write_text(json.dumps({
            "level_names_and_times": [{"name": "/Game/Maps/Ascent/Ascent", "time_ms": 0}],
            "players": [{"character_net_guid": 1, "subject": "aaaaaaaa-0000"}],
        }), encoding="utf-8")
        pq.write_table(pa.table({
            "time_ms": pa.array([0, 8], pa.uint32()),
            "character_net_guid": pa.array([1, 1], pa.uint32()),
            "pos_x": pa.array([0.0, 5000.0], pa.float32()),
            "pos_y": pa.array([0.0, 0.0], pa.float32()),
            "pos_z": pa.array([100.0, 100.0], pa.float32()),
            "vel_z": pa.array([0.0, 0.0], pa.float32()),
            "yaw": pa.array([0.0, 90.0], pa.float32()),
        }), export / "movement.parquet")
        pq.write_table(pa.table({
            "time_ms": pa.array([0], pa.uint32()),
            "actor_net_guid": pa.array([1], pa.uint32()),
            "event": pa.array(["open"], pa.string()),
            "class_path": pa.array(["/G/TestCharacter.TestCharacter_C"], pa.string()),
            "spawn_x": pa.array([0.0], pa.float32()),
            "spawn_y": pa.array([0.0], pa.float32()),
            "spawn_z": pa.array([100.0], pa.float32()),
        }), export / "actors.parquet")
        pq.write_table(pa.table({
            "time1": pa.array([0], pa.uint32()),
            "group": pa.array(["roundStarted"], pa.string()),
            "word0": pa.array([0], pa.uint32()),
            "word1": pa.array([0], pa.uint32()),
        }), export / "events.parquet")

        out = self.tmp / "teleport.html"
        counts = builder.build(export, out, self.tmp / "cache", fetch=fetch)
        self.assertEqual(counts["teleport"], 1,
                         "measurement must see the jump at full rate, not "
                         "whatever build() left after any downsampling")

        html = out.read_text(encoding="utf-8")
        match = re.search(r"const PAYLOAD = (\{.*\});\n\n// This page", html, re.S)
        self.assertIsNotNone(match, "PAYLOAD assignment not found in the rendered page")
        payload = json.loads(match.group(1))
        blob = base64.b64decode(payload["frames"]["1"]["data"])
        self.assertEqual(len(blob), 9,
                         "playback must keep exactly one sample: both rows "
                         "share one 50 ms bucket")


class PayloadShapeTests(unittest.TestCase):
    """The template reads these fields by name (`row.section`,
    `k.x_multiplier`, ...). A payload that silently reverted `health` to bare
    tuples, or dropped `constants`, would still produce a well-formed HTML
    file -- `BuildTests` above would not notice, since none of it inspects
    the JSON structure. These do.
    """

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.export = self.tmp / "export"
        self.export.mkdir()
        _write_minimal_export(self.export)
        # One LifeResult call for the manifest player, with no resolvable
        # ChangedComponent (net_guids.parquet absent) -- health_series must
        # still return a row, labelled "unknown" (see viewer_data.py).
        pq.write_table(pa.table({
            "time_ms": pa.array([100], pa.uint32()),
            "actor_net_guid": pa.array([1], pa.uint32()),
            "packet_id": pa.array([1000], pa.uint32()),
            "channel_index": pa.array([1], pa.uint32()),
            "field_name": pa.array(
                ["MulticastNotifyDamage_Point.LifeChangeEvents[0].LifeResult"], pa.string()),
            "value_f64": pa.array([61.0], pa.float64()),
            "value_i64": pa.array([None], pa.int64()),
        }), self.export / "fields.parquet")

    def _payload(self):
        context = vd.read_export(self.export)
        constants = vp.load_constants(
            "/Game/Maps/Ascent/Ascent", self.tmp / "cache", fetch=fetch)
        context["constants"] = constants
        findings, counts = vd.run_checks(context)
        return builder._build_payload(context, constants, findings, counts), constants

    def test_health_rows_are_json_objects_with_named_keys(self):
        payload, _constants = self._payload()
        self.assertEqual(len(payload["health"]), 1)
        row = payload["health"][0]
        self.assertIsInstance(row, dict)
        self.assertEqual(set(row),
                         {"time_ms", "guid", "life_result", "is_heal", "section", "instance"})
        self.assertEqual(row["section"], "unknown")
        self.assertEqual(row["life_result"], 61.0)

    def test_payload_carries_projection_constants(self):
        """Effects hold raw world spawn_x/spawn_y, unlike frames, which
        arrive pre-projected -- the page's `drawEffects` needs these four
        fields to place an effect on the minimap at all."""
        payload, constants = self._payload()
        self.assertIn("constants", payload)
        for key in ("x_multiplier", "y_multiplier", "x_scalar", "y_scalar"):
            self.assertIn(key, payload["constants"])
            self.assertEqual(payload["constants"][key], getattr(constants, key))

    def test_the_full_build_embeds_constants_and_object_shaped_health(self):
        """The same two guarantees, exercised through the real `build()`
        entry point and the actual HTML output, not just the payload dict in
        isolation."""
        out = self.tmp / "replay.html"
        builder.build(self.export, out, self.tmp / "cache", fetch=fetch)
        html = out.read_text(encoding="utf-8")
        match = re.search(r"const PAYLOAD = (\{.*\});\n\n// This page", html, re.S)
        self.assertIsNotNone(match, "PAYLOAD assignment not found in the rendered page")
        payload = json.loads(match.group(1))
        self.assertEqual(payload["constants"]["x_multiplier"], 7.2e-05)
        self.assertEqual(len(payload["health"]), 1)
        self.assertIsInstance(payload["health"][0], dict)
        self.assertIn("section", payload["health"][0])


class FramePackingTests(unittest.TestCase):
    """`_pack_yaw_byte` and the sort order `_pack_frames` feeds it.

    The brief's own sketch (`int(yaw) % 256`) does not round-trip: 300 deg
    and 44 deg both land on byte 44, so a valid-looking value would point
    every arrow in a wrong direction with nothing to catch it. The formula
    that actually round-trips is documented above `decodeFrames` in
    `viewer_template.html`:

        pack:   byte = round((yaw_deg % 360.0) / 360.0 * 256.0) % 256
        unpack: yaw_deg = byte * 360 / 256
    """

    CONSTANTS = vp.MapConstants(
        map_url="/x", x_multiplier=7.2e-05, y_multiplier=-7.2e-05,
        x_scalar=0.5, y_scalar=0.5, display_icon_url="https://example.invalid/x.png")

    def test_the_naive_int_mod_formula_would_collide(self):
        """Sanity check that the bug this class guards against is real."""
        self.assertEqual(int(300) % 256, int(44) % 256)

    def test_300_and_44_degrees_pack_to_different_bytes(self):
        self.assertNotEqual(builder._pack_yaw_byte(300.0), builder._pack_yaw_byte(44.0))

    def test_quarter_turns_round_trip_exactly(self):
        for deg in (0.0, 90.0, 180.0, 270.0):
            byte = builder._pack_yaw_byte(deg)
            self.assertEqual(byte * 360 / 256, deg)

    def test_a_missing_yaw_packs_to_zero(self):
        self.assertEqual(builder._pack_yaw_byte(None), 0)

    def test_a_negative_yaw_wraps_to_the_same_byte_as_its_positive_equivalent(self):
        self.assertEqual(builder._pack_yaw_byte(-90.0), builder._pack_yaw_byte(270.0))

    def test_the_packed_frame_bytes_use_the_same_formula(self):
        """End-to-end through `_pack_frames`, not just the helper in
        isolation -- pins that the packer actually calls `_pack_yaw_byte`
        rather than some other formula living beside it."""
        context = {
            "movement": [(0, 1, 0.0, 0.0, 100.0, 0.0)],
            "yaws": [300.0],
            "players": {1: "aaaa1111"},
            "actor_classes": {},
        }
        packed = builder._pack_frames(context, self.CONSTANTS)
        blob = base64.b64decode(packed["1"]["data"])
        _time_ms, _u, _v, yaw_byte = struct.unpack("<IHHB", blob)
        self.assertEqual(yaw_byte, builder._pack_yaw_byte(300.0))

    def test_a_time_ms_tie_is_broken_by_stream_order_not_by_position(self):
        """DATA.md: `time_ms` is non-decreasing but not strictly increasing,
        so real ties exist. Sorting the bare `(time_ms, x, y, yaw)` tuple
        would break a tie on `x` ascending instead of preserving call order
        -- `health_series` documents the identical hazard at length for the
        same reason."""
        context = {
            "movement": [(0, 1, 5000.0, 5000.0, 100.0, 0.0),
                        (0, 1, 1000.0, 1000.0, 100.0, 0.0)],
            "yaws": [0.0, 0.0],
            "players": {1: "aaaa1111"},
            "actor_classes": {},
        }
        packed = builder._pack_frames(context, self.CONSTANTS)
        blob = base64.b64decode(packed["1"]["data"])
        # downsample keeps only the first sample per 50 ms bucket; both rows
        # share time_ms=0, so exactly one record survives.
        self.assertEqual(len(blob), 9)
        _time_ms, u, _v, _yaw_byte = struct.unpack("<IHHB", blob)
        expected_u_norm, _ = vp.project(5000.0, 5000.0, self.CONSTANTS)
        expected_u = max(0, min(65535, int(expected_u_norm * 65535)))
        self.assertEqual(u, expected_u,
                         "kept the row that was first in STREAM order (x=5000), "
                         "not first in VALUE order (x=1000)")


if __name__ == "__main__":
    unittest.main()
