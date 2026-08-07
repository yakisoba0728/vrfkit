"""Round slicing and downsampling for the 2D viewer.

The test that matters here is the last one. Movement is sampled at 125 Hz and
playback runs at 20 Hz, so a teleport -- the exact defect this instrument
exists to catch -- can fall between two rendered frames. Downsampling is
therefore only allowed to affect PLAYBACK; the measurement pass must still see
the full-rate stream. If that separation ever collapses, the viewer will look
correct while hiding the thing it was built to find.
"""
import json
import sys
import tempfile
import unittest
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import viewer_data as vd  # noqa: E402
import viewer_projection as vp  # noqa: E402


class RoundSlicingTests(unittest.TestCase):
    def test_rounds_run_from_one_start_to_the_next(self):
        rounds = vd.rounds_from_events(
            [62, 92033, 227108], ["roundStarted"] * 3, match_end_ms=300000)
        self.assertEqual([(r.start_ms, r.end_ms) for r in rounds],
                         [(62, 92033), (92033, 227108), (227108, 300000)])

    def test_non_round_events_do_not_create_rounds(self):
        rounds = vd.rounds_from_events(
            [62, 500, 92033], ["roundStarted", "characterDeath", "roundStarted"],
            match_end_ms=100000)
        self.assertEqual(len(rounds), 2)

    def test_a_replay_with_no_round_events_is_one_round(self):
        """Better one usable timeline than zero. The count is reported, so an
        empty roundStarted set is visible rather than silently absent."""
        rounds = vd.rounds_from_events([], [], match_end_ms=5000)
        self.assertEqual([(r.index, r.start_ms, r.end_ms) for r in rounds], [(0, 0, 5000)])


class DownsampleTests(unittest.TestCase):
    def test_twenty_hertz_keeps_one_sample_per_fifty_milliseconds(self):
        samples = [(t, float(t)) for t in range(0, 1000, 8)]
        kept = vd.downsample(samples, hz=20)
        gaps = [b[0] - a[0] for a, b in zip(kept, kept[1:])]
        self.assertTrue(all(g >= 50 for g in gaps), f"gaps too small: {gaps[:5]}")
        # 18 is the arithmetic ceiling, not a loosened bound: samples are 8 ms
        # apart, so the smallest gap that clears 50 ms is 56 (7 * 8), and
        # 992 ms / 56 ms + 1 == 18. Asking for 19 here is unsatisfiable
        # together with the gap assertion above for ANY implementation.
        self.assertGreaterEqual(len(kept), 18)

    def test_downsampling_does_not_hide_a_teleport(self):
        """The single most important test in this suite.

        Inject a 5000 cm jump between two 8 ms samples that both fall inside
        one 50 ms playback frame. Playback may drop them; the measurement pass
        reads the full-rate list and must still report it.

        A count check is not enough here: `range(0, 400, 8)` always yields 50
        elements and `samples` is never reassigned, so `len(samples) == 50`
        can only fail if downsample changes the list's LENGTH -- it stays
        silent about whether the jump itself survives. Pin the separation
        directly instead: the full-rate list must come back byte-for-byte
        unchanged (not merely the same length), and the jump must still be
        sitting in it at the value it was injected with.
        """
        samples = [(t, 0.0, 0.0) for t in range(0, 400, 8)]
        samples[3] = (24, 5000.0, 0.0)  # inside the first 50 ms frame
        before = list(samples)
        kept = vd.downsample(samples, hz=20)
        self.assertNotIn(samples[3], kept, "fixture is wrong: the jump survived playback")
        self.assertEqual(samples, before, "downsample must not touch the full-rate stream")
        self.assertIn((24, 5000.0, 0.0), samples, "measurement must still see the jump")


CONSTANTS = vp.MapConstants("/m", 7.2e-05, -7.2e-05, 0.5, 0.5, "https://x.invalid/m.png")


def context(**over):
    # movement carries one clean row for player 1 (rather than being empty) so
    # that the "reports its zero" baseline actually exercises every loop --
    # an all-empty context is zero because nothing runs, not because nothing
    # is wrong. Tests that need a different movement stream override it below.
    # Tuple shape: (time_ms, guid, pos_x, pos_y, pos_z, vel_z). vel_z is the
    # sixth element added in this round, needed to tell a genuine fall (Abyss
    # has no floor; docs/DATA.md: real falls carry negative vel_z, median
    # around -1600 cm/s) from an anomalous vertical displacement.
    base = dict(movement=[(0, 1, 0.0, 0.0, 100.0, 0.0)], rounds=[vd.Round(0, 0, 100000)],
                players={1: "p1"}, pawn_classes={}, deaths=[], effects=[], health=[],
                constants=CONSTANTS)
    base.update(over)
    return base


