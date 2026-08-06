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
