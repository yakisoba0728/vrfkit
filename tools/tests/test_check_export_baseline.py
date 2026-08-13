"""Guards for the export baseline pinner.

`measure` deliberately records a counter the summary did not print as None
rather than 0 -- its own comment says "a counter that silently reads as absent
is how this class of bug survives". `--update` then pinned the None anyway, so
a summary that STOPPED printing a counter matched the baseline from then on.

The cross-check already catches this for the four counters that are Parquet row
identities. The other twenty had nothing.
"""
import contextlib
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_export_baseline as guard  # noqa: E402


def measurement(**overrides):
    counters = {key: 1 for key in guard.COUNTERS}
    counters.update(overrides)
    return {
        "counters": counters,
        "parquet": {name: {"rows": 1, "bytes": 100, "sha256": "a" * 64}
                    for name in guard.PARQUET_FILES},
    }


class UnpinnableTests(unittest.TestCase):
    def test_a_complete_summary_can_be_pinned(self):
        self.assertEqual(guard.unpinnable(measurement()), [])

    def test_a_counter_the_summary_did_not_print_cannot_be_pinned(self):
        reasons = guard.unpinnable(measurement(struct_blobs_decoded=None))
        self.assertTrue(reasons)
        self.assertIn("struct_blobs_decoded", " ".join(reasons))

    def test_every_absent_counter_is_named_not_just_the_first(self):
        reasons = guard.unpinnable(
            measurement(struct_blobs_decoded=None, effect_blobs_decoded=None))
        self.assertEqual(len(reasons), 2, reasons)

    def test_the_checkpoint_counters_are_only_required_when_measured(self):
        """A default run never prints them, so their absence is not a fault.

        They live outside `COUNTERS` precisely so a default run does not record
        them as None and diff that against a `--checkpoints` baseline.
        """
        current = measurement()
        self.assertNotIn("cp_frames", current["counters"])
        self.assertEqual(guard.unpinnable(current), [])


class CrossCheckTests(unittest.TestCase):
    """Unchanged behaviour, pinned alongside the new refusal."""

    def test_a_summary_disagreeing_with_its_parquet_is_a_lie(self):
        current = measurement(net_guid_rows=99)
        lies = guard.cross_checks(current["counters"], current["parquet"])
        self.assertTrue(any("NetGUID rows" in l for l in lies))

    def test_an_absent_identity_counter_is_itself_a_failure(self):
        current = measurement(movement_rows=None)
        lies = guard.cross_checks(current["counters"], current["parquet"])
        self.assertTrue(any("did not print it" in l for l in lies))


class ContentIdentityTests(unittest.TestCase):
    def test_equal_size_different_bytes_do_not_satisfy_byte_identity(self):
        baseline = measurement()
        current = measurement()
        current["parquet"]["fields"]["sha256"] = "b" * 64

        problems = guard.diff(baseline, current)

        self.assertTrue(any("fields.parquet sha256" in p for p in problems), problems)


class RequiredInputTests(unittest.TestCase):
    def test_explicit_required_mode_cannot_report_missing_replay_as_skip(self):
        with tempfile.TemporaryDirectory() as temp:
            baseline = Path(temp) / "baseline.json"
            baseline.write_text(
                json.dumps({"replay": "missing.vrf"}), encoding="utf-8"
            )
            argv = sys.argv
            sys.argv = [
                "check_export_baseline.py",
                "--baseline", str(baseline),
                "--exe", sys.executable,
                "--require-input",
            ]
            output = io.StringIO()
            try:
                with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
                    code = guard.main()
            finally:
                sys.argv = argv

        self.assertEqual(code, 2)
        self.assertIn("required", output.getvalue().lower())
        self.assertNotIn("SKIP:", output.getvalue())


class TransactionalOutputTests(unittest.TestCase):
    SUMMARY = """
Total content blocks: 1
Fields emitted: 1
RPCs emitted: 1
Movement rows: 1
Event rows: 1
NetGUID rows: 1
Actor opens: 1
Actor closes: 0
"""

    def run_fake_export(self, *, fail: bool):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        replay = root / "match.vrf"
        replay.write_bytes(b"replay")
        output = root / "published"
        output.mkdir()
        sentinel = output / "complete.sentinel"
        sentinel.write_text("previous complete output", encoding="utf-8")

        script = root / "export"
        if fail:
            script.write_text(
                "import sys\n"
                "print('deliberate export failure', file=sys.stderr)\n"
                "raise SystemExit(7)\n",
                encoding="utf-8",
            )
        else:
            script.write_text(
                "import os, shutil, sys\n"
                "from pathlib import Path\n"
                "import pyarrow as pa\n"
                "import pyarrow.parquet as pq\n"
                "out = Path(sys.argv[sys.argv.index('--out') + 1])\n"
                "stage = out.parent / (out.name + '.stage')\n"
                "backup = out.parent / (out.name + '.backup')\n"
                "stage.mkdir()\n"
                "for name in ('actors', 'fields', 'movement', 'net_guids', 'events'):\n"
                "    pq.write_table(pa.table({'value': [1]}), stage / (name + '.parquet'))\n"
                "os.replace(out, backup)\n"
                "os.replace(stage, out)\n"
                "shutil.rmtree(backup)\n"
                f"print({self.SUMMARY!r})\n",
                encoding="utf-8",
            )

        previous = Path.cwd()
        os.chdir(root)
        try:
            if fail:
                with self.assertRaises(SystemExit) as caught:
                    guard.measure(Path(sys.executable), replay, output)
                self.assertIn("exit 7", str(caught.exception))
                self.assertIn("deliberate export failure", str(caught.exception))
                result = None
            else:
                result = guard.measure(Path(sys.executable), replay, output)
        finally:
            os.chdir(previous)
        return output, sentinel, result

    def test_failed_export_preserves_previous_complete_output(self):
        _, sentinel, _ = self.run_fake_export(fail=True)

        self.assertEqual(sentinel.read_text(encoding="utf-8"),
                         "previous complete output")

    def test_successful_export_replaces_previous_complete_output(self):
        output, sentinel, result = self.run_fake_export(fail=False)

        self.assertFalse(sentinel.exists())
        self.assertTrue((output / "fields.parquet").is_file())
        self.assertEqual(result["parquet"]["fields"]["rows"], 1)


if __name__ == "__main__":
    unittest.main()
