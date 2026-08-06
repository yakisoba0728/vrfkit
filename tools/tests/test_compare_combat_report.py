"""Guards for the CombatReport comparison.

This script is listed in `docs/USAGE.md` among the regression gates, and it
could not fail. Everything ran at module import, the verdict was printed, and
there was no exit path at all -- so `SOME SHAPES DIFFER` and
`ALL INTERESTING SHAPES MATCH` both left the process at 0. Anything gating on
`$?` read a broken decoder as a pass.
"""
import collections
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import compare_combat_report as guard  # noqa: E402

SHAPE = "Rounds[].Reports[].Interactions[].DamageDealt"


def counters(pairs):
    return collections.defaultdict(collections.Counter,
                                   {s: collections.Counter(c) for s, c in pairs})


class CompareTests(unittest.TestCase):
    def test_identical_multisets_match(self):
        both = [(SHAPE, {35: 2, 40: 1})]
        _rows, ok = guard.compare(counters(both), counters(both), {SHAPE})
        self.assertTrue(ok)

    def test_a_differing_count_does_not_match(self):
        _rows, ok = guard.compare(counters([(SHAPE, {35: 2})]),
                                  counters([(SHAPE, {35: 1})]), {SHAPE})
        self.assertFalse(ok)

    def test_a_shape_absent_on_both_sides_still_matches(self):
        _rows, ok = guard.compare(counters([]), counters([]), {SHAPE})
        self.assertTrue(ok)

    def test_a_shape_present_on_one_side_only_does_not_match(self):
        _rows, ok = guard.compare(counters([(SHAPE, {35: 1})]), counters([]),
                                  {SHAPE})
        self.assertFalse(ok)


class ExitCodeTests(unittest.TestCase):
    """The part that was actually broken: the verdict reaching the caller."""

    def test_matching_data_exits_zero(self):
        both = [(SHAPE, {35: 2})]
        self.assertEqual(guard.main(counters(both), counters(both), {SHAPE}), 0)

    def test_differing_data_exits_nonzero(self):
        code = guard.main(counters([(SHAPE, {35: 2})]),
                          counters([(SHAPE, {35: 1})]), {SHAPE})
        self.assertNotEqual(code, 0)


if __name__ == "__main__":
    unittest.main()
