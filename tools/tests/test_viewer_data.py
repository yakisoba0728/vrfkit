"""Round slicing and downsampling for the 2D viewer.

The test that matters here is the last one. Movement is sampled at 125 Hz and
playback runs at 20 Hz, so a teleport -- the exact defect this instrument
exists to catch -- can fall between two rendered frames. Downsampling is
therefore only allowed to affect PLAYBACK; the measurement pass must still see
the full-rate stream. If that separation ever collapses, the viewer will look
correct while hiding the thing it was built to find.
"""
import sys
import unittest
from pathlib import Path

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


if __name__ == "__main__":
    unittest.main()