class CheckTests(unittest.TestCase):
    def test_every_kind_reports_its_zero(self):
        """A line that appears only when non-zero cannot tell 'nothing wrong'
        apart from 'the check stopped running'."""
        _, counts = vd.run_checks(context())
        self.assertEqual(sorted(counts), sorted(vd.CHECK_KINDS))
        self.assertTrue(all(v == 0 for v in counts.values()), counts)

    def test_a_teleport_is_reported(self):
        # The 8 ms spacing is load-bearing, not incidental: both rows fall in
        # the same 50 ms playback bucket, so downsample() would collapse this
        # pair to one sample and hide the jump. This is the only thing in the
        # suite that would catch run_checks silently downsampling before
        # measuring -- widening the gap removes that guard.
        mv = [(1000, 1, 0.0, 0.0, 100.0, 0.0), (1008, 1, 5000.0, 0.0, 100.0, 0.0)]
        findings, counts = vd.run_checks(context(movement=mv))
        self.assertEqual(counts["teleport"], 1)
        self.assertEqual(findings[0].time_ms, 1008)

    def test_a_respawn_jump_is_not_a_teleport(self):
        """A round start moves everyone to spawn. That is not a defect."""
        mv = [(1000, 1, 0.0, 0.0, 100.0, 0.0), (1008, 1, 5000.0, 0.0, 100.0, 0.0)]
        rounds = [vd.Round(0, 0, 500), vd.Round(1, 500, 100000)]
        _, counts = vd.run_checks(context(movement=mv, rounds=rounds))
        self.assertEqual(counts["teleport"], 0)

    def test_a_vertical_jump_is_reported(self):
        """Horizontal hypot() is blind to a pure-vertical displacement: two
        rows with identical x/y report zero horizontal speed no matter how
        far apart in z. This is the check that catches that case."""
        mv = [(1000, 1, 0.0, 0.0, 100.0, 0.0), (1008, 1, 0.0, 0.0, 5100.0, 50.0)]
        findings, counts = vd.run_checks(context(movement=mv))
        self.assertEqual(counts["vertical_teleport"], 1)
        self.assertEqual(counts["teleport"], 0, "pure-vertical must not also trip the horizontal check")

    def test_a_fall_with_negative_vel_z_is_not_a_vertical_teleport(self):
        """docs/DATA.md: Abyss has no floor and real falls carry negative
        vel_z (median around -1600 cm/s, against 0 for in-range rows).
        Folding z into the horizontal distance would make every real fall a
        false teleport; vel_z is what tells the two apart."""
        mv = [(1000, 1, 0.0, 0.0, 5100.0, 0.0), (1008, 1, 0.0, 0.0, 100.0, -1600.0)]
        _, counts = vd.run_checks(context(movement=mv))
        self.assertEqual(counts["vertical_teleport"], 0)

    def test_a_vertical_jump_at_a_round_boundary_is_not_reported(self):
        """Respawn parity with the horizontal check: a round transition
        moves players to spawn, z included, and that is not a defect."""
        mv = [(1000, 1, 0.0, 0.0, 100.0, 0.0), (1008, 1, 0.0, 0.0, 5100.0, 50.0)]
        rounds = [vd.Round(0, 0, 500), vd.Round(1, 500, 100000)]
        _, counts = vd.run_checks(context(movement=mv, rounds=rounds))
        self.assertEqual(counts["vertical_teleport"], 0)

    def test_horizontal_and_vertical_can_both_fire_on_the_same_pair(self):
        """Pins independence between the two checks. Both walk the same
        (t0, t1) pair; the horizontal check firing must not suppress the
        vertical one, or vice versa. A `continue` reinstated after the
        horizontal branch (the brief's original shape) would exit the pair
        before the vertical block ever runs -- exactly how a pure-vertical
        teleport went silent before this check existed, except here it would
        hide a MIXED teleport instead, which none of the single-axis tests
        above can catch."""
        mv = [(1000, 1, 0.0, 0.0, 100.0, 0.0), (1008, 1, 5000.0, 0.0, 5100.0, 50.0)]
        _, counts = vd.run_checks(context(movement=mv))
        self.assertEqual(counts["teleport"], 1)
        self.assertEqual(counts["vertical_teleport"], 1)

    def test_a_parked_row_is_counted_not_dropped_in_silence(self):
        mv = [(10, 1, -50000.0, 0.0, -49900.0, 0.0)]
        _, counts = vd.run_checks(context(movement=mv))
        self.assertEqual(counts["parked"], 1)

    def test_a_position_outside_the_map_is_reported(self):
        mv = [(10, 1, 9.0e6, 9.0e6, 100.0, 0.0)]
        _, counts = vd.run_checks(context(movement=mv))
        self.assertEqual(counts["off_map"], 1)

    def test_a_player_with_no_movement_in_a_round_is_reported(self):
        _, counts = vd.run_checks(context(movement=[], players={1: "p1"}))
        self.assertEqual(counts["absent_player"], 1)

    def test_a_death_with_no_nearby_movement_is_reported(self):
        _, counts = vd.run_checks(context(deaths=[(4000, 1)]))
        self.assertEqual(counts["death_without_position"], 1)

    def test_an_unknown_movement_guid_is_reported(self):
        mv = [(10, 999, 0.0, 0.0, 100.0, 0.0)]
        _, counts = vd.run_checks(context(movement=mv))
        self.assertEqual(counts["unknown_guid"], 1)

    def test_a_known_pawn_guid_is_not_unknown(self):
        mv = [(10, 999, 0.0, 0.0, 100.0, 0.0)]
        _, counts = vd.run_checks(
            context(movement=mv, pawn_classes={999: "Pawn_Hunter_E_Drone_C"}))
        self.assertEqual(counts["unknown_guid"], 0)

    def test_an_effect_outside_the_map_is_reported(self):
        eff = [{"open_ms": 10, "spawn_x": 9.0e6, "spawn_y": 9.0e6, "spawn_z": 0.0,
                "effect_type": "smoke", "actor_net_guid": 5}]
        _, counts = vd.run_checks(context(effects=eff))
        self.assertEqual(counts["effect_off_map"], 1)

    def test_health_rising_without_a_heal_is_reported(self):
        hp = [(1000, 1, 50.0, False), (2000, 1, 90.0, False)]
        _, counts = vd.run_checks(context(health=hp))
        self.assertEqual(counts["unexplained_heal"], 1)

    def test_health_rising_with_a_heal_is_not_reported(self):
        hp = [(1000, 1, 50.0, False), (2000, 1, 90.0, True)]
        _, counts = vd.run_checks(context(health=hp))
        self.assertEqual(counts["unexplained_heal"], 0)


