import json
import sys
import tempfile
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_baseline_schemas as schemas  # noqa: E402


class BaselineSchemaTests(unittest.TestCase):
    def test_committed_baselines_are_schema_valid_and_cross_consistent(self):
        self.assertEqual(schemas.validate_repository(require_hashes=False), [])

    def test_legacy_export_without_hashes_fails_strict_validation(self):
        data = {
            "replay": "sample.vrf",
            "counters": {key: 0 for key in schemas.MAIN_COUNTERS},
            "parquet": {
                name: {"rows": 0, "bytes": 0} for name in schemas.MAIN_PARQUET
            },
        }
        path = Path("export_sample.json")

        strict = schemas.validate_export_baseline(path, data)
        migrating = schemas.validate_export_baseline(
            path, data, require_hashes=False
        )

        self.assertTrue(any("sha256" in problem for problem in strict), strict)
        self.assertEqual(migrating, [])

    def test_a_non_sha256_placeholder_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            data = {
                "replay": "sample.vrf",
                "counters": {key: 0 for key in schemas.MAIN_COUNTERS},
                "parquet": {
                    name: {"rows": 0, "bytes": 0, "sha256": "not-measured"}
                    for name in schemas.MAIN_PARQUET
                },
            }
            path = root / "export_sample.json"
            path.write_text(json.dumps(data), encoding="utf-8")

            problems = schemas.validate_export_baseline(path, data)

        self.assertTrue(any("sha256" in problem for problem in problems), problems)

    def test_unknown_baseline_json_fails_closed(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "unvalidated.json").write_text("{}", encoding="utf-8")
            problems = schemas.validate_repository(root, require_hashes=False)
        self.assertTrue(any("unvalidated.json" in p and "unknown" in p for p in problems),
                        problems)

    def test_bench_rejects_boolean_timing_and_non_replay_name(self):
        problems = schemas.validate_bench_baseline(
            Path("bench.json"), {"export": True, "replay": 7}
        )
        self.assertTrue(any("export" in p for p in problems), problems)
        self.assertTrue(any("replay" in p for p in problems), problems)

    def test_metrics_reject_wrong_replay_and_negative_or_wrong_typed_values(self):
        metrics = json.loads(
            (schemas.BASELINES / "metrics_builds.json").read_text(encoding="utf-8")
        )
        metrics["replays"]["12.10"] = 12
        metrics["metrics"]["12.11"]["kills"] = -1
        metrics["metrics"]["13.00"]["players"] = 1.5
        metrics["metrics"]["13.01"]["damage_dealt"] = "24139.22"
        metrics["metrics"]["13.02"]["team_score"] = {"Blue": 13, "Red": True}

        problems = schemas.validate_metrics_baseline(
            Path("metrics_builds.json"), metrics
        )

        joined = "\n".join(problems)
        for expected in ("replays.12.10", "kills", "players", "damage_dealt",
                         "team_score.Red"):
            self.assertIn(expected, joined)


if __name__ == "__main__":
    unittest.main()
