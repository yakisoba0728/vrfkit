import copy
import gzip
import hashlib
import json
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import compare_movement_provenance as comparator


HEADER = {"type": "movement_provenance_header", "schema_version": 1,
          "float_precision": "f32"}


def rust_move(index=0, **changes):
    row = {"type": "move", "initial_packet_id": 7, "completion_packet_id": 41,
           "channel_index": 2, "actor_net_guid": 11, "object_net_guid": None,
           "bunch_index": 35, "block_index": 0, "rpc_index": 0, "rpc_handle": 4,
           "array_ordinal": 0, "update_ordinal": 0, "update_index": 3,
           "component_ordinal": 0, "move_index": index, "character_net_guid": 100,
           "timestamp": 500 + index, "movement_state": 0, "move_type": 1,
           "pos_x": 13.25, "pos_y": -25.5, "pos_z": 1.0, "yaw": 90.0,
           "pitch": 45.0, "vel_x": 10.0, "vel_y": -0.0, "vel_z": 0.0}
    row.update(changes)
    return row


def reference_move(index=1, **changes):
    row = {"type": "remote_character_movement", "packet_id": 7,
           "channel": 2, "actor_net_guid": 11, "object_net_guid": 11,
           "update_index": 3, "move_index": index, "shooter_character_net_guid": 100,
           "timestamp": 500 + index, "movement_state": 0, "mode_flags": 0,
           "move_type": 1, "position": {"x": 13.25, "y": -25.5, "z": 1.0},
           "yaw": 90.0, "pitch": 45.0,
           "velocity": {"x": 10.0, "y": -0.0, "z": 0.0}, "error_sentinel": False}
    row.update(changes)
    return row


def summary(rows):
    return {"type": "movement_provenance_summary", "movement_rows": rows,
            "movement_rpc_errors": 0, "lost_content_blocks": 0, "partial_errors": 0,
            "unfinished_partials": 0, "block_verdict": "Passed", "skipped_checkpoint_chunks": 6}


class MovementProvenanceComparisonTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.reference = self.root / "reference.ndjson"
        self.rust = self.root / "rust.jsonl"
        self.work = self.root / "work"
        self.work.mkdir()

    def tearDown(self):
        self.temporary.cleanup()

    def write(self, *, reference=None, rust=None, header=None, footer=True):
        reference = [reference_move()] if reference is None else reference
        rust = [rust_move(0), rust_move(1)] if rust is None else rust
        objects = [copy.deepcopy(HEADER if header is None else header), *rust]
        if footer:
            objects.append(summary(len(rust)))
        self.reference.write_text("".join(json.dumps(row) + "\n" for row in reference), encoding="utf-8")
        self.rust.write_text("".join(json.dumps(row) + "\n" for row in objects), encoding="utf-8")

    def compare(self, **kwargs):
        return comparator.compare(self.reference, self.rust, work_dir=self.work, **kwargs)

    def test_complete_bijection_uses_observed_initial_packet_and_all_moves(self):
        self.write()
        report = self.compare()
        self.assertEqual(report["status"], "pass")
        self.assertEqual(report["rust"]["rows"], 2)
        self.assertEqual(report["rust"]["fragmented_move_rows"], 2)
        self.assertEqual(report["reference"]["represented_moves"], 2)
        self.assertEqual(report["value_comparison"]["matched_updates"], 1)
        self.assertEqual(report["reference"]["logical_content_sha256"],
                         hashlib.sha256(self.reference.read_bytes()).hexdigest())
        self.assertFalse(list(self.work.iterdir()))

    def test_completion_packet_or_fixed_offset_is_not_a_join(self):
        for packet in (9, 41):
            with self.subTest(packet=packet):
                self.write(reference=[reference_move(packet_id=packet)])
                report = self.compare()
                self.assertEqual(report["status"], "fail")
                self.assertEqual(report["missing_reference_updates"], 1)
                self.assertEqual(report["extra_rust_components"], 1)

    def test_values_never_select_identity(self):
        for changed in ({"timestamp": 900}, {"shooter_character_net_guid": 200}):
            with self.subTest(changed=changed):
                self.write(reference=[reference_move(**changed)])
                report = self.compare()
                self.assertEqual(report["status"], "fail")
                self.assertEqual(report["value_comparison"]["matched_updates"], 1)
                self.assertEqual(sum(report["value_comparison"]["mismatch_counts"].values()), 1)

    def test_f32_quantization_is_explicit_and_raw_delta_retained(self):
        self.write(reference=[reference_move(position={"x": 13.25000001, "y": -25.5, "z": 1.0})])
        report = self.compare()
        self.assertEqual(report["status"], "pass")
        self.assertGreater(report["value_comparison"]["maximum_absolute_reference_delta"]["pos_x"], 0)

    def test_signed_zero_difference_fails_bits_comparison(self):
        self.write(reference=[reference_move(velocity={"x": 10.0, "y": 0.0, "z": 0.0})])
        report = self.compare()
        self.assertEqual(report["status"], "fail")
        self.assertEqual(report["value_comparison"]["mismatch_counts"]["vel_y_bits"], 1)

    def test_duplicate_reference_identity_is_ambiguous_even_when_values_equal(self):
        self.write(reference=[reference_move(), reference_move()])
        report = self.compare()
        self.assertEqual(report["status"], "fail")
        self.assertEqual(report["reference"]["duplicate_identities"], {"groups": 1, "extra_rows": 1})
        self.assertEqual(report["value_comparison"]["status"], "not_run_ambiguous_identity")

    def test_repeated_update_components_are_never_collapsed(self):
        for changes in ({"array_ordinal": 1}, {"update_ordinal": 1}, {"component_ordinal": 1},
                        {"rpc_index": 1}):
            with self.subTest(changes=changes):
                self.write(rust=[rust_move(0), rust_move(1), rust_move(0, **changes)])
                report = self.compare()
                self.assertEqual(report["status"], "fail")
                self.assertEqual(report["rust"]["duplicate_identities"]["groups"], 1)

    def test_explicit_subobject_zero_is_not_the_actor(self):
        self.write(reference=[reference_move(object_net_guid=0)],
                   rust=[rust_move(0, object_net_guid=0), rust_move(1, object_net_guid=0)])
        self.assertEqual(self.compare()["status"], "pass")
        self.write(rust=[rust_move(0, object_net_guid=0), rust_move(1, object_net_guid=0)])
        self.assertEqual(self.compare()["status"], "fail")

    def test_missing_nullable_identity_is_invalid(self):
        row = rust_move(0)
        del row["object_net_guid"]
        self.write(rust=[row])
        with self.assertRaisesRegex(comparator.ComparisonError, "missing provenance member"):
            self.compare()

    def test_schema_version_requires_an_integer(self):
        for version in (True, 1.0, 2):
            with self.subTest(version=version):
                self.write(header={**HEADER, "schema_version": version})
                with self.assertRaisesRegex(comparator.ComparisonError, "header"):
                    self.compare()

    def test_move_indices_are_contiguous_and_reset_only_per_component(self):
        for rows in ([rust_move(1)], [rust_move(0), rust_move(2)], [rust_move(0), rust_move(0)]):
            with self.subTest(rows=len(rows)):
                self.write(rust=rows)
                with self.assertRaisesRegex(comparator.ComparisonError, "contiguous"):
                    self.compare()

    def test_earlier_move_character_cannot_change_within_component(self):
        self.write(rust=[rust_move(0, character_net_guid=101), rust_move(1)])
        with self.assertRaisesRegex(comparator.ComparisonError, "character identity changes"):
            self.compare()

    def test_context_cannot_change_within_component(self):
        self.write(rust=[rust_move(0, block_index=1), rust_move(1)])
        with self.assertRaisesRegex(comparator.ComparisonError, "source identity changes"):
            self.compare()

    def test_reappearing_component_is_rejected(self):
        self.write(rust=[rust_move(0), rust_move(0, rpc_index=1), rust_move(0)])
        with self.assertRaisesRegex(comparator.ComparisonError, "emission order"):
            self.compare()

    def test_reference_move_index_must_equal_last_observed_move(self):
        self.write(reference=[reference_move(0)])
        report = self.compare()
        self.assertEqual(report["status"], "fail")
        self.assertEqual(report["value_comparison"]["mismatch_counts"]["move_count"], 1)

    def test_component_above_reference_cap_is_unsupported(self):
        self.write(rust=[rust_move(i) for i in range(17)])
        with self.assertRaisesRegex(comparator.ComparisonError, "16-move"):
            self.compare()

    def test_truncated_reader_output_is_never_a_success(self):
        self.write(footer=False)
        with self.assertRaisesRegex(comparator.ComparisonError, "terminal Reader summary"):
            self.compare()

    def test_footer_must_be_final_and_its_counts_and_quality_must_pass(self):
        for field, value in (("movement_rows", 1), ("movement_rpc_errors", 1),
                             ("lost_content_blocks", 1), ("partial_errors", 1),
                             ("unfinished_partials", 1), ("block_verdict", "NoContentBlocks")):
            with self.subTest(field=field):
                self.write()
                objects = [json.loads(line) for line in self.rust.read_text(encoding="utf-8").splitlines()]
                objects[-1][field] = value
                self.rust.write_text("".join(json.dumps(row) + "\n" for row in objects), encoding="utf-8")
                with self.assertRaises(comparator.ComparisonError):
                    self.compare()
        self.write()
        with self.rust.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(rust_move(0)) + "\n")
        with self.assertRaisesRegex(comparator.ComparisonError, "follows"):
            self.compare()

    def test_bad_numeric_types_nonfinite_and_unrepresentable_rust_values_fail(self):
        for changes in ({"pos_x": True}, {"pos_x": float("nan")}, {"pos_x": float("inf")},
                        {"pos_x": 1.1}, {"pos_x": 1e300}, {"timestamp": True},
                        {"initial_packet_id": -1}, {"initial_packet_id": 100}):
            with self.subTest(changes=changes):
                self.write(rust=[rust_move(0, **changes)])
                with self.assertRaises(comparator.ComparisonError):
                    self.compare()

    def test_reference_unknown_sentinel_flags_and_variant_are_unsupported(self):
        for changes in ({"error_sentinel": None}, {"error_sentinel": 0}, {"mode_flags": 1},
                        {"move_type": 0}, {"velocity": None}, {"timestamp": -1}):
            with self.subTest(changes=changes):
                self.write(reference=[reference_move(**changes)])
                with self.assertRaises(comparator.ComparisonError):
                    self.compare()

    def test_empty_comparison_cannot_pass(self):
        self.write(reference=[], rust=[])
        self.assertEqual(self.compare()["status"], "fail")

    def test_extra_and_missing_updates_fail(self):
        self.write(reference=[])
        report = self.compare()
        self.assertEqual(report["extra_rust_components"], 1)
        self.assertEqual(report["status"], "fail")
        self.write(rust=[])
        report = self.compare()
        self.assertEqual(report["missing_reference_updates"], 1)
        self.assertEqual(report["status"], "fail")

    def test_gzip_inputs_have_logical_content_hashes(self):
        self.write()
        expected = self.compare()
        for name in ("rust", "reference"):
            path = getattr(self, name)
            compressed = path.with_suffix(path.suffix + ".gz")
            compressed.write_bytes(gzip.compress(path.read_bytes()))
            setattr(self, name, compressed)
        actual = self.compare()
        self.assertEqual(actual["status"], "pass")
        for name in ("rust", "reference"):
            self.assertEqual(actual[name]["logical_content_sha256"], expected[name]["logical_content_sha256"])
            self.assertTrue(actual[name]["compressed"])

    def test_line_and_row_limits_fail_closed_and_clean_temporary_storage(self):
        self.write()
        for kwargs in ({"max_rows": 1}, {"max_line_bytes": 100}):
            with self.subTest(kwargs=kwargs):
                with self.assertRaises(comparator.ComparisonError):
                    self.compare(**kwargs)
                self.assertFalse(list(self.work.iterdir()))

    def test_database_limit_fails_and_cleans_up(self):
        self.write(rust=[rust_move(0, rpc_index=i, update_index=i) for i in range(2000)])
        with self.assertRaises(sqlite3.DatabaseError):
            self.compare(max_db_bytes=65536)
        self.assertFalse(list(self.work.iterdir()))

    def test_duplicate_json_members_are_rejected(self):
        self.write()
        self.rust.write_text(self.rust.read_text(encoding="utf-8").replace(
            '"move_index": 0', '"move_index": 0, "move_index": 0'), encoding="utf-8")
        with self.assertRaisesRegex(comparator.ComparisonError, "invalid JSON"):
            self.compare()

    def test_cli_refuses_input_output_alias(self):
        self.write()
        before = self.reference.read_bytes()
        self.assertEqual(comparator.main([str(self.reference), str(self.rust), "--out", str(self.reference)]), 2)
        self.assertEqual(self.reference.read_bytes(), before)

    def test_cli_reports_failure_with_nonzero_exit_and_no_identifiers(self):
        self.write(reference=[reference_move(timestamp=999)])
        out = self.root / "report.json"
        self.assertEqual(comparator.main([str(self.reference), str(self.rust), "--out", str(out)]), 1)
        report = json.loads(out.read_text(encoding="utf-8"))
        self.assertEqual(report["status"], "fail")
        self.assertNotIn("shooter_character_net_guid", out.read_text(encoding="utf-8"))
        self.write(footer=False)
        self.assertEqual(comparator.main([str(self.reference), str(self.rust), "--out", str(out)]), 2)
        self.assertFalse(json.loads(out.read_text(encoding="utf-8"))["comparison_complete"])


if __name__ == "__main__":
    unittest.main()
