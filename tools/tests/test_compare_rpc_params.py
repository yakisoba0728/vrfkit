"""Guards for the RPC parameter comparison.

Two holes, and the second is the one that matters.

The comparison ran entirely inside `main`, so the verdict could not be
asserted on -- the same shape `compare_combat_report.py` had.

And BOTH SIDES EMPTY counted as a match. Two empty Counters compare equal, so
every parameter of a replay containing none of these RPCs reported `MATCH`,
`all_match` stayed True, and the script printed `ALL RPC PARAMETER VALUES
MATCH` having compared nothing at all. The `both empty` arm that was supposed
to name that case sat below the equality test and could never be reached.
"""
import collections
import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import compare_rpc_params as guard  # noqa: E402


ONE_RPC = {"MulticastEndRound": [("NewRoundNumber", "int")]}
KEY = ("MulticastEndRound", "NewRoundNumber")


def side(values=None):
    return {KEY: collections.Counter(values or {})}


class CompareTests(unittest.TestCase):
    def test_identical_multisets_match(self):
        _rows, ok, checked = guard.compare(side({1: 2}), side({1: 2}), ONE_RPC)
        self.assertTrue(ok)
        self.assertEqual(checked, 1)

    def test_a_differing_count_does_not_match(self):
        _rows, ok, _ = guard.compare(side({1: 2}), side({1: 1}), ONE_RPC)
        self.assertFalse(ok)

    def test_a_parameter_present_on_one_side_only_does_not_match(self):
        _rows, ok, _ = guard.compare(side({1: 2}), side(), ONE_RPC)
        self.assertFalse(ok)

    def test_both_sides_empty_is_not_something_that_was_compared(self):
        """The hole: nothing to compare read as agreement."""
        _rows, _ok, checked = guard.compare(side(), side(), ONE_RPC)
        self.assertEqual(checked, 0)

    def test_both_sides_empty_is_reported_as_such_not_as_a_match(self):
        """The dead arm, now reachable: it sat below `cs_vals == rust_vals`."""
        rows, _ok, _checked = guard.compare(side(), side(), ONE_RPC)
        self.assertIn("both empty", " ".join(rows))
        self.assertNotIn("MATCH", " ".join(rows))


class ExitCodeTests(unittest.TestCase):
    def test_matching_data_exits_zero(self):
        self.assertEqual(guard.main(side({1: 2}), side({1: 2}), ONE_RPC), 0)

    def test_differing_data_exits_nonzero(self):
        self.assertNotEqual(guard.main(side({1: 2}), side({1: 1}), ONE_RPC), 0)

    def test_a_replay_with_none_of_these_rpcs_does_not_claim_a_match(self):
        self.assertNotEqual(guard.main(side(), side(), ONE_RPC), 0)


class ToleranceTests(unittest.TestCase):
    def test_the_float_tolerance_is_named_so_the_verdict_can_state_it(self):
        """`MATCH` is only true to this many decimal places, and said so nowhere."""
        self.assertEqual(guard.norm(1.234, "float"),
                         round(1.234, guard.FLOAT_PLACES))


if __name__ == "__main__":
    unittest.main()
