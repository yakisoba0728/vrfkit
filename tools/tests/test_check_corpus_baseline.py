"""Guards for the corpus baseline pinner.

`--update` wrote whatever the run produced, including runs where the oracle
failed. `measure` records a failed replay as `{"error": "exit 1"}` and skips it
when summing, so pinning such a run stored zeros -- and a later run that failed
in exactly the same way then MATCHED the baseline and reported OK.

`check_metrics_baseline.py` already refuses to pin a broken run ("baseline NOT
updated -- refusing to pin a broken run"). This is the same rule for the
validate path.
"""
import os
import sys
import tempfile
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_corpus_baseline as guard  # noqa: E402


def measurement(per_file, totals=None):
    return {
        "branches": {"1302": len(per_file)},
        "totals": totals or {"blocks": 10, "fields": 20, "rpcs": 5,
                             "malformed": 0, "skipped": 0},
        "per_file": per_file,
    }


CLEAN_ENTRY = {"branch": "1302", "rate": "100.000000", "blocks": 10,
               "fields": 20, "rpcs": 5, "malformed": 0, "skipped": 0}


class UnpinnableTests(unittest.TestCase):
    def test_a_clean_run_can_be_pinned(self):
        self.assertEqual(guard.unpinnable(measurement({"a.vrf": CLEAN_ENTRY})), [])

    def test_a_run_with_a_failed_replay_cannot_be_pinned(self):
        """Pinning it stores zeros that the same failure will match later."""
        current = measurement({"a.vrf": CLEAN_ENTRY,
                               "b.vrf": {"error": "exit 1"}})
        reasons = guard.unpinnable(current)
        self.assertTrue(reasons)
        self.assertIn("b.vrf", " ".join(reasons))

    def test_a_counter_the_oracle_did_not_print_cannot_be_pinned(self):
        """`measure` records it as None rather than 0, and None must not pin.

        A None in the baseline is matched by the same counter going missing
        again, which is the vacuous-zero failure one level up.
        """
        entry = dict(CLEAN_ENTRY, malformed=None)
        reasons = guard.unpinnable(measurement({"a.vrf": entry}))
        self.assertTrue(reasons)
        self.assertIn("malformed", " ".join(reasons))

    def test_a_run_with_no_replays_at_all_cannot_be_pinned(self):
        self.assertTrue(guard.unpinnable(measurement({})))


class DiffTests(unittest.TestCase):
    """Unchanged behaviour, pinned so the refusal cannot be bolted on wrongly."""

    def test_identical_measurements_do_not_drift(self):
        m = measurement({"a.vrf": CLEAN_ENTRY})
        self.assertEqual(guard.diff(m, m), [])

    def test_a_replay_leaving_the_corpus_is_drift(self):
        before = measurement({"a.vrf": CLEAN_ENTRY, "b.vrf": CLEAN_ENTRY})
        after = measurement({"a.vrf": CLEAN_ENTRY})
        self.assertTrue(any("missing replay: b.vrf" in d for d in guard.diff(before, after)))


class CorpusMeasurementTests(unittest.TestCase):
    SUMMARY = """
Branch: ++Ares-Core+release-13.02
Total content blocks: 10
Fields emitted: 20
RPCs emitted: 5
Malformed framing: 0
Skipped bits: 0
ORACLE PASS RATE: 100.000000%
"""

    def run_measure(self, script: str, relative_files: list[str]):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            corpus = root / "corpus"
            corpus.mkdir()
            for relative in relative_files:
                path = corpus / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"replay")
            (root / "validate").write_text(script, encoding="utf-8")
            previous = Path.cwd()
            os.chdir(root)
            try:
                return guard.measure(Path(sys.executable), corpus)
            finally:
                os.chdir(previous)

    def test_duplicate_basenames_are_keyed_by_relative_path(self):
        script = (
            "from pathlib import Path\nimport sys\n"
            f"summary = {self.SUMMARY!r}\n"
            "if 'bad' in Path(sys.argv[1]).parts:\n"
            "    print('deliberate failure', file=sys.stderr)\n"
            "    raise SystemExit(7)\n"
            "print(summary)\n"
        )
        result = self.run_measure(script, ["good/same.vrf", "bad/same.vrf"])

        self.assertEqual(
            set(result["per_file"]), {"good/same.vrf", "bad/same.vrf"}
        )
        self.assertNotIn("error", result["per_file"]["good/same.vrf"])
        self.assertIn("error", result["per_file"]["bad/same.vrf"])
        self.assertTrue(any("bad/same.vrf" in r for r in guard.unpinnable(result)))

    def test_missing_branch_is_a_controlled_unpinnable_failure(self):
        script = f"print({self.SUMMARY.replace('Branch: ++Ares-Core+release-13.02', '')!r})\n"
        result = self.run_measure(script, ["nested/replay.vrf"])

        entry = result["per_file"]["nested/replay.vrf"]
        self.assertIn("error", entry)
        self.assertIn("Branch", entry["error"])
        self.assertTrue(guard.unpinnable(result))


if __name__ == "__main__":
    unittest.main()
