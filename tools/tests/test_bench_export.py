"""Guards for the benchmark harness's judgement.

Timing is noisy and the harness cannot change that. What it must not do is turn
noise into a verdict, or let a genuinely faster run pass silently -- a run well
under the baseline means the baseline is stale, which is the same problem as a
regression pointed the other way.
"""
import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import bench_export as bench  # noqa: E402
import check_baseline_schemas as schemas  # noqa: E402


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


class BaselineKeyContractTests(unittest.TestCase):
    """The generator must not write a key its own repo's validator rejects.

    `--checkpoints --update` wrote `export_checkpoints` into bench.json, and
    `check_baseline_schemas.py` -- which the pre-PR sweep in CONTRIBUTING.md
    runs -- checks that file's keys for EQUALITY with `{export, replay}`. So
    following this tool's own printed advice ("record one with --update")
    produced a committed baseline that failed the sweep, with the error naming
    the baseline rather than the tool that wrote it.

    The validator is the side that is right: bench.json is committed, one
    replay's timing is what it is for, and a schema that enumerates its keys is
    the thing that catches an unknown key rather than skipping it. So this tool
    refuses instead.
    """

    def test_the_key_set_matches_what_the_validator_accepts(self):
        """Read the expectation off the validator, not a copy of it.

        `validate_bench_baseline` is the authority. Asserting against a
        hand-copied set here would drift in the same step as the bug -- the
        shape of Defect 1 in this same fix.
        """
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "bench.json"
            good = {key: 1.0 if key == "export" else "m.vrf"
                    for key in bench.BASELINE_KEYS}
            self.assertEqual(
                schemas.validate_bench_baseline(path, good), [],
                "the validator rejects the exact key set bench_export writes",
            )

    def test_the_validator_rejects_the_key_this_tool_used_to_write(self):
        """Pins the defect itself: `export_checkpoints` is not acceptable."""
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "bench.json"
            problems = schemas.validate_bench_baseline(
                path, {"export": 1.0, "replay": "m.vrf",
                       "export_checkpoints": 2.0},
            )
            self.assertTrue(problems)
            self.assertIn("export_checkpoints", " ".join(problems))

    def test_checkpoints_is_not_a_recordable_key(self):
        self.assertNotIn(bench.TIMING_KEYS[True], bench.BASELINE_KEYS)
        self.assertIn(bench.TIMING_KEYS[False], bench.BASELINE_KEYS)


class UpdateTests(unittest.TestCase):
    """What `--update` writes, checked against the validator that reads it."""

    def setUp(self):
        self._temp = tempfile.TemporaryDirectory()
        self.addCleanup(self._temp.cleanup)
        self.root = Path(self._temp.name)
        self.exe = self.root / "vrfkit"
        self.exe.write_bytes(b"exe")
        self.baseline = self.root / "bench.json"

    def replay(self, name: str) -> Path:
        path = self.root / name
        path.write_bytes(b"replay")
        return path

    def run_bench(self, replay: Path, extra=(), seconds=1.0) -> int:
        argv = sys.argv
        sys.argv = [
            "bench_export.py",
            "--exe", str(self.exe),
            "--replay", str(replay),
            "--baseline", str(self.baseline),
            "--repeats", "1",
            *extra,
        ]
        try:
            with mock.patch.object(bench, "time_export", return_value=[seconds]):
                return bench.main()
        finally:
            sys.argv = argv

    def read(self) -> dict:
        return json.loads(self.baseline.read_text(encoding="utf-8"))

    def test_a_plain_update_writes_a_baseline_the_validator_accepts(self):
        code = self.run_bench(self.replay("m.vrf"), ["--update"])
        self.assertEqual(code, 0)
        data = self.read()
        self.assertEqual(set(data), set(bench.BASELINE_KEYS))
        self.assertEqual(
            schemas.validate_bench_baseline(self.baseline, data), [])

    def test_checkpoints_update_is_refused_and_writes_nothing(self):
        """Refusing is the fix, and refusing must not leave a file behind."""
        code = self.run_bench(self.replay("m.vrf"), ["--checkpoints", "--update"])
        self.assertEqual(code, 2)
        self.assertFalse(
            self.baseline.exists(),
            "the refused run still wrote a baseline",
        )

    def test_checkpoints_update_does_not_corrupt_an_existing_baseline(self):
        self.run_bench(self.replay("m.vrf"), ["--update"])
        before = self.read()
        code = self.run_bench(self.replay("m.vrf"), ["--checkpoints", "--update"])
        self.assertEqual(code, 2)
        self.assertEqual(self.read(), before)

    def test_recording_a_new_replay_does_not_keep_the_old_timing(self):
        """`replay` and the timing beside it must come from the same run.

        The old code merged into whatever the file held, so `--update` against
        a different replay replaced `replay` and left the PREVIOUS replay's
        `export` seconds under the new name -- a plausible number attributed to
        a measurement that never happened, which is the one thing this repo's
        doctrine forbids outright.
        """
        self.run_bench(self.replay("old.vrf"), ["--update"], seconds=1.0)
        self.assertEqual(self.read()["export"], 1.0)

        self.run_bench(self.replay("new.vrf"), ["--update"], seconds=7.5)
        data = self.read()
        self.assertEqual(data["replay"], "new.vrf")
        self.assertEqual(
            data["export"], 7.5,
            "the timing does not belong to the replay named beside it",
        )

    def test_a_stale_unknown_key_is_dropped_rather_than_carried_forward(self):
        """A baseline already carrying the bad key is repaired by --update.

        Someone who ran the old `--checkpoints --update` has an invalid
        bench.json; a merge would preserve the key forever.
        """
        self.baseline.write_text(
            json.dumps({"export": 1.0, "replay": "old.vrf",
                        "export_checkpoints": 2.0}) + "\n",
            encoding="utf-8",
        )
        self.run_bench(self.replay("new.vrf"), ["--update"], seconds=3.0)
        data = self.read()
        self.assertNotIn("export_checkpoints", data)
        self.assertEqual(
            schemas.validate_bench_baseline(self.baseline, data), [])

    def test_checkpoints_without_update_still_reports_the_timing(self):
        """Refusing to RECORD must not stop the tool from measuring."""
        argv = sys.argv
        sys.argv = [
            "bench_export.py",
            "--exe", str(self.exe),
            "--replay", str(self.replay("m.vrf")),
            "--baseline", str(self.baseline),
            "--repeats", "1",
            "--checkpoints",
        ]
        buf = io.StringIO()
        try:
            with mock.patch.object(bench, "time_export",
                                    return_value=[2.5]) as timer:
                with contextlib.redirect_stdout(buf):
                    code = bench.main()
        finally:
            sys.argv = argv
        self.assertEqual(code, 0)
        timer.assert_called_once()
        self.assertIn("export_checkpoints: median 2.500s", buf.getvalue(),
                       buf.getvalue())


if __name__ == "__main__":
    unittest.main()