class ClassifyTests(unittest.TestCase):
    def test_a_manifest_guid_is_a_player(self):
        self.assertEqual(vd.classify_guid(870, {870: "p1"}, {}), "player")

    def test_a_drone_is_a_pawn(self):
        self.assertEqual(
            vd.classify_guid(13836, {}, {13836: "/G/Pawn_Hunter_E_Drone.Pawn_Hunter_E_Drone_C"}),
            "pawn")

    def test_a_post_death_camera_is_not_a_position(self):
        """Drawing these renders a dead player apparently walking around.
        They are spectator cameras; the layer is off by default."""
        self.assertEqual(
            vd.classify_guid(34312, {}, {34312: "/G/Smonk_PostDeath_PC.Smonk_PostDeath_PC_C"}),
            "postdeath")

    def test_an_unresolved_guid_is_unknown(self):
        self.assertEqual(vd.classify_guid(5, {}, {}), "unknown")


class ReadExportTests(unittest.TestCase):
    """read_export turns one export directory into the context dict
    run_checks (Task 4) consumes, plus manifest/match_end_ms/actor_classes.

    Fixtures write only the columns each reader actually asks for --
    read_export's own columns=[...] selections, and separately
    extract_active_effects.build_with_tally's full-table read of actors.parquet
    -- rather than a full replica of the real schema.
    """

    DRONE_PATH = "/Game/Pawn_Hunter_E_Drone.Pawn_Hunter_E_Drone_C"
    POSTDEATH_PATH = "/Game/Characters/Smonk/Smonk_PostDeath_PC.Smonk_PostDeath_PC_C"

    @staticmethod
    def _write_movement(path, rows):
        def col(name, default, dtype):
            return pa.array([r.get(name, default) for r in rows], type=dtype)
        table = pa.table({
            "time_ms": col("time_ms", 0, pa.uint32()),
            "character_net_guid": col("guid", 0, pa.uint32()),
            "pos_x": col("pos_x", 0.0, pa.float32()),
            "pos_y": col("pos_y", 0.0, pa.float32()),
            "pos_z": col("pos_z", 0.0, pa.float32()),
            "vel_z": col("vel_z", 0.0, pa.float32()),
            "yaw": col("yaw", 0.0, pa.float32()),
        })
        pq.write_table(table, path)

    @staticmethod
    def _write_actors(path, rows):
        def col(name, default, dtype):
            return pa.array([r.get(name, default) for r in rows], type=dtype)
        table = pa.table({
            "actor_net_guid": col("guid", 0, pa.uint32()),
            "event": col("event", "open", pa.string()),
            "time_ms": col("time_ms", 0, pa.uint32()),
            # Nullable, unlike the other columns: actors.rs writes a null
            # class_path when the GUID cache had no mapping for the row.
            "class_path": pa.array([r.get("class_path") for r in rows], type=pa.string()),
            "spawn_x": col("spawn_x", 0.0, pa.float32()),
            "spawn_y": col("spawn_y", 0.0, pa.float32()),
            "spawn_z": col("spawn_z", 0.0, pa.float32()),
        })
        pq.write_table(table, path)

    @staticmethod
    def _write_events(path, rows):
        def col(name, default, dtype):
            return pa.array([r.get(name, default) for r in rows], type=dtype)
        table = pa.table({
            "time1": col("time1", 0, pa.uint32()),
            "group": col("group", "", pa.string()),
            # Nullable: events.rs stores word0/word1 as Option<u32>, null for
            # groups that carry fewer than two payload words.
            "word0": pa.array([r.get("word0") for r in rows], type=pa.uint32()),
            "word1": pa.array([r.get("word1") for r in rows], type=pa.uint32()),
        })
        pq.write_table(table, path)

    def write_export(self, root: Path, *, movement=None, actors=None, events=None,
                      players=None, manifest_extra=None) -> Path:
        export = root / "export"
        export.mkdir()
        self._write_movement(export / "movement.parquet", movement or [])
        self._write_actors(export / "actors.parquet", actors or [])
        self._write_events(export / "events.parquet", events or [])
        manifest = {"players": players if players is not None else []}
        manifest.update(manifest_extra or {})
        (export / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        return export

    def test_movement_rows_are_six_tuples_with_vel_z(self):
        """Task 4's run_checks unpacks (time_ms, guid, x, y, z, vel_z) and
        raises ValueError on anything else. vel_z is what separates a real
        Abyss fall from a vertical teleport (see run_checks); read_export
        must carry it through rather than the pre-Task-4 five-tuple."""
        with tempfile.TemporaryDirectory() as tmp:
            export = self.write_export(
                Path(tmp),
                movement=[{"time_ms": 1000, "guid": 1, "pos_x": 1.0, "pos_y": 2.0,
                           "pos_z": 3.0, "vel_z": -1600.0, "yaw": 90.0}],
                players=[{"actor_net_guid": 9, "subject": "11112222aaaa",
                          "character_net_guid": 1}])
            context = vd.read_export(export)
        self.assertEqual(context["movement"], [(1000, 1, 1.0, 2.0, 3.0, -1600.0)])
        self.assertEqual(context["yaws"], [90.0])

    def test_players_come_from_the_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            export = self.write_export(
                Path(tmp),
                players=[{"actor_net_guid": 9, "subject": "abcdefgh-1111-2222",
                          "character_net_guid": 870}])
            context = vd.read_export(export)
        self.assertEqual(context["players"], {870: "abcdefgh"})

    def test_a_pawn_guid_lands_in_pawn_classes_and_actor_classes(self):
        with tempfile.TemporaryDirectory() as tmp:
            export = self.write_export(
                Path(tmp), actors=[{"guid": 13836, "class_path": self.DRONE_PATH}])
            context = vd.read_export(export)
        self.assertEqual(context["actor_classes"], {13836: self.DRONE_PATH})
        self.assertEqual(context["pawn_classes"], {13836: self.DRONE_PATH})

    def test_a_postdeath_camera_lands_in_pawn_classes_so_it_is_not_unknown(self):
        """The trap this task exists to avoid: Smonk_PostDeath_PC is a
        spectator camera, not a position. It must still be classifiable
        (not "unknown_guid" in run_checks) so the page can key its layer
        toggle off classify_guid rather than treat it as noise."""
        with tempfile.TemporaryDirectory() as tmp:
            export = self.write_export(
                Path(tmp), actors=[{"guid": 34312, "class_path": self.POSTDEATH_PATH}])
            context = vd.read_export(export)
        self.assertIn(34312, context["pawn_classes"])
        self.assertEqual(context["actor_classes"][34312], self.POSTDEATH_PATH)

    def test_a_player_guid_is_not_duplicated_into_pawn_classes(self):
        with tempfile.TemporaryDirectory() as tmp:
            export = self.write_export(
                Path(tmp),
                actors=[{"guid": 870, "class_path": "/Game/Characters/Rune/PlayerPawn_C"}],
                players=[{"actor_net_guid": 9, "subject": "playersubj1",
                          "character_net_guid": 870}])
            context = vd.read_export(export)
        self.assertNotIn(870, context["pawn_classes"])

    def test_a_null_class_path_row_does_not_poison_a_later_resolvable_row(self):
        """actors.rs appends a null class_path when the GUID cache had no
        mapping -- e.g. an orphan close for an actor opened before the export
        window. NetGUIDs are recycled across rounds, so the same numeric GUID
        can later carry a real, resolvable open. A plain `setdefault` pins
        the first row's null forever and the later real pawn classifies as
        unknown; read_export must skip the null instead."""
        with tempfile.TemporaryDirectory() as tmp:
            export = self.write_export(
                Path(tmp),
                actors=[
                    {"guid": 999, "class_path": None, "event": "close", "time_ms": 10},
                    {"guid": 999, "class_path": self.DRONE_PATH, "event": "open",
                     "time_ms": 5000},
                ])
            context = vd.read_export(export)
        self.assertEqual(context["actor_classes"][999], self.DRONE_PATH)
        self.assertIn(999, context["pawn_classes"])

    def test_deaths_use_word1_as_the_victim_not_word0(self):
        """events.characterDeath carries (word0, word1) = (killer, killed)
        per record.rs and USAGE.md; word1 is the victim read_export must
        report, not word0."""
        with tempfile.TemporaryDirectory() as tmp:
            export = self.write_export(
                Path(tmp),
                events=[{"time1": 5000, "group": "characterDeath", "word0": 111,
                         "word1": 222}])
            context = vd.read_export(export)
        self.assertEqual(context["deaths"], [(5000, 222)])

    def test_match_end_ms_is_past_every_observed_timestamp(self):
        """Derived from the data (last movement/event timestamp + 1) rather
        than manifest duration_ms: if the manifest's stated duration outran
        the last real sample, rounds_from_events would manufacture a
        trailing dead zone with no movement rows in it, and run_checks would
        report every player absent for a stretch that was never recorded."""
        with tempfile.TemporaryDirectory() as tmp:
            export = self.write_export(
                Path(tmp),
                movement=[{"time_ms": 900, "guid": 1}],
                events=[{"time1": 1200, "group": "roundStarted", "word0": 1}],
                manifest_extra={"duration_ms": 999999})
            context = vd.read_export(export)
        self.assertEqual(context["match_end_ms"], 1201)
        self.assertEqual(context["rounds"], [vd.Round(0, 1200, 1201)])

    def test_health_is_empty_the_join_lands_in_task_6(self):
        with tempfile.TemporaryDirectory() as tmp:
            export = self.write_export(Path(tmp))
            context = vd.read_export(export)
        self.assertEqual(context["health"], [])

    def test_manifest_is_returned_for_the_page_to_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            export = self.write_export(
                Path(tmp),
                players=[{"actor_net_guid": 1, "subject": "x", "character_net_guid": 1}])
            context = vd.read_export(export)
        self.assertEqual(context["manifest"]["players"][0]["character_net_guid"], 1)

    def test_a_missing_movement_table_raises_rather_than_returning_empty_layers(self):
        with tempfile.TemporaryDirectory() as tmp:
            export = self.write_export(Path(tmp))
            (export / "movement.parquet").unlink()
            with self.assertRaises(SystemExit) as ctx:
                vd.read_export(export)
        self.assertIn("movement.parquet", str(ctx.exception))

    def test_a_missing_manifest_raises_rather_than_returning_empty_layers(self):
        """A second, distinct missing file: a guard that only checks
        movement.parquet would let this one slip through as an empty-layer
        fallback instead of failing loudly."""
        with tempfile.TemporaryDirectory() as tmp:
            export = self.write_export(Path(tmp))
            (export / "manifest.json").unlink()
            with self.assertRaises(SystemExit) as ctx:
                vd.read_export(export)
        self.assertIn("manifest.json", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
