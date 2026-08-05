"""Guards for the benchmark harness's judgement.

Timing is noisy and the harness cannot change that. What it must not do is turn
noise into a verdict, or let a genuinely faster run pass silently -- a run well
under the baseline means the baseline is stale, which is the same problem as a
regression pointed the other way.
"""
import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import bench_export as bench  # noqa: E402


class MedianTests(unittest.TestCase):
    def test_an_odd_count_takes_the_middle(self):
        self.assertEqual(bench.median([3.0, 1.0, 2.0]), 2.0)

    def test_an_even_count_averages_the_two_middles(self):
        self.assertEqual(bench.median([1.0, 2.0, 3.0, 4.0]), 2.5)

    def test_a_single_sample_is_itself(self):
        self.assertEqual(bench.median([1.5]), 1.5)

    def test_no_samples_is_an_error_not_a_zero(self):
        """A zero would read as an infinitely fast run."""
        with self.assertRaises(ValueError):
            bench.median([])


class CompareTests(unittest.TestCase):
    TOL = 0.20

    def test_the_same_time_is_ok(self):
        verdict, ratio = bench.compare(1.0, 1.0, self.TOL)
        self.assertEqual(verdict, "ok")
        self.assertAlmostEqual(ratio, 1.0)

    def test_noise_inside_the_tolerance_is_ok(self):
        for measured in (1.19, 0.81):
            self.assertEqual(bench.compare(measured, 1.0, self.TOL)[0], "ok")

    def test_past_the_tolerance_is_a_regression(self):
        verdict, ratio = bench.compare(1.5, 1.0, self.TOL)
        self.assertEqual(verdict, "slower")
        self.assertAlmostEqual(ratio, 1.5)

    def test_well_under_the_baseline_is_reported_not_ignored(self):
        """Faster than recorded means the baseline no longer describes the code."""
        self.assertEqual(bench.compare(0.5, 1.0, self.TOL)[0], "faster")

    def test_a_zero_baseline_is_an_error(self):
        with self.assertRaises(ValueError):
            bench.compare(1.0, 0.0, self.TOL)


if __name__ == "__main__":
    unittest.main()
