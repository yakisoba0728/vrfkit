"""Guards for the C# comparison report.

This one is a REPORT, not a gate: vrfkit deliberately exports more than the C#
parser does, so most of what it prints is a measurement rather than a verdict,
and no threshold in it can be defended without the corpus in hand.

What a report still may not do is claim a result it did not measure. Its
coverage section printed

    ### C# only: NONE -- vrfkit covers everything C# has! (checkmark)

whenever `cs_only` was empty -- including when the C# side yielded no pairs at
all, which is what an empty, slimmed or wrong events.ndjson produces. Nothing
compared reads exactly like total coverage.

`main` also returned None and was called bare from `__main__`, so even a
deliberate nonzero could not have escaped the process.
"""
import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import compare_with_csharp as guard  # noqa: E402


PAIR_A = ("/Script/ShooterGame.Thing", "Health")
PAIR_B = ("/Script/ShooterGame.Thing", "Armor")


class CoverageProblemTests(unittest.TestCase):
    def test_an_overlapping_comparison_has_no_problems(self):
        self.assertEqual(guard.coverage_problems({PAIR_A}, {PAIR_A, PAIR_B}), [])

    def test_an_empty_csharp_side_is_not_full_coverage(self):
        problems = guard.coverage_problems(set(), {PAIR_A})
        self.assertTrue(problems)
        self.assertIn("C#", " ".join(problems))

    def test_an_empty_vrfkit_side_is_not_a_comparison_either(self):
        self.assertTrue(guard.coverage_problems({PAIR_A}, set()))

    def test_two_sides_sharing_nothing_is_total_disagreement(self):
        problems = guard.coverage_problems({PAIR_A}, {PAIR_B})
        self.assertTrue(problems)
        self.assertIn("no", " ".join(problems).lower())

    def test_missing_pairs_alone_are_not_reported_here(self):
        """C#-only pairs are the report's subject, not a gate.

        vrfkit's stated aim is to reproduce AND EXCEED the C# parser, and what
        counts as an acceptable miss cannot be decided without the corpus. The
        section already prints every one of them under INVESTIGATE.
        """
        self.assertEqual(guard.coverage_problems({PAIR_A, PAIR_B}, {PAIR_A}), [])


class CoverageTextTests(unittest.TestCase):
    def test_full_coverage_is_only_claimed_when_something_was_compared(self):
        lines = guard.coverage_lines({PAIR_A}, {PAIR_A, PAIR_B})
        self.assertIn("covers everything", " ".join(lines))

    def test_nothing_compared_never_claims_full_coverage(self):
        lines = guard.coverage_lines(set(), {PAIR_A})
        self.assertNotIn("covers everything", " ".join(lines))

    def test_a_real_miss_is_still_listed_for_investigation(self):
        lines = guard.coverage_lines({PAIR_A, PAIR_B}, {PAIR_A})
        joined = " ".join(lines)
        self.assertIn("INVESTIGATE", joined)
        self.assertIn("Armor", joined)


if __name__ == "__main__":
    unittest.main()
