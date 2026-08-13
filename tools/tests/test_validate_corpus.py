"""Guards for the corpus oracle sweep.

The accumulator carries a comment saying "A counter the oracle stopped printing
must not read as zero. That is precisely how the malformed figure stayed a
vacuous 0 for the whole corpus while its pattern was wrong" -- and then the
absent counter was printed as a WARNING and the run exited 0 anyway. Writing
the argument down is not the same as acting on it.
"""
import collections
import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import validate_corpus as guard  # noqa: E402


class ProblemTests(unittest.TestCase):
    def test_a_clean_sweep_has_no_problems(self):
        self.assertEqual(guard.problems([], collections.Counter()), [])

    def test_a_replay_the_oracle_could_not_validate_is_a_problem(self):
        found = guard.problems([("a.vrf", "exit 101")], collections.Counter())
        self.assertTrue(found)
        self.assertIn("a.vrf", " ".join(found))

    def test_a_counter_the_oracle_stopped_printing_is_a_problem(self):
        """The defect: this was a WARNING beside an exit 0."""
        found = guard.problems([], collections.Counter({"malformed": 3}))
        self.assertTrue(found)
        joined = " ".join(found)
        self.assertIn("malformed", joined)
        self.assertIn("3", joined)

    def test_every_absent_counter_is_named_not_just_the_first(self):
        found = guard.problems(
            [], collections.Counter({"malformed": 3, "skipped": 1}))
        self.assertEqual(len(found), 2, found)

    def test_failures_and_absent_counters_are_both_reported(self):
        found = guard.problems([("a.vrf", "timeout")],
                               collections.Counter({"malformed": 1}))
        self.assertEqual(len(found), 2, found)


class PatternTests(unittest.TestCase):
    """The regexes are shared with check_corpus_baseline.py so they cannot drift."""

    def test_the_malformed_pattern_matches_the_label_the_oracle_prints(self):
        m = guard.PATTERNS["malformed"].search("Malformed framing:  0")
        self.assertIsNotNone(m)
        self.assertEqual(m.group(1), "0")

    def test_every_accumulated_counter_has_a_pattern(self):
        for key in ("blocks", "malformed", "skipped", "fields", "rpcs"):
            self.assertIn(key, guard.PATTERNS)

    def test_missing_branch_is_a_controlled_parse_failure(self):
        text = """
Total content blocks: 10
Fields emitted: 20
RPCs emitted: 5
Malformed framing: 0
Skipped bits: 0
ORACLE PASS RATE: 100.000000%
"""
        parsed, error = guard.parse_oracle_output(text)
        self.assertIsNone(parsed)
        self.assertIn("Branch", error)


class ArgParsingTests(unittest.TestCase):
    """Defect 1 wiring: discovery now goes through corpus_scan.py, and the
    recursion choice is an explicit, opt-in flag rather than a hardcoded glob.
    """

    def test_recursive_defaults_to_false(self):
        args = guard.parse_args(["validate_corpus.py", "vrfkit.exe", "corpus"])
        self.assertFalse(args.recursive)

    def test_recursive_flag_is_readable(self):
        args = guard.parse_args(
            ["validate_corpus.py", "vrfkit.exe", "corpus", "--recursive"])
        self.assertTrue(args.recursive)

    def test_the_optional_limit_still_parses_positionally(self):
        """Backward compatibility: `<exe> <corpus> [limit]` must keep working."""
        args = guard.parse_args(["validate_corpus.py", "vrfkit.exe", "corpus", "5"])
        self.assertEqual(args.limit, 5)

    def test_limit_is_optional(self):
        args = guard.parse_args(["validate_corpus.py", "vrfkit.exe", "corpus"])
        self.assertIsNone(args.limit)


if __name__ == "__main__":
    unittest.main()
