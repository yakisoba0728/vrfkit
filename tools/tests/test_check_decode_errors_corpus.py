"""Guards for the corpus decode-error gate.

The gate's own docstring argues that "a counter that stops being printed must
not read as zero". Its exit path did not carry that argument one step further:
`Decoded OK` and `Struct blobs: N decoded` were summed and printed and then
never read, so an exporter that decoded NOTHING -- every counter a legitimate
zero -- printed "OK: every replay reported Decode errors: 0" and exited 0. A
counter that cannot move must not read as success either.

The second hole was the process exit status. The summary is printed before the
Parquet files are finalised, so an exporter that dies writing them has already
printed `Decode errors: 0`, and the run counted as a clean replay.

A third hole, unrelated to either of those: this file never parsed `No field
name`, even though summary.rs defines
`Rows offered = decoded_ok + decoded_err + raw_or_skip + not_in_table +
no_field_name`. The five categories this tool DID print therefore summed to
about 0.3% less than its own `rows offered` line, and a reader could not make
the numbers add up without going to read the Rust source. See `LIVE_EXPORT`
and `ReconcileTests`.
"""
import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_decode_errors_corpus as guard  # noqa: E402


#: A healthy export summary, labels as driver.rs prints them.
CLEAN = """
Rows offered:      130000
Decoded OK:        129000
Decode errors:     0
Raw/Skip:          900
Not in table:      100
No field name:     0
Struct blobs:      63 decoded / 0 failed
"""

#: The same summary from an exporter whose decoders never ran. Every counter is
#: a legitimate zero and `Decode errors: 0` is true, vacuously.
NOTHING_RAN = """
Rows offered:      0
Decoded OK:        0
Decode errors:     0
Raw/Skip:          0
Not in table:      0
No field name:     0
Struct blobs:      0 decoded / 0 failed
"""

#: Measured on a live export -- not synthesized. 742738 + 0 + 72644 + 171605 +
#: 1996 = 988983, an exact match to "Rows offered" only once `No field name`
#: is part of the sum.
LIVE_EXPORT = """
Rows offered:      988983
Decoded OK:        742738
Decode errors:     0
Raw/Skip:          72644
Not in table:      171605
No field name:     1996
Struct blobs:      63 decoded / 0 failed
"""


class ReadCountersTests(unittest.TestCase):
    def test_a_clean_summary_reads_as_counters(self):
        counters, err = guard.read_counters(CLEAN, 0)
        self.assertEqual(err, "")
        self.assertEqual(counters["decode_errors"], 0)
        self.assertEqual(counters["decoded_ok"], 129000)
        self.assertEqual(counters["struct_blobs_decoded"], 63)

    def test_a_nonzero_exit_is_not_a_clean_replay(self):
        """The summary prints before the Parquet files are finalised.

        An exporter that prints `Decode errors: 0` and then dies writing its
        output is not a replay that decoded cleanly, and counting it as one is
        how a whole corpus can pass on partial exports.
        """
        counters, err = guard.read_counters(CLEAN, 1)
        self.assertIsNone(counters)
        self.assertIn("exit 1", err)

    def test_a_summary_without_decoded_ok_is_unreadable(self):
        text = "\n".join(l for l in CLEAN.splitlines() if "Decoded OK" not in l)
        counters, err = guard.read_counters(text, 0)
        self.assertIsNone(counters)
        self.assertIn("Decoded OK", err)

    def test_a_summary_without_the_struct_blob_line_is_unreadable(self):
        text = "\n".join(l for l in CLEAN.splitlines() if "Struct blobs" not in l)
        counters, err = guard.read_counters(text, 0)
        self.assertIsNone(counters)

    def test_no_field_name_is_read(self):
        counters, err = guard.read_counters(LIVE_EXPORT, 0)
        self.assertEqual(err, "")
        self.assertEqual(counters["no_field_name"], 1996)

    def test_a_summary_without_no_field_name_is_unreadable(self):
        """`no_field_name` feeds the reconciliation, so it must be REQUIRED --
        the same "a counter that stops being printed must not read as zero"
        rule the other counters already get."""
        text = "\n".join(l for l in CLEAN.splitlines() if "No field name" not in l)
        counters, err = guard.read_counters(text, 0)
        self.assertIsNone(counters)
        self.assertIn("No field name", err)


