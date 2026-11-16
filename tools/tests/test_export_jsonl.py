"""JSONL is streamed, byte-preserving for selected cells, and atomic on failure."""

from __future__ import annotations

import io
import json
import math
import os
import sys
from unittest import mock

import pyarrow as pa
import pyarrow.parquet as pq

from support import TempDirTestCase, run_cli
import atomic_io
import export_jsonl


class ExportJsonlTests(TempDirTestCase):
    def fixture(self, table=None, *, name="fields", row_group_size=2):
        root = self.tmp()
        export = root / "export"
        export.mkdir()
        if table is None:
            table = pa.table({"ordinal": pa.array([0, 1, 2, 3, 4], type=pa.uint64())})
        pq.write_table(table, export / f"{name}.parquet", row_group_size=row_group_size)
        return root, export, root / "rows.jsonl"

    def read_lines(self, output):
        return [json.loads(line) for line in output.read_text(encoding="utf-8").splitlines()]

    def test_values_keep_raw_null_empty_unicode_and_exact_integer_tokens(self):
        table = pa.table({
            "raw": pa.array([b"\x00\xff\x80", b"", None], type=pa.binary()),
            "name": ["\uc6d0\ubcf8\n\"\\", "", None],
            "enabled": [True, False, None],
            "count": pa.array([2**64 - 1, 0, None], type=pa.uint64()),
            "signed": pa.array([-2**63, 2**63 - 1, None], type=pa.int64()),
            "value": pa.array([-0.0, 1.25, None], type=pa.float32()),
        })
        _, export, output = self.fixture(table)
        result = export_jsonl.export_jsonl(export, output, batch_size=1)
        lines = self.read_lines(output)
        header, rows = lines[0], [line["values"] for line in lines[1:]]
        self.assertEqual(result, {"rows": 3, "columns": 6, "table": "fields.parquet"})
        self.assertEqual(header["type"], "schema")
        self.assertEqual(header["format"], "vrfkit.parquet-jsonl")
        self.assertEqual(header["version"], 1)
        self.assertEqual(header["rows"], 3)
        self.assertEqual(header["columns"][0], {
            "name": "raw", "arrow_type": "binary", "nullable": True, "encoding": "hex",
        })
        self.assertEqual([row["raw"] for row in rows], ["00ff80", "", None])
        self.assertEqual(bytes.fromhex(rows[0]["raw"]), b"\x00\xff\x80")
        self.assertEqual([row["name"] for row in rows], table["name"].to_pylist())
        self.assertEqual([row["count"] for row in rows], [2**64 - 1, 0, None])
        self.assertEqual([row["signed"] for row in rows], [-2**63, 2**63 - 1, None])
        self.assertEqual([row["enabled"] for row in rows], [True, False, None])
        self.assertEqual(math.copysign(1, rows[0]["value"]), -1)
        self.assertEqual(rows[2], {name: None for name in table.column_names})
        self.assertTrue(all(line["type"] == "row" for line in lines[1:]))
        self.assertEqual(len(output.read_bytes().splitlines()), 4)

    def test_dictionary_values_and_fixed_width_binary_use_explicit_encodings(self):
        table = pa.table({
            "label": pa.array(["same", None, "same"]).dictionary_encode(),
            "raw": pa.array([b"\x00\xff", None, b"\xff\x00"], type=pa.binary(2)),
        })
        _, export, output = self.fixture(table)
        export_jsonl.export_jsonl(export, output)
        lines = self.read_lines(output)
        self.assertEqual(lines[0]["columns"][0]["encoding"], "json_string")
        self.assertIn("dictionary", lines[0]["columns"][0]["arrow_type"])
        self.assertEqual(lines[0]["columns"][1]["encoding"], "hex")
        self.assertEqual([line["values"]["label"] for line in lines[1:]], ["same", None, "same"])

    def test_empty_table_still_has_a_complete_schema(self):
        table = pa.Table.from_arrays([pa.array([], type=pa.binary())],
                                    schema=pa.schema([pa.field("raw", pa.binary(), nullable=False)]))
        _, export, output = self.fixture(table)
        self.assertEqual(export_jsonl.export_jsonl(export, output)["rows"], 0)
        lines = self.read_lines(output)
        self.assertEqual(len(lines), 1)
        self.assertFalse(lines[0]["columns"][0]["nullable"])

    def test_null_type_retains_each_row_and_a_declared_null_encoding(self):
        _, export, output = self.fixture(pa.table({"missing": [None, None, None]}))
        export_jsonl.export_jsonl(export, output, batch_size=1)
        lines = self.read_lines(output)
        self.assertEqual(lines[0]["columns"][0]["encoding"], "json_null")
        self.assertEqual([line["values"] for line in lines[1:]], [{"missing": None}] * 3)

    def test_selected_table_and_columns_follow_requested_order(self):
        _, export, output = self.fixture()
        pq.write_table(pa.table({"a": [1, 2], "b": ["one", "two"], "unused": [0, 0]}),
                       export / "actors.parquet")
        code, out, err = run_cli(export_jsonl.main, "--export", export, "--out", output,
                                "--table", "actors.parquet", "--columns", "b", "a",
                                "--batch-size", 1)
        self.assertEqual((code, out, err), (0, "OK: 2 rows, 2 columns\n", ""))
        lines = self.read_lines(output)
        self.assertEqual([col["name"] for col in lines[0]["columns"]], ["b", "a"])
        self.assertEqual(list(lines[1]["values"]), ["b", "a"])
        self.assertEqual([line["values"] for line in lines[1:]],
                         [{"b": "one", "a": 1}, {"b": "two", "a": 2}])

    def test_row_envelope_cannot_collide_with_source_column_names(self):
        _, export, output = self.fixture(pa.table({"type": ["schema"], "values": ["original"]}))
        export_jsonl.export_jsonl(export, output)
        self.assertEqual(self.read_lines(output)[1],
                         {"type": "row", "values": {"type": "schema", "values": "original"}})

    def test_batches_are_written_before_requesting_more_and_readers_close(self):
        _, export, output = self.fixture(row_group_size=2)
        real_reader = pq.ParquetFile
        readers, steps, batch_sizes = [], [], []
        buffer = io.BytesIO()

        class Reader:
            def __init__(self, *args, **kwargs):
                self.reader = real_reader(*args, **kwargs)
                readers.append(self.reader)
                self.schema_arrow = self.reader.schema_arrow
                self.metadata = self.reader.metadata
                self.pre_buffer = kwargs["pre_buffer"]

            def __enter__(self):
                return self

            def __exit__(self, *args):
                self.reader.close()

            def read(self, *args, **kwargs):
                raise AssertionError("whole-table read is forbidden")

            def iter_batches(self, **kwargs):
                self_kwargs = kwargs
                for batch in self.reader.iter_batches(**kwargs):
                    steps.append(len(buffer.getvalue().splitlines()))
                    batch_sizes.append(batch.num_rows)
                    yield batch
                assert self_kwargs == {"batch_size": 2, "columns": ["ordinal"], "use_threads": False}
                assert self.pre_buffer is False

        with mock.patch.object(pq, "ParquetFile", Reader), mock.patch.object(
                atomic_io, "atomic_write_file", side_effect=lambda path, write: write(buffer)):
            export_jsonl.export_jsonl(export, output, batch_size=2)
        self.assertEqual(steps, [1, 3, 5])
        self.assertEqual(batch_sizes, [2, 2, 1])
        self.assertTrue(all(reader.closed for reader in readers))
        lines = [json.loads(line) for line in buffer.getvalue().splitlines()]
        self.assertEqual([line["values"]["ordinal"] for line in lines[1:]], list(range(5)))

    def test_nonfinite_later_rows_fail_atomically_without_printing_values(self):
        for value in (float("nan"), float("inf"), float("-inf")):
            with self.subTest(value=value):
                _, export, output = self.fixture(pa.table({"private": [1.0, 2.0, value]}))
                output.write_text("previous complete file\n", encoding="utf-8")
                code, out, err = run_cli(export_jsonl.main, "--export", export, "--out", output,
                                        "--batch-size", 1)
                self.assertEqual(code, 1)
                self.assertEqual(out, "")
                self.assertEqual(err, "FAILED: non-finite number at row 2, column private\n")
                self.assertEqual(output.read_text(encoding="utf-8"), "previous complete file\n")
                self.assertEqual(list(output.parent.glob(".rows.jsonl.*.tmp")), [])

    def test_midstream_read_failure_keeps_old_output_and_closes_reader(self):
        _, export, output = self.fixture()
        output.write_bytes(b"old")
        real_batches = pq.ParquetFile.iter_batches
        readers = []

        def broken_batches(reader, **kwargs):
            readers.append(reader)
            yield next(real_batches(reader, **kwargs))
            raise OSError("simulated later-page failure")

        with mock.patch.object(pq.ParquetFile, "iter_batches", broken_batches):
            with self.assertRaisesRegex(OSError, "later-page"):
                export_jsonl.export_jsonl(export, output, batch_size=1)
        self.assertEqual(output.read_bytes(), b"old")
        self.assertTrue(all(reader.closed for reader in readers))
        self.assertEqual(list(output.parent.glob(".rows.jsonl.*.tmp")), [])

    def test_incomplete_batch_iterator_cannot_publish_a_truncated_success(self):
        _, export, output = self.fixture()
        output.write_bytes(b"old")
        real_batches = pq.ParquetFile.iter_batches

        def incomplete_batches(reader, **kwargs):
            yield next(real_batches(reader, **kwargs))

        with mock.patch.object(pq.ParquetFile, "iter_batches", incomplete_batches):
            with self.assertRaisesRegex(ValueError, "row count"):
                export_jsonl.export_jsonl(export, output, batch_size=1)
        self.assertEqual(output.read_bytes(), b"old")

    def test_replace_failure_keeps_old_output_and_cleans_temporary_file(self):
        _, export, output = self.fixture()
        output.write_bytes(b"old")
        with mock.patch.object(atomic_io.os, "replace", side_effect=OSError("replace failed")):
            with self.assertRaisesRegex(OSError, "replace failed"):
                export_jsonl.export_jsonl(export, output)
        self.assertEqual(output.read_bytes(), b"old")
        self.assertEqual(list(output.parent.glob(".rows.jsonl.*.tmp")), [])

    def test_invalid_selections_types_and_batch_sizes_do_not_publish(self):
        _, export, output = self.fixture(pa.table({"ok": [1], "nested": [[1, 2]]}))
        output.write_bytes(b"old")
        options = [
            {"columns": ["missing"]}, {"columns": ["ok", "ok"]}, {"columns": []},
            {"columns": ["nested"]}, {"batch_size": 0}, {"batch_size": -1},
            {"batch_size": True}, {"batch_size": 1.5}, {"batch_size": sys.maxsize + 1},
            {"table": "../fields"},
            {"table": "..\\fields"}, {"table": "missing"},
        ]
        for kwargs in options:
            with self.subTest(kwargs=kwargs):
                with self.assertRaises(ValueError):
                    export_jsonl.export_jsonl(export, output, **kwargs)
                self.assertEqual(output.read_bytes(), b"old")
        # Unsupported unselected columns must not block a valid projection.
        self.assertEqual(export_jsonl.export_jsonl(export, output, columns=["ok"])["rows"], 1)

    def test_duplicate_input_names_are_rejected_without_overwriting_rows(self):
        table = pa.Table.from_arrays([pa.array([1]), pa.array([2])], names=["same", "same"])
        _, export, output = self.fixture(table)
        for columns in (None, ["same"]):
            with self.subTest(columns=columns), self.assertRaises(ValueError):
                export_jsonl.export_jsonl(export, output, columns=columns)
        self.assertFalse(output.exists())

    def test_cli_rejects_nonpositive_and_noninteger_batches(self):
        _, export, output = self.fixture()
        for value in ("0", "-1", "1.5", "word", str(sys.maxsize + 1)):
            with self.subTest(value=value), self.assertRaises(SystemExit) as error:
                run_cli(export_jsonl.main, "--export", export, "--out", output,
                        "--batch-size", value)
            self.assertEqual(error.exception.code, 2)
        self.assertFalse(output.exists())

    def test_export_tables_manifest_and_original_replay_paths_are_protected(self):
        root, export, output = self.fixture()
        other_table = export / "movement.parquet"
        other_table.write_bytes(b"other table")
        replay = root / "original.data"
        replay.write_bytes(b"original replay")
        manifest = export / "manifest.json"
        manifest.write_text(json.dumps({"source_file": str(replay)}), encoding="utf-8")
        for target in (export / "fields.parquet", other_table, manifest, replay,
                       root / "match.vrf", root / "match.REPLAY"):
            with self.subTest(target=target.name):
                before = target.read_bytes() if target.exists() else None
                with self.assertRaises(ValueError):
                    export_jsonl.export_jsonl(export, target)
                self.assertEqual(target.read_bytes() if target.exists() else None, before)
        export_jsonl.export_jsonl(export, output)
        self.assertTrue(output.exists())

    def test_replay_header_is_protected_even_with_an_unrelated_file_extension(self):
        root, export, _ = self.fixture()
        output = root / "unregistered.data"
        original = bytes.fromhex("dd ef f4 43") + b"synthetic replay header"
        output.write_bytes(original)
        with self.assertRaisesRegex(ValueError, "replay header"):
            export_jsonl.export_jsonl(export, output)
        self.assertEqual(output.read_bytes(), original)

    def test_cli_protection_failure_does_not_print_the_manifests_replay_path(self):
        root, export, _ = self.fixture()
        replay = root / "distinctive-private-source.data"
        replay.write_bytes(b"original replay")
        (export / "manifest.json").write_text(json.dumps({"source_file": str(replay)}),
                                              encoding="utf-8")
        code, out, err = run_cli(export_jsonl.main, "--export", export, "--out", replay)
        self.assertEqual((code, out), (1, ""))
        self.assertEqual(err, "FAILED: output aliases a protected input or implementation file\n")
        self.assertNotIn(replay.name, err)
        self.assertEqual(replay.read_bytes(), b"original replay")

    def test_symlink_aliases_are_refused_and_hardlinks_preserve_input_bytes(self):
        root, export, output = self.fixture()
        original = export / "fields.parquet"
        before = original.read_bytes()
        try:
            output.symlink_to(original)
        except OSError:
            self.skipTest("symlink creation unavailable on this platform")
        with self.assertRaises(ValueError):
            export_jsonl.export_jsonl(export, output)
        self.assertEqual(original.read_bytes(), before)
        output.unlink()
        os.link(original, output)
        export_jsonl.export_jsonl(export, output)
        self.assertEqual(original.read_bytes(), before)
        self.assertEqual(self.read_lines(output)[0]["rows"], 5)

    def test_manifest_errors_fail_before_publication(self):
        _, export, output = self.fixture()
        output.write_bytes(b"old")
        for document in ("{", "[]", '{"source_file": 123}', '{"source_file": ""}'):
            with self.subTest(document=document):
                (export / "manifest.json").write_text(document, encoding="utf-8")
                with self.assertRaises(ValueError):
                    export_jsonl.export_jsonl(export, output)
                self.assertEqual(output.read_bytes(), b"old")
