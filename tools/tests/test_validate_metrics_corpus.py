"""Guards for the metrics cross-validation run.

Two independent ways this reported success it had not earned.

Its exit status was `return 0` on any run where at least one replay completed.
A corpus where nineteen of twenty replays died at export and the twentieth
disagreed on every section still exited 0.

And its output directories persist between runs. `compute_metrics.py` is
invoked without `-o`, so the comparison reads `metrics.json` from inside the
bundle directory -- and if a run leaves that file behind, the NEXT run reads it
whenever compute_metrics exits 0 without writing. `check_export_baseline.py`
already states the rule: "Exporting over a previous run would leave a file the
exporter has stopped writing sitting there with last run's contents".
"""
import sys
import tempfile
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import validate_metrics_corpus as guard  # noqa: E402

OK = {"id": "a", "stage": "ok", "elapsed_s": 1.0, "sections": {"combat": "EXACT"}}


class FreshDirTests(unittest.TestCase):
    def test_a_stale_file_does_not_survive_into_the_next_run(self):
        import tempfile
        with tempfile.TemporaryDirectory() as parent:
            target = Path(parent) / "xval" / "some-id"
            target.mkdir(parents=True)
            stale = target / "metrics.json"
            stale.write_text('{"combat": "from the previous run"}',
                             encoding="utf-8")

            guard.fresh_dir(target)

            self.assertTrue(target.is_dir())
            self.assertFalse(stale.exists())

    def test_a_directory_that_does_not_exist_yet_is_created(self):
        import tempfile
        with tempfile.TemporaryDirectory() as parent:
            target = Path(parent) / "never" / "existed"
            guard.fresh_dir(target)
            self.assertTrue(target.is_dir())


class UnsafeReplayIdTests(unittest.TestCase):
    """An untrusted --only value must never become an rmtree target."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.saved = {
            name: getattr(guard, name)
            for name in ("REPO", "EXPORTS", "VRF_DIR", "COMPUTE", "VRFKIT", "ADAPTER")
        }
        guard.REPO = self.root / "repo"
        guard.EXPORTS = self.root / "references"
        guard.VRF_DIR = self.root / "replays"
        guard.COMPUTE = self.root / "compute_metrics.py"
        guard.VRFKIT = Path(sys.executable)
        guard.ADAPTER = self.root / "to_valplay_bundle.py"
        for directory in (guard.REPO, guard.EXPORTS, guard.VRF_DIR):
            directory.mkdir(parents=True)

    def tearDown(self):
        for name, value in self.saved.items():
            setattr(guard, name, value)
        self.temp.cleanup()

    def _assert_rejected_without_deleting(self, replay_id: str, victim: Path):
        victim.mkdir(parents=True)
        sentinel = victim / "keep.txt"
        sentinel.write_text("owned by someone else", encoding="utf-8")

        result = guard.process(replay_id)

        self.assertEqual(result["stage"], "input", result)
        self.assertTrue(sentinel.is_file(), f"unsafe id deleted {sentinel}")

    def test_parent_traversal_is_rejected_before_output_cleanup(self):
        victim = guard.REPO / "out" / "outside"
        self._assert_rejected_without_deleting("../outside", victim)

    def test_absolute_replay_id_is_rejected_before_output_cleanup(self):
        victim = self.root / "absolute-victim"
        self._assert_rejected_without_deleting(str(victim.resolve()), victim)

    def test_missing_replay_and_reference_preserve_previous_outputs(self):
        export = guard.REPO / "out" / "xval" / "missing"
        bundle = guard.REPO / "out" / "xval_bundle" / "missing"
        for directory in (export, bundle):
            directory.mkdir(parents=True)
            (directory / "keep.txt").write_text("old complete run", encoding="utf-8")

        result = guard.process("missing")

        self.assertEqual(result["stage"], "input", result)
        self.assertTrue((export / "keep.txt").is_file())
        self.assertTrue((bundle / "keep.txt").is_file())


class SubprocessTimeoutTests(unittest.TestCase):
    def test_timeout_is_returned_as_a_controlled_stage_error(self):
        result, error = guard.run_stage(
            [sys.executable, "-c", "import time; time.sleep(1)"],
            timeout=0.01,
        )
        self.assertIsNone(result)
        self.assertIn("timeout", error.lower())


class FailureTests(unittest.TestCase):
    def test_a_run_where_everything_completed_has_no_failures(self):
        self.assertEqual(guard.failures([OK]), [])

    def test_a_replay_that_died_is_a_failure(self):
        results = [OK, {"id": "b", "stage": "export", "error": "boom"}]
        problems = guard.failures(results)
        self.assertEqual(len(problems), 1)
        self.assertIn("b", problems[0])
        self.assertIn("export", problems[0])

    def test_every_dead_replay_is_named_not_just_the_first(self):
        results = [{"id": "b", "stage": "export", "error": "boom"},
                   {"id": "c", "stage": "metrics", "error": "boom"}]
        self.assertEqual(len(guard.failures(results)), 2)


if __name__ == "__main__":
    unittest.main()