class DeadCounterTests(unittest.TestCase):
    """The counters that are summed for the whole corpus and must have moved."""

    def test_a_working_corpus_has_no_dead_counters(self):
        totals = {"decode_errors": 0, "decoded_ok": 129000, "raw_skip": 900,
                  "not_in_table": 100, "no_field_name": 0, "rows_offered": 130000,
                  "struct_blobs_decoded": 63, "struct_blobs_failed": 0}
        self.assertEqual(guard.dead_counters(totals), [])

    def test_a_corpus_where_no_overlay_row_decoded_is_not_a_pass(self):
        totals = {"decode_errors": 0, "decoded_ok": 0, "raw_skip": 0,
                  "not_in_table": 0, "no_field_name": 0, "rows_offered": 0,
                  "struct_blobs_decoded": 63, "struct_blobs_failed": 0}
        dead = guard.dead_counters(totals)
        self.assertTrue(dead)
        self.assertIn("Decoded OK", " ".join(dead))

    def test_a_corpus_where_no_struct_blob_decoded_is_not_a_pass(self):
        """The 13.02 shape: the decoders stop running and nothing else moves."""
        totals = {"decode_errors": 0, "decoded_ok": 129000, "raw_skip": 900,
                  "not_in_table": 100, "no_field_name": 0, "rows_offered": 130000,
                  "struct_blobs_decoded": 0, "struct_blobs_failed": 0}
        dead = guard.dead_counters(totals)
        self.assertTrue(dead)
        self.assertIn("Struct blobs", " ".join(dead))

    def test_an_exporter_that_decoded_nothing_at_all_fails_on_both(self):
        totals = {"decode_errors": 0, "decoded_ok": 0, "raw_skip": 0,
                  "not_in_table": 0, "no_field_name": 0, "rows_offered": 0,
                  "struct_blobs_decoded": 0, "struct_blobs_failed": 0}
        self.assertEqual(len(guard.dead_counters(totals)), 2)


class ReconcileTests(unittest.TestCase):
    """`Rows offered` is defined in summary.rs as the sum of five categories;
    this tool used to print only four of them. `reconcile()` is the check that
    the five categories this tool now prints actually add up to the sixth
    number it also prints, so a reader never has to go read Rust source to
    make the totals add up.
    """

    def test_the_live_export_reconciles(self):
        """742738 + 0 + 72644 + 171605 + 1996 = 988983 -- measured, not invented."""
        counters, err = guard.read_counters(LIVE_EXPORT, 0)
        self.assertEqual(err, "")
        self.assertIsNone(guard.reconcile(counters))

    def test_a_mismatch_is_reported_with_both_numbers(self):
        totals = dict(decoded_ok=742738, decode_errors=0, raw_skip=72644,
                      not_in_table=171605, no_field_name=0,  # dropped 1996
                      rows_offered=988983)
        problem = guard.reconcile(totals)
        self.assertIsNotNone(problem)
        self.assertIn("988,983", problem)
        self.assertIn("986,987", problem)  # the wrong sum without no_field_name

    def test_no_field_name_absent_from_totals_is_a_loud_error_not_a_silent_zero(self):
        """`.get(..., 0)` here would make an absent counter reconcile by
        accident -- the same doctrine violation this whole fix exists to
        close. Indexing directly means a missing key raises."""
        totals = dict(decoded_ok=129000, decode_errors=0, raw_skip=900,
                      not_in_table=100, rows_offered=130000)
        with self.assertRaises(KeyError):
            guard.reconcile(totals)


class ArgParsingTests(unittest.TestCase):
    """Defect 1 wiring: discovery now goes through corpus_scan.py."""

    def test_recursive_defaults_to_false(self):
        args = guard.parse_args(["vrfkit.exe", "corpus"])
        self.assertFalse(args.recursive)

    def test_recursive_flag_is_readable(self):
        args = guard.parse_args(["vrfkit.exe", "corpus", "--recursive"])
        self.assertTrue(args.recursive)


if __name__ == "__main__":
    unittest.main()
