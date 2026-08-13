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


if __name__ == "__main__":
    unittest.main()
