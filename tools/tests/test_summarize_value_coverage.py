"""Physical coverage counts rows once and never substitutes truthiness for null."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

import pyarrow as pa
import pyarrow.parquet as pq

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import summarize_value_coverage as coverage


class PhysicalCoverageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def write(self, directory, filename="fields.parquet"):
        directory.mkdir(exist_ok=True)
        pq.write_table(pa.table({
            "value_i64": pa.array([0, None, None, None, 7], type=pa.int64()),
            "value_f64": pa.array([None, None, None, None, 2.5], type=pa.float64()),
            "value_bool": pa.array([None, False, None, None, None], type=pa.bool_()),
            "value_str": pa.array([None, None, "", None, None], type=pa.string()),
        }), directory / filename)

    def test_null_union_and_checkpoint_denominators_are_independent(self):
        first, second = self.root / "first", self.root / "second"
        self.write(first)
        self.write(first, "checkpoint_fields.parquet")
        self.write(second)
        paths = coverage.discover([self.root, first])
        self.assertEqual(len(paths), 2)
        report = coverage.summarize(paths, 2)
        main = report["tables"]["fields"]
        cp = report["tables"]["checkpoint_fields"]
        self.assertTrue(report["complete"])
        self.assertEqual((main["rows"], main["typed_rows"], main["multi_value_rows"]), (10, 8, 2))
        self.assertEqual(main["typed_fraction"], 0.8)
        self.assertEqual((cp["exports_with_table"], cp["rows"], cp["typed_rows"]), (1, 5, 4))

    def test_bad_export_is_explicit_and_cli_fails(self):
        self.write(self.root / "good")
        broken = self.root / "broken"
        broken.mkdir()
        (broken / "fields.parquet").write_bytes(b"not parquet")
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = coverage.main([str(self.root)])
        report = json.loads(output.getvalue())
        self.assertEqual(code, 1)
        self.assertFalse(report["complete"])
        self.assertEqual(report["successful_exports"], 1)
        self.assertEqual(len(report["errors"]), 1)

    def test_empty_table_has_unknown_fraction_and_schema_is_required(self):
        empty = self.root / "fields.parquet"
        pq.write_table(pa.table({name: pa.array([], type=pa.int64())
                                for name in coverage.VALUE_COLUMNS}), empty)
        self.assertEqual(coverage.count_table(empty)["rows"], 0)
        report = coverage.summarize([self.root], 1)
        self.assertTrue(report["complete"])
        self.assertIsNone(report["tables"]["fields"]["typed_fraction"])
        self.assertIsNone(report["tables"]["checkpoint_fields"]["typed_fraction"])
        pq.write_table(pa.table({"different": [1]}), empty)
        with self.assertRaisesRegex(ValueError, "missing value columns"):
            coverage.count_table(empty)


if __name__ == "__main__":
    unittest.main()
