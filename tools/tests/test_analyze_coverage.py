"""Guards for the coverage analysis.

This is an analysis script, not a gate, and it stays one: overlay coverage is
known-incomplete by design, so failing on an extractor miss would make it
permanently red and it would be switched off.

What it may not do is print an unmeasured figure as a zero. Without the C#
descriptor directory `csharp_paths` is empty, so EVERY uncovered group falls
into "no descriptor" and the line

    C# descriptor exists but extractor missed: 0

is printed on a machine that never looked. That is the same vacuous zero the
malformed counter had for the project's whole history.
"""
import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import analyze_coverage as guard  # noqa: E402


class MissedReportTests(unittest.TestCase):
    def test_a_measured_zero_is_reported_as_a_zero(self):
        line = guard.missed_report(0, measured=True)
        self.assertIn("0", line)
        self.assertNotIn("NOT MEASURED", line)

    def test_a_measured_count_is_reported(self):
        self.assertIn("7", guard.missed_report(7, measured=True))

    def test_an_unmeasured_run_does_not_report_a_zero(self):
        line = guard.missed_report(0, measured=False)
        self.assertIn("NOT MEASURED", line)
        self.assertNotIn(": 0", line)


class ClassifyTests(unittest.TestCase):
    def test_a_group_in_the_overlay_is_covered(self):
        counts, _, _ = guard.classify(
            [{"path": "/Script/A", "fields": [1, 2]}], {"/Script/A"}, set())
        self.assertEqual(counts["covered"], 1)

    def test_a_group_with_a_descriptor_and_no_overlay_entry_was_missed(self):
        counts, missed, _ = guard.classify(
            [{"path": "/Script/A", "fields": [1, 2]}], set(), {"/Script/A"})
        self.assertEqual(counts["extractor_missed"], 1)
        self.assertEqual(missed, [("/Script/A", 2)])

    def test_a_group_nobody_describes_is_raw_only(self):
        counts, _, no_desc = guard.classify(
            [{"path": "/Script/A", "fields": [1]}], set(), set())
        self.assertEqual(counts["no_descriptor"], 1)
        self.assertEqual(no_desc, [("/Script/A", 1)])


if __name__ == "__main__":
    unittest.main()
