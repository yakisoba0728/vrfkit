"""Guards for the benchmark harness's judgement.

Timing is noisy and the harness cannot change that. What it must not do is turn
noise into a verdict, or let a genuinely faster run pass silently -- a run well
under the baseline means the baseline is stale, which is the same problem as a
regression pointed the other way.
"""
import json
import os
import unittest
from pathlib import Path
from unittest import mock


from support import TempDirTestCase, run_cli
import bench_export as bench
import check_baseline_schemas as schemas


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

    def test_nonfinite_or_nonpositive_timings_are_not_verdicts(self):
        for bad in (float("nan"), float("inf"), -float("inf"), 0.0, -1.0, True):
            for measured, baseline in ((bad, 1.0), (1.0, bad)):
                with self.subTest(measured=measured, baseline=baseline):
                    with self.assertRaises(ValueError):
                        bench.compare(measured, baseline, self.TOL)

    def test_invalid_tolerance_cannot_disable_regression_detection(self):
        for bad in (float("nan"), float("inf"), -float("inf"), -0.01, 1.0, True):
            with self.subTest(tolerance=bad):
                with self.assertRaises(ValueError):
                    bench.compare(10.0, 1.0, bad)

    def test_zero_tolerance_and_exact_boundaries_remain_valid(self):
        self.assertEqual(bench.compare(1.0, 1.0, 0.0)[0], "ok")
        for measured in (0.8, 1.2):
            self.assertEqual(bench.compare(measured, 1.0, 0.2)[0], "ok")


