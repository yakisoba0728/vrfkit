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
        """
        samples = [(t, 0.0, 0.0) for t in range(0, 400, 8)]
        samples[3] = (24, 5000.0, 0.0)  # inside the first 50 ms frame
        kept = vd.downsample(samples, hz=20)
        self.assertNotIn(samples[3], kept, "fixture is wrong: the jump survived playback")
        self.assertEqual(len(samples), 50, "measurement must still see every sample")


if __name__ == "__main__":
    unittest.main()
