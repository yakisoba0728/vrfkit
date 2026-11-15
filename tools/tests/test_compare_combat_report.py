"""Guards for the CombatReport comparison, a regression gate docs/USAGE.md
lists: its verdict must reach the exit code, and comparing nothing is no
match."""
import collections
import io
import unittest
from pathlib import Path
from unittest import mock

import support  # puts tools/ on sys.path

import compare_combat_report as guard

SHAPE = "Rounds[].Reports[].Interactions[].DamageDealt"


def counters(pairs):
    return collections.defaultdict(collections.Counter,
                                   {s: collections.Counter(c) for s, c in pairs})


class CompareTests(unittest.TestCase):
    def test_identical_multisets_match(self):
        both = [(SHAPE, {35: 2, 40: 1})]
        _rows, ok, _ = guard.compare(counters(both), counters(both), {SHAPE})
        self.assertTrue(ok)

    def test_a_differing_count_does_not_match(self):
        _rows, ok, _ = guard.compare(counters([(SHAPE, {35: 2})]),
                                  counters([(SHAPE, {35: 1})]), {SHAPE})
        self.assertFalse(ok)

    def test_a_shape_absent_on_both_sides_still_matches(self):
        _rows, ok, _ = guard.compare(counters([]), counters([]), {SHAPE})
        self.assertTrue(ok)

    def test_a_shape_present_on_one_side_only_does_not_match(self):
        _rows, ok, _ = guard.compare(counters([(SHAPE, {35: 1})]), counters([]),
                                  {SHAPE})
        self.assertFalse(ok)


class VacuousMatchTests(unittest.TestCase):
    """Empty counters satisfy `a == b`, so a replay carrying none of the
    shapes must not read as a match, and the `absent both sides` arm must be
    reached before the equality test."""

    def test_nothing_to_compare_is_counted_as_nothing_compared(self):
        self.assertEqual(guard.compare(counters([]), counters([]), {SHAPE})[2], 0)

    def test_a_shape_on_either_side_counts_as_compared(self):
        one = counters([(SHAPE, {35: 1})])
        self.assertEqual(guard.compare(one, counters([]), {SHAPE})[2], 1)
        self.assertEqual(guard.compare(counters([]), one, {SHAPE})[2], 1)

    def test_the_absent_on_both_sides_verdict_comes_before_equality(self):
        rows, _ok, _checked = guard.compare(counters([]), counters([]), {SHAPE})
        self.assertIn("absent both sides", " ".join(rows))
        self.assertNotIn("MATCH", " ".join(rows))


class ExitCodeTests(unittest.TestCase):
    """The verdict must reach the caller as the exit code."""

    def test_matching_data_exits_zero(self):
        both = [(SHAPE, {35: 2})]
        self.assertEqual(guard.main(counters(both), counters(both), {SHAPE}), 0)

    def test_differing_data_exits_nonzero(self):
        code = guard.main(counters([(SHAPE, {35: 2})]),
                          counters([(SHAPE, {35: 1})]), {SHAPE})
        self.assertNotEqual(code, 0)

    def test_comparing_nothing_does_not_exit_zero(self):
        self.assertNotEqual(guard.main(counters([]), counters([]), {SHAPE}), 0)

    def test_one_shape_missing_from_both_sides_is_not_a_pass(self):
        """A matching shape must not carry the run past one nobody compared."""
        other = "Rounds[].Reports[].Interactions[].DidKill"
        both = [(SHAPE, {35: 2})]
        self.assertEqual(guard.main(counters(both), counters(both), {SHAPE, other}), 2)

    def test_a_disagreement_outranks_a_missing_shape(self):
        other = "Rounds[].Reports[].Interactions[].DidKill"
        self.assertEqual(guard.main(counters([(SHAPE, {35: 2})]),
                                    counters([(SHAPE, {35: 1})]), {SHAPE, other}), 1)


class InputTests(unittest.TestCase):
    def test_a_missing_reference_exits_2_and_says_where_it_looked(self):
        missing = Path(__file__).with_name("no-such-reference.ndjson")
        with mock.patch("sys.stderr", new_callable=io.StringIO) as err:
            code = guard.main(argv=["--reference", str(missing), "--ours", str(missing)])
        self.assertEqual(code, 2)
        self.assertIn(str(missing), err.getvalue())

    def test_the_default_reference_is_not_the_valplay_bundle(self):
        self.assertNotIn("valplay", guard.DEFAULT_REFERENCE.lower())
        self.assertIn("csharp-reference", guard.DEFAULT_REFERENCE)


class DeclaredLayoutTests(support.TempDirTestCase):
    def test_shifted_handles_use_the_export_declarations(self):
        from test_to_valplay_bundle import write_export

        group = "/Game/Test.CombatReportComponent_C"
        prefix = "Rounds[0].Reports[0].Interactions[0]."
        fields = [
            {"group_path": group, "field_name": prefix + "HitsDealt", "handle": 200,
             "value_i64": 7},
            {"group_path": group, "field_name": prefix +
             "DealtInteractions[0].RegionalDamageInteractions[0].Hits", "handle": 47,
             "value_i64": 3},
            {"group_path": group, "field_name": prefix +
             "DealtInteractions[0].RegionalDamageInteractions[0].Damage", "handle": 48,
             "value_f64": 120.0},
        ]
        manifest = {"net_field_export_groups": [{"path": group, "fields": [
            {"handle": 200, "name": "HitsDealt"},
            {"handle": 47, "name": "Hits"},
            {"handle": 48, "name": "Damage"},
        ]}]}
        export = write_export(self.tmp() / "export", fields, manifest=manifest)
        ours = guard.load_ours(export / "fields.parquet")
        self.assertEqual(ours["Rounds[].Reports[].Interactions[].HitsDealt"], {7: 1})
        regional = "Rounds[].Reports[].Interactions[].DealtInteractions[].Regions[]."
        self.assertEqual(ours[regional + "Hits"], {3: 1})
        self.assertEqual(ours[regional + "Damage"], {120.0: 1})