class MainTests(TempDirTestCase):
    """What `--update` writes, checked against the validator that reads it,
    and what a run compares against."""

    def setUp(self):
        self.root = self.tmp()
        self.exe = self.root / "vrfkit"
        self.exe.write_bytes(b"exe")
        self.baseline = self.root / "bench.json"

    def replay(self, name: str) -> Path:
        path = self.root / name
        path.write_bytes(b"replay")
        return path

    def run_bench(self, replay: Path, extra=(), seconds=1.0) -> tuple[int, str]:
        with mock.patch.object(bench, "time_export", return_value=[seconds]):
            return run_cli(bench.main, "--exe", self.exe, "--replay", replay, "--baseline", self.baseline,
                           "--repeats", "1", *extra, prog="bench_export.py", merged=True)[:2]

    def read(self) -> dict:
        return json.loads(self.baseline.read_text(encoding="utf-8"))

    def test_each_mode_records_its_own_slot_beside_the_other(self):
        """`export_checkpoints` is recorded only beside an `export` slot for the
        same replay: bench.json without `export` fails check_baseline_schemas."""
        cp_update = ["--checkpoints", "--update"]
        code, output = self.run_bench(self.replay("m.vrf"), cp_update)  # no bench.json
        self.assertEqual((code, self.baseline.exists()), (2, False), output)
        self.assertEqual(self.run_bench(self.replay("m.vrf"), ["--update"])[0], 0)
        self.assertEqual(self.read(), {"export": 1.0, "replay": "m.vrf"})
        self.assertEqual(schemas.validate_bench_baseline(self.baseline, self.read()), [])
        self.run_bench(self.replay("m.vrf"), cp_update, seconds=2.5)
        both = {"export": 1.0, "export_checkpoints": 2.5, "replay": "m.vrf"}
        self.assertEqual(self.read(), both)
        self.assertEqual(schemas.validate_bench_baseline(self.baseline, self.read()), [])
        code, output = self.run_bench(self.replay("new.vrf"), cp_update)  # another replay
        self.assertEqual((code, self.read()), (2, both), output)
        self.assertIn("--update without --checkpoints", output)

    def test_a_timing_is_never_kept_beside_another_replay_or_an_unknown_key(self):
        """A timing next to a replay it did not time is a plausible number
        for a measurement that never happened."""
        for replay, stored, want in (
                ("new.vrf", {"export": 1.0, "export_checkpoints": 2.0, "replay": "old.vrf"},
                 {"export": 7.5, "replay": "new.vrf"}),
                ("m.vrf", {"export": 1.0, "replay": "m.vrf", "export_debug": 2.0},
                 {"export": 7.5, "replay": "m.vrf"})):
            with self.subTest(replay=replay):
                self.baseline.write_text(json.dumps(stored), encoding="utf-8")
                self.run_bench(self.replay(replay), ["--update"], seconds=7.5)
                self.assertEqual(self.read(), want)

    def test_a_run_compares_against_its_own_slot(self):
        self.baseline.write_text(json.dumps({"export": 1.0, "export_checkpoints": 2.5,
                                             "replay": "m.vrf"}), encoding="utf-8")
        for extra, seconds, code in (((), 1.0, 0), (("--checkpoints",), 2.5, 0),
                                     (("--checkpoints",), 1.0, 1), ((), 2.5, 1)):
            with self.subTest(extra=extra, seconds=seconds):
                got, output = self.run_bench(self.replay("m.vrf"), extra, seconds)
                self.assertEqual(got, code, output)

    def test_a_missing_replay_or_slot_skips_unless_the_corpus_is_required(self):
        self.baseline.write_text(json.dumps({"export": 1.0, "replay": "m.vrf"}),
                                 encoding="utf-8")
        for replay in (self.root / "absent.vrf", self.replay("other.vrf")):
            for required, code, text in (("", 0, "SKIP:"), ("1", 2, "REQUIRED INPUT MISSING")):
                with self.subTest(replay=replay.name, required=required), \
                        mock.patch.dict(os.environ, {"VRFKIT_REQUIRE_CORPUS": required}):
                    got, output = self.run_bench(replay)
                    self.assertEqual(got, code, output)
                    self.assertIn(text, output)

    def test_a_missing_binary_or_no_samples_is_a_usage_error_not_a_skip(self):
        """A typo'd --exe must not read as a benchmark that passed, nor zero
        samples as an infinitely fast run."""
        with self.assertRaises(SystemExit) as raised:
            self.run_bench(self.replay("m.vrf"), ["--repeats", "0"])
        self.assertEqual(raised.exception.code, 2)
        self.exe = self.root / "relase" / "vrfkit"
        code, output = self.run_bench(self.replay("m.vrf"))
        self.assertEqual(code, 2)
        self.assertIn("build the release binary first", output)

    def test_invalid_tolerance_is_rejected_before_spawning(self):
        for value in ("nan", "inf", "-0.1", "1"):
            with self.subTest(value=value), mock.patch.object(bench, "time_export") as timed:
                with self.assertRaises(SystemExit) as raised:
                    self.run_bench(self.replay("m.vrf"), ["--tolerance", value])
                self.assertEqual(raised.exception.code, 2)
                timed.assert_not_called()

    def test_invalid_samples_cannot_poison_a_baseline(self):
        for seconds in (float("nan"), float("inf"), 0.0, -1.0):
            with self.subTest(seconds=seconds):
                code, output = self.run_bench(self.replay("m.vrf"), ["--update"], seconds)
                self.assertEqual(code, 2, output)
                self.assertFalse(self.baseline.exists())

    def test_invalid_stored_timing_is_rejected_before_spawning(self):
        for seconds in (float("nan"), float("inf"), 0.0, -1.0, True):
            with self.subTest(seconds=seconds):
                self.baseline.write_text(json.dumps({"export": seconds, "replay": "m.vrf"}), encoding="utf-8")
                with mock.patch.object(bench, "time_export") as timed:
                    code, output = run_cli(bench.main, "--exe", self.exe,
                        "--replay", self.replay("m.vrf"), "--baseline", self.baseline,
                        prog="bench_export.py", merged=True)[:2]
                self.assertEqual(code, 2, output)
                timed.assert_not_called()

    def test_submillisecond_measurement_stays_positive_when_recorded(self):
        code, output = self.run_bench(self.replay("m.vrf"), ["--update"], 0.0000123)
        self.assertEqual(code, 0, output)
        self.assertEqual(self.read()["export"], 0.0000123)
        self.assertEqual(schemas.validate_bench_baseline(self.baseline, self.read()), [])

    def test_missing_samples_are_not_a_measurement(self):
        for samples in ([], [1.0, 2.0]):
            with self.subTest(samples=samples), mock.patch.object(bench, "time_export", return_value=samples):
                code, output = run_cli(bench.main, "--exe", self.exe,
                    "--replay", self.replay("m.vrf"), "--baseline", self.baseline,
                    "--repeats", "1", "--update", prog="bench_export.py", merged=True)[:2]
                self.assertEqual(code, 2, output)
                self.assertFalse(self.baseline.exists())
