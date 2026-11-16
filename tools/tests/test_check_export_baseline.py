"""Guards for the export baseline pinner: the Parquet cross-checks, the
manifest agreement checks, the checkpoint GUID cross-check, and the refusal to
pin an unmeasured counter or a machine path. The summary patterns are
test_summary_counters.py's."""
import contextlib
import io
import json
import os
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

from support import TempDirTestCase, read_parquet, run_cli
import check_export_baseline as guard


def measurement(**overrides):
    counters = {key: 1 for key in guard.COUNTERS}
    counters.update(overrides)
    return {
        "counters": counters,
        "parquet": {name: {"rows": 1, "bytes": 100, "sha256": "a" * 64}
                    for name in guard.PARQUET_FILES},
    }


def checkpoint_measurement(**overrides):
    current = measurement()
    current["counters"].update({key: 1 for key in guard.CHECKPOINT_COUNTERS})
    current["counters"]["cp_literal_paths"] = 0
    current["parquet"].update({
        name: {"rows": 1, "bytes": 100, "sha256": "a" * 64}
        for name in guard.CHECKPOINT_PARQUET_FILES
    })
    current["counters"].update(overrides)
    return current


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

    def test_reward_opaque_counter_is_required_even_when_zero(self):
        reasons = guard.unpinnable(measurement(
            tracked_rewards_opaque_empty_variants=None))
        self.assertIn("tracked_rewards_opaque_empty_variants", " ".join(reasons))


class CrossCheckTests(TempDirTestCase):
    def test_partial_identity_includes_checkpoint_rows_only_when_present(self):
        current = measurement(partial_rows=2, cp_partial_rows=3)
        current["parquet"]["partials"]["rows"] = 5
        checks = guard.cross_check_identities(current["counters"], current["parquet"])
        self.assertIn(("Partial raw rows (main + checkpoint)", 5, 5), checks)
        current["parquet"]["partials"]["rows"] = 2
        self.assertTrue(any("Partial raw" in problem for problem in guard.cross_checks(current["counters"], current["parquet"])))

    def test_the_sink_fields_tally_is_each_pass_field_table_row_count(self):
        for key, table, label in (("fields_emitted", "fields", "Sink tally fields"),
                                  ("cp_fields_emitted", "checkpoint_fields",
                                   "Checkpoint sink fields")):
            with self.subTest(key=key):
                current = checkpoint_measurement(actor_closes=0, cp_partial_rows=0)
                self.assertEqual(guard.cross_checks(current["counters"], current["parquet"]), [])
                current["parquet"][table]["rows"] = 7
                problems = guard.cross_checks(current["counters"], current["parquet"])
                self.assertEqual(problems, [f"{label}: summary says 1, Parquet holds 7"])

    def test_a_summary_disagreeing_with_its_parquet_is_a_lie(self):
        current = measurement(net_guid_rows=99)
        lies = guard.cross_checks(current["counters"], current["parquet"])
        self.assertTrue(any("NetGUID rows" in l for l in lies))

    def test_an_absent_identity_counter_is_itself_a_failure(self):
        current = measurement(movement_rows=None)
        lies = guard.cross_checks(current["counters"], current["parquet"])
        self.assertTrue(any("did not print it" in l for l in lies))

    def test_checkpoint_actor_and_guid_identities_reject_counter_mismatches(self):
        current = checkpoint_measurement(cp_actor_rows_written=2, cp_net_guid_rows_written=3,
                                         cp_block_rows_written=5)
        current["parquet"]["checkpoint_actors"]["rows"] = 1
        current["parquet"]["checkpoint_net_guids"]["rows"] = 4
        problems = guard.cross_checks(current["counters"], current["parquet"])
        self.assertTrue(any("Checkpoint actors" in p for p in problems), problems)
        self.assertTrue(any("Checkpoint GUID rows" in p for p in problems), problems)
        self.assertTrue(any("Checkpoint blocks" in p for p in problems), problems)

    def test_checkpoint_schema_rows_must_match_reader_and_writer_counts(self):
        for table, parsed, written in (
            ("checkpoint_guid_entries", "cp_guid_entries", "cp_guid_entry_rows_written"),
            ("checkpoint_export_groups", "cp_group_records", "cp_export_group_rows_written"),
            ("checkpoint_export_fields", "cp_exported_fields", "cp_export_field_rows_written"),
        ):
            with self.subTest(table=table):
                current = checkpoint_measurement(actor_closes=0, cp_partial_rows=0)
                self.assertEqual(guard.cross_checks(current["counters"], current["parquet"]), [])
                # A dropped row with a matching writer counter is still wrong.
                current["counters"][written] = 0
                current["parquet"][table]["rows"] = 0
                problems = guard.cross_checks(current["counters"], current["parquet"])
                self.assertEqual(len(problems), 1, problems)
                self.assertIn("parsed", problems[0])
                current["counters"][parsed] = 0
                if parsed == "cp_guid_entries":
                    current["counters"]["cp_indexed_paths"] = 0
                    current["counters"]["cp_resolved_path_indices"] = 0
                self.assertEqual(guard.cross_checks(current["counters"], current["parquet"]), [])
                current["counters"][written] = None
                self.assertIn("did not print", " ".join(guard.cross_checks(
                    current["counters"], current["parquet"])))

    def test_guid_path_counters_reject_omission_and_arithmetic_mismatch(self):
        current = checkpoint_measurement(actor_closes=0, cp_partial_rows=0)
        self.assertEqual(guard.cross_checks(current["counters"], current["parquet"]), [])
        current["counters"]["cp_literal_paths"] = None
        self.assertIn("did not print", " ".join(guard.cross_checks(current["counters"], current["parquet"])))
        current = checkpoint_measurement(actor_closes=0, cp_partial_rows=0,
                                         cp_guid_entries=4, cp_literal_paths=2,
                                         cp_indexed_paths=1, cp_resolved_path_indices=0)
        problems = guard.cross_checks(current["counters"], current["parquet"])
        self.assertTrue(any("literals + indices" in problem for problem in problems), problems)
        self.assertTrue(any("resolved indices" in problem for problem in problems), problems)

    def test_checkpoint_measurement_requires_every_new_table_and_zero_dropped_actors(self):
        root = self.tmp()
        out = root / "out"
        out.mkdir()
        for name in (*guard.PARQUET_FILES, "checkpoint_fields", "checkpoint_net_guids"):
            pq.write_table(pa.table({"value": [1]}), out / f"{name}.parquet")
        (out / "manifest.json").write_text(json.dumps({"quality": {"checkpoints": {
            "checkpoint_actor_rows_dropped": 0,
            "checkpoint_path_resolution_mode": "preceding_literal_zero_based",
            "checkpoint_literal_paths": 1,
            "checkpoint_indexed_paths": 0,
            "checkpoint_resolved_path_indices": 0,
            "checkpoint_guid_entries": 1}}}), encoding="utf-8")
        with patch.object(guard.sc.subprocess, "run", return_value=SimpleNamespace(
                returncode=0, stdout="", stderr="")):
            with self.assertRaisesRegex(SystemExit, "checkpoint_actors.parquet"):
                guard.measure(Path("fake.exe"), root / "sample.vrf", out, checkpoints=True)

        (out / "manifest.json").write_text(json.dumps({"quality": {"checkpoints": {
            "checkpoint_actor_rows_dropped": 1,
            "checkpoint_path_resolution_mode": "preceding_literal_zero_based",
            "checkpoint_literal_paths": 1,
            "checkpoint_indexed_paths": 0,
            "checkpoint_resolved_path_indices": 0,
            "checkpoint_guid_entries": 1}}}), encoding="utf-8")
        self.assertIn("expected 0", " ".join(guard.checkpoint_manifest_errors(out)))

    def test_checkpoint_manifest_rejects_missing_and_mismatched_path_evidence(self):
        temp = self.tmp()
        manifest = temp / "manifest.json"
        manifest.write_text(json.dumps({"quality": {"checkpoints": {
            "checkpoint_actor_rows_dropped": 0}}}), encoding="utf-8")
        self.assertIn("omits required", " ".join(guard.checkpoint_manifest_errors(temp)))
        manifest.write_text(json.dumps({"quality": {"checkpoints": {
            "checkpoint_actor_rows_dropped": 0,
            "checkpoint_path_resolution_mode": "legacy_decimal",
            "checkpoint_literal_paths": 2,
            "checkpoint_indexed_paths": 1,
            "checkpoint_resolved_path_indices": 0,
            "checkpoint_guid_entries": 4}}}), encoding="utf-8")
        problems = guard.checkpoint_manifest_errors(temp)
        self.assertTrue(any("mode" in problem for problem in problems), problems)
        self.assertTrue(any("literals + indices" in problem for problem in problems), problems)
        self.assertTrue(any("resolved indices" in problem for problem in problems), problems)

    def test_manifest_path_counts_must_match_summary_and_have_integer_types(self):
        root = self.tmp()
        manifest = root / "manifest.json"
        cp = {"checkpoint_actor_rows_dropped": 0,
              "checkpoint_path_resolution_mode": "preceding_literal_zero_based",
              "checkpoint_literal_paths": 2, "checkpoint_indexed_paths": 1,
              "checkpoint_resolved_path_indices": 1, "checkpoint_guid_entries": 3}
        counters = {"cp_literal_paths": 2, "cp_indexed_paths": 1,
                    "cp_resolved_path_indices": 1, "cp_guid_entries": 3}
        manifest.write_text(json.dumps({"quality": {"checkpoints": cp}}), encoding="utf-8")
        self.assertEqual(guard.checkpoint_manifest_errors(root, counters), [])
        self.assertIn("disagrees", " ".join(guard.checkpoint_manifest_errors(
            root, dict(counters, cp_literal_paths=3, cp_guid_entries=4))))
        for value in (None, True, "2", 2.0, -1):
            with self.subTest(value=value):
                changed = dict(cp, checkpoint_literal_paths=value)
                manifest.write_text(json.dumps({"quality": {"checkpoints": changed}}), encoding="utf-8")
                self.assertIn("nonnegative integers", " ".join(guard.checkpoint_manifest_errors(root)))


#: Checkpoint GUID declarations, independent of how an index is encoded:
#: `(checkpoint_index, net_guid, wire outer, target)`, where `target` is a
#: literal path or an int k meaning "a reference to this checkpoint's k-th
#: literal". Both checkpoints use one wire ID, so only `checkpoint_index`
#: separates their tables. The shapes are chosen so that each wrong rule below
#: resolves at least one reference to a different path.
DECLARATIONS = (
    (0, 3, 0, "/Game/Maps/Ascent"),
    (0, 5, 3, 0),                  # Ascent
    (0, 7, 3, "/Game/A.A_C"),
    (0, 19, 3, "/Game/D.D_C"),
    (0, 9, 7, 1),                  # A.A_C; D.D_C if references were appended
    (0, 13, 3, 0),                 # Ascent; A.A_C if one-based
    (1, 11, 0, "/Game/B.B_C"),
    (1, 15, 11, "/Game/C.C_C"),
    (1, 21, 11, "/Game/E.E_C"),
    (1, 23, 11, "/Game/F.F_C"),
    (1, 12, 11, 0),                # dynamic GUID; B.B_C in a fresh table, F.F_C in a shared one
    (1, 16, 15, 1),                # C.C_C
    (1, 17, 0, "/Game/OnlyInCheckpoint"),
)
#: The main stream's final registry: net_guid -> (path, outer or None).
MAIN_GUIDS = {
    3: ("/Game/Maps/Ascent", None), 5: ("/Game/Maps/Ascent", 3),
    7: ("/Game/A.A_C", 3), 19: ("/Game/D.D_C", 3), 9: ("/Game/A.A_C", 7),
    13: ("/Game/Maps/Ascent", 3), 11: ("/Game/B.B_C", None),
    15: ("/Game/C.C_C", 11), 21: ("/Game/E.E_C", 11), 23: ("/Game/F.F_C", 11),
    12: ("/Game/B.B_C", 11), 16: ("/Game/C.C_C", 15),
    40: ("/Game/OnlyInMain", None),
}
GUID_ENTRY_SCHEMA = pa.schema([
    ("checkpoint_index", pa.uint32()), ("checkpoint_id", pa.string()),
    ("ordinal", pa.uint32()), ("net_guid", pa.uint32()),
    ("outer_net_guid", pa.uint32()), ("path_is_string", pa.bool_()),
    ("literal_path", pa.string()), ("name_index", pa.uint32()), ("flags", pa.uint8()),
])


def encode_guid_entries(rule="zero_based", declarations=DECLARATIONS):
    """Raw `checkpoint_guid_entries` rows as a serializer following `rule` writes them.

    `zero_based` is the rule the checkpoint reader implements. The others are
    the wrong rules the doc's negative controls measure: one-based positions,
    references appended to the table, and one table shared by every
    checkpoint instead of one per checkpoint.
    """
    rows, current, earlier_literals = [], None, 0
    for checkpoint, guid, outer, target in declarations:
        if checkpoint != current:
            if current is not None:
                earlier_literals += len(literal_slots)
            current, table, literal_slots = checkpoint, [], []
        row = {"checkpoint_index": checkpoint, "checkpoint_id": "cp",
               "ordinal": len(table), "net_guid": guid, "outer_net_guid": outer,
               "flags": 0}
        if isinstance(target, str):
            literal_slots.append(len(table))
            table.append(target)
            row.update(path_is_string=True, literal_path=target, name_index=None)
        else:
            index = {"zero_based": target, "one_based": target + 1,
                     "append_references": literal_slots[target],
                     "shared_table": earlier_literals + target}[rule]
            table.append(table[literal_slots[target]])
            row.update(path_is_string=False, literal_path=None, name_index=index)
        rows.append(row)
    return rows


def write_guid_tables(out, entries, main=MAIN_GUIDS, duplicate=None):
    """Write the two tables the cross-check joins; `net_guids.path` is
    dictionary-encoded, as the exporter writes it."""
    pq.write_table(pa.Table.from_pylist(entries, schema=GUID_ENTRY_SCHEMA),
                   out / "checkpoint_guid_entries.parquet")
    guids = sorted(main) + ([duplicate] if duplicate is not None else [])
    pq.write_table(pa.table({
        "net_guid": pa.array(guids, pa.uint32()),
        "path": pa.array([main[g][0] for g in guids]).dictionary_encode(),
        "outer_net_guid": pa.array([main[g][1] for g in guids], pa.uint32()),
    }), out / "net_guids.parquet")


class CheckpointGuidCrossCheckTests(TempDirTestCase):
    """The main stream is the independent side of the path-index rule's check."""

    def crosscheck(self, entries, main=MAIN_GUIDS, duplicate=None):
        out = self.tmp()
        write_guid_tables(out, entries, main, duplicate)
        return guard.checkpoint_guid_crosscheck(out)

    def test_the_readers_rule_agrees_with_the_main_stream(self):
        counts, errors = self.crosscheck(encode_guid_entries())
        self.assertEqual(errors, [])
        expected = dict.fromkeys(guard.GUID_CROSSCHECK_KEYS, 0)
        expected.update(indexed_joined=5, indexed_path_equal=5, indexed_outer_equal=5,
                        literal_joined=7, literal_path_equal=7, literal_outer_equal=7,
                        literal_unjoined=1)
        self.assertEqual(counts, expected)

    def test_rows_are_ordered_by_checkpoint_and_ordinal_not_file_order(self):
        rows = encode_guid_entries()
        counts, errors = self.crosscheck(list(reversed(rows)))
        self.assertEqual(errors, [])
        self.assertEqual((counts["indexed_path_equal"], counts["literal_path_equal"]), (5, 7))

    def assert_rejected(self, counts, errors, **expected):
        self.assertTrue(errors, counts)
        for key, value in expected.items():
            self.assertEqual(counts[key], value, key)

    def test_one_based_indices_are_rejected(self):
        counts, errors = self.crosscheck(encode_guid_entries("one_based"))
        self.assert_rejected(counts, errors, indexed_path_equal=0,
                             indexed_path_differs=4, indexed_unresolved=1)

    def test_indices_counting_appended_references_are_rejected(self):
        counts, errors = self.crosscheck(encode_guid_entries("append_references"))
        self.assert_rejected(counts, errors, indexed_path_equal=4, indexed_path_differs=1)
        self.assertIn("net_guid 9", " ".join(errors))

    def test_indices_into_a_table_not_reset_per_checkpoint_are_rejected(self):
        counts, errors = self.crosscheck(encode_guid_entries("shared_table"))
        self.assert_rejected(counts, errors, indexed_path_equal=3,
                             indexed_path_differs=1, indexed_unresolved=1)

    def test_an_edited_main_stream_path_is_rejected(self):
        main = dict(MAIN_GUIDS)
        main[9] = ("/Game/Other.Other_C", 7)
        counts, errors = self.crosscheck(encode_guid_entries(), main)
        self.assert_rejected(counts, errors, indexed_path_differs=1, indexed_path_equal=4)
        main = dict(MAIN_GUIDS)
        main[19] = ("/Game/Other.Other_C", 3)
        counts, errors = self.crosscheck(encode_guid_entries(), main)
        self.assert_rejected(counts, errors, literal_path_differs=1, indexed_path_differs=0)

    def test_an_index_past_the_preceding_literals_is_counted_not_raised(self):
        rows = encode_guid_entries()
        rows[4]["name_index"] = 99
        counts, errors = self.crosscheck(rows)
        self.assert_rejected(counts, errors, indexed_unresolved=1, indexed_joined=4)
        self.assertIn("index 99, 3 preceding literals", " ".join(errors))

    def test_nothing_joined_is_a_failure_not_a_vacuous_pass(self):
        without_indexed = {g: v for g, v in MAIN_GUIDS.items() if g not in (5, 9, 13, 12, 16)}
        literal_only = tuple(d for d in DECLARATIONS if isinstance(d[3], str))
        for name, entries, main in (
                ("no checkpoint rows", [], MAIN_GUIDS),
                ("indexed GUIDs absent from the main stream", encode_guid_entries(), without_indexed),
                ("literal entries only", encode_guid_entries(declarations=literal_only), MAIN_GUIDS)):
            with self.subTest(name):
                counts, errors = self.crosscheck(entries, main)
                self.assertEqual(counts["indexed_joined"], 0)
                self.assertIn("not compared at all", " ".join(errors))

    def test_a_repeated_main_guid_fails_even_when_both_rows_agree(self):
        counts, errors = self.crosscheck(encode_guid_entries(), duplicate=9)
        self.assert_rejected(counts, errors, main_duplicate_guids=1, indexed_path_differs=0)

    def test_outers_compare_by_an_explicit_rule_not_null_as_zero(self):
        for name, guid, main_outer, key in (
                ("main writes 0 where the checkpoint has no outer", 3, 0,
                 "literal_outer_presence_differs"),
                ("main has no outer where the checkpoint has one", 5, None,
                 "indexed_outer_presence_differs"),
                ("both have an outer and they differ", 9, 4, "indexed_outer_value_differs")):
            with self.subTest(name):
                main = dict(MAIN_GUIDS)
                main[guid] = (main[guid][0], main_outer)
                counts, errors = self.crosscheck(encode_guid_entries(), main)
                self.assert_rejected(counts, errors, **{key: 1})
                self.assertEqual(counts["indexed_path_differs"] + counts["literal_path_differs"], 0)

    def test_an_incomplete_raw_record_fails_before_paths_are_compared(self):
        def reference_with_literal(rows):
            rows[4]["literal_path"] = "/Game/A.A_C"

        def literal_with_index(rows):
            rows[0]["name_index"] = 0

        def null_outer(rows):
            rows[4]["outer_net_guid"] = None

        def gap(rows):
            del rows[10]

        def repeat(rows):
            rows.insert(10, dict(rows[10]))

        for change, key in ((reference_with_literal, "malformed_entries"),
                            (literal_with_index, "malformed_entries"),
                            (null_outer, "malformed_entries"),
                            (gap, "ordinal_errors"), (repeat, "ordinal_errors")):
            with self.subTest(change.__name__):
                rows = encode_guid_entries()
                change(rows)
                counts, errors = self.crosscheck(rows)
                self.assert_rejected(counts, errors, **{key: 1}, indexed_joined=0)
                self.assertIn("not a complete raw record", " ".join(errors))

    def test_a_missing_table_or_column_is_reported_not_raised(self):
        out = self.tmp()
        write_guid_tables(out, encode_guid_entries())
        (out / "net_guids.parquet").unlink()
        counts, errors = guard.checkpoint_guid_crosscheck(out)
        self.assertIn("cannot read", " ".join(errors))
        write_guid_tables(out, encode_guid_entries())
        entries = read_parquet(out / "checkpoint_guid_entries.parquet")
        pq.write_table(entries.drop_columns(["name_index"]),
                       out / "checkpoint_guid_entries.parquet")
        counts, errors = guard.checkpoint_guid_crosscheck(out)
        self.assertIn("cannot read", " ".join(errors))
        self.assertEqual(set(counts.values()), {0})

    def test_every_count_is_printed_including_zeros(self):
        counts, _ = self.crosscheck(encode_guid_entries())
        line = guard.format_guid_crosscheck(counts)
        for key in guard.GUID_CROSSCHECK_KEYS:
            self.assertIn(f"{key.replace('_', ' ')} {counts[key]},", line + ",")
        self.assertIn("indexed path differs 0,", line)

    def test_checkpoint_measurement_runs_every_export_check_and_prints_the_crosscheck(self):
        """`measure` fails on a path the main stream does not declare, on a
        checkpoint-only manifest count (`Trailing bytes`) the summary
        misreports, and on dropped checkpoint actor rows."""
        literals = sum(isinstance(d[3], str) for d in DECLARATIONS)
        indices = len(DECLARATIONS) - literals
        summary = (f"Reward opaque: 0 empty variants\nCheckpoint reward opaque: 0 empty variants\n"
                   f"Target locations: 0 array children\nCheckpoint targets: 0 array children\n"
                   "  Frame skips:      0 external blobs / 0 external bytes / 0 game-specific bytes\n"
                   "  Checkpoint frame skips: 0 external blobs / 0 external bytes"
                   " / 0 game-specific bytes\n"
                   "  Frame times:      0 non-finite\n  Checkpoint frame times: 0 non-finite\n"
                   "  Movement components: 0 attempted / 0 complete / 0 errors / 0 empty\n"
                   "  Checkpoint movement components: 0 attempted / 0 complete / 0 errors / 0 empty\n"
                   "  Component envelopes: 0 validated / 0 payload bytes / 0 unused bits\n"
                   "  Checkpoint component envelopes: 0 validated / 0 payload bytes / 0 unused bits\n"
                   "  Component headers: 0 input / 0 discrepancy\n"
                   "  Checkpoint component headers: 0 input / 0 discrepancy\n"
                   "  ActiveBlinds trailers: 0 empty deltas\n"
                   "  Checkpoint ActiveBlinds trailers: 0 empty deltas\n"
                   "  Trailing bytes:   {trailing}\n"
                   f"GUID entries: {len(DECLARATIONS)}\n"
                   f"GUID paths: {literals} literals / {indices} indices / {indices} resolved\n")
        sink = {"tracked_rewards_opaque_empty_variants": 0, "targeting_world_locations_decoded": 0,
                "movement_rpc_errors": 0, **dict.fromkeys(guard.SINK_TALLY_KEYS, 0)}
        frames = {f"frame_{key}": 0 for key in guard.FRAME_SKIP_KEYS}
        manifest = {"quality": {"sink": sink, **frames, "checkpoints": dict(
            sink=sink, checkpoint_actor_rows_dropped=0, checkpoint_trailing_bytes=0,
            **{f"checkpoint_{key}": value for key, value in frames.items()},
            checkpoint_path_resolution_mode="preceding_literal_zero_based",
            checkpoint_literal_paths=literals, checkpoint_indexed_paths=indices,
            checkpoint_resolved_path_indices=indices, checkpoint_guid_entries=len(DECLARATIONS))}}
        edited = dict(MAIN_GUIDS)
        edited[12] = ("/Game/F.F_C", 11)
        for main, trailing, dropped, refusal in (
                (MAIN_GUIDS, 0, 0, None),
                (edited, 0, 0, "path the main stream does not declare"),
                (MAIN_GUIDS, 1, 0, "cp_trailing_bytes=0 disagrees with summary 1"),
                (MAIN_GUIDS, 0, 2, "checkpoint_actor_rows_dropped=2, expected 0")):
            with self.subTest(refusal=refusal), tempfile.TemporaryDirectory() as temp:
                out = Path(temp)
                for name in (*guard.PARQUET_FILES, *guard.CHECKPOINT_PARQUET_FILES):
                    pq.write_table(pa.table({"value": [1]}), out / f"{name}.parquet")
                write_guid_tables(out, encode_guid_entries(), main)
                manifest["quality"]["checkpoints"]["checkpoint_actor_rows_dropped"] = dropped
                (out / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
                printed = io.StringIO()
                with patch.object(guard.sc.subprocess, "run", return_value=SimpleNamespace(
                        returncode=0, stdout=summary.replace("{trailing}", str(trailing)),
                        stderr="")), contextlib.redirect_stdout(printed):
                    if refusal is None:
                        guard.measure(Path("fake.exe"), out / "sample.vrf", out, checkpoints=True)
                    else:
                        with self.assertRaisesRegex(SystemExit, refusal):
                            guard.measure(Path("fake.exe"), out / "sample.vrf", out, checkpoints=True)
                self.assertIn("Checkpoint GUID cross-check: indexed joined 5,", printed.getvalue())


class ContentIdentityTests(unittest.TestCase):
    def test_equal_size_different_bytes_do_not_satisfy_byte_identity(self):
        baseline = measurement()
        current = measurement()
        current["parquet"]["fields"]["sha256"] = "b" * 64

        problems = guard.diff(baseline, current)

        self.assertTrue(any("fields.parquet sha256" in p for p in problems), problems)


#: A manifest `quality` block carrying every MANIFEST_CHECKS count, distinct
#: values in each pass, and the summary counters that agree with it.
MAIN_SINK = {"tracked_rewards_opaque_empty_variants": 4470,
             "targeting_world_locations_decoded": 12, "active_blinds_empty_trailers": 13,
             "movement_component_streams": 11, "movement_component_complete": 11,
             "movement_component_errors": 0, "movement_component_empty": 1,
             "movement_component_envelopes": 11, "movement_component_payload_bytes": 300,
             "movement_component_unused_bits": 40, "movement_component_input_headers": 10,
             "movement_component_discrepancy_headers": 9}
CP_SINK = {"tracked_rewards_opaque_empty_variants": 7, "targeting_world_locations_decoded": 0,
           "active_blinds_empty_trailers": 23,
           "movement_component_streams": 21, "movement_component_complete": 21,
           "movement_component_errors": 0, "movement_component_empty": 2,
           "movement_component_envelopes": 21, "movement_component_payload_bytes": 570,
           "movement_component_unused_bits": 70, "movement_component_input_headers": 19,
           "movement_component_discrepancy_headers": 17}
MAIN_FRAMES = {"external_data_blobs": 2, "external_data_bytes": 9, "game_specific_bytes": 0,
               "non_finite_times": 6}
CP_FRAMES = {"external_data_blobs": 1, "external_data_bytes": 4, "game_specific_bytes": 5,
             "non_finite_times": 8}


def manifest_quality():
    return {"sink": dict(MAIN_SINK, movement_rpc_errors=0), **{f"frame_{k}": v for k, v in MAIN_FRAMES.items()},
            "checkpoints": {"sink": dict(CP_SINK, movement_rpc_errors=0), "checkpoint_trailing_bytes": 14,
                            **{f"checkpoint_frame_{k}": v for k, v in CP_FRAMES.items()}}}


AGREEING_COUNTERS = {**MAIN_SINK, **{"cp_" + k: v for k, v in CP_SINK.items()},
                     **{f"frame_{k}": v for k, v in MAIN_FRAMES.items()},
                     **{f"cp_frame_{k}": v for k, v in CP_FRAMES.items()},
                     "cp_trailing_bytes": 14}


class ManifestCheckTests(TempDirTestCase):
    """Each MANIFEST_CHECKS count must equal the summary's in both passes,
    zeros included, and be a count the manifest actually holds."""

    def test_native_envelopes_are_read_independently_in_both_passes(self):
        text = ("Movement err: Component envelopes: 9 validated / 99 payload bytes / 4 unused bits\n"
                "Component envelopes: 2 validated / 80 payload bytes / 5 unused bits\n"
                "Checkpoint component envelopes: 3 validated / 190 payload bytes / 7 unused bits\n")
        wanted = ("movement_component_envelopes", "movement_component_payload_bytes",
                  "movement_component_unused_bits")
        counts = guard.sc.read(text, wanted + tuple("cp_" + key for key in wanted))
        self.assertEqual(counts, dict(zip(wanted + tuple("cp_" + key for key in wanted),
                                          (2, 80, 5, 3, 190, 7))))

    def test_native_envelopes_preserve_missing_and_zero_counts(self):
        wanted = ("movement_component_envelopes", "movement_component_payload_bytes",
                  "movement_component_unused_bits")
        for text, expected in (("", None),
                               ("Component envelopes: 0 validated / 0 payload bytes / 0 unused bits", 0)):
            self.assertEqual(guard.sc.read(text, wanted), dict.fromkeys(wanted, expected))

    def errors(self, quality, counters, checkpoints=True):
        root = self.tmp()
        (root / "manifest.json").write_text(json.dumps({"quality": quality}),
                                            encoding="utf-8")
        return " ".join(guard.manifest_errors(root, counters, checkpoints))

    def test_every_count_must_match_the_summary_in_its_pass(self):
        self.assertEqual(self.errors(manifest_quality(), AGREEING_COUNTERS), "")
        for key, value in AGREEING_COUNTERS.items():
            with self.subTest(key=key):
                self.assertIn(f"manifest {key}={value} disagrees with summary {value + 1}",
                              self.errors(manifest_quality(),
                                          dict(AGREEING_COUNTERS, **{key: value + 1})))
                unprinted = {k: v for k, v in AGREEING_COUNTERS.items() if k != key}
                self.assertIn(f"manifest {key}={value} disagrees with summary None",
                              self.errors(manifest_quality(), unprinted))
        main_only = {k: v for k, v in AGREEING_COUNTERS.items() if not k.startswith("cp_")}
        quality = manifest_quality()
        del quality["checkpoints"]
        self.assertEqual(self.errors(quality, main_only, checkpoints=False), "")

    def test_a_missing_or_non_count_value_is_named_not_read(self):
        for what, path in (
                ("tracked rewards opaque-empty", ("sink", "tracked_rewards_opaque_empty_variants")),
                ("targeting world-location",
                 ("checkpoints", "sink", "targeting_world_locations_decoded")),
                ("sink component/opaque tally",
                 ("checkpoints", "sink", "movement_component_unused_bits")),
                ("frame-skip", ("frame_external_data_bytes",)),
                ("checkpoint trailing-bytes", ("checkpoints", "checkpoint_trailing_bytes"))):
            for value in (None, True, -1, "0", 0.0, "absent"):
                with self.subTest(what=what, value=value):
                    quality = manifest_quality()
                    parent = quality
                    for part in path[:-1]:
                        parent = parent[part]
                    if value == "absent":
                        del parent[path[-1]]
                    else:
                        parent[path[-1]] = value
                    self.assertIn(f"manifest omits {what} quality data" if value == "absent"
                                  else f"{what} counts must be nonnegative integers",
                                  self.errors(quality, AGREEING_COUNTERS))
        # The sink counts live below `quality.sink`; a flat shape is a wiring drift.
        flat = manifest_quality()
        flat["tracked_rewards_opaque_empty_variants"] = flat["sink"].pop(
            "tracked_rewards_opaque_empty_variants")
        self.assertIn("manifest omits tracked rewards opaque-empty",
                      self.errors(flat, AGREEING_COUNTERS))


class MainTests(TempDirTestCase):
    """`main`'s input handling: a machine path is never pinned, a bare name
    resolves against VRFKIT_CORPUS_DIR, and a missing replay is fatal only
    when required."""

    def run_main(self, root: Path, *extra: str, corpus_dir: str | None = None,
                 require: str | None = None):
        current = measurement(actor_closes=0)
        self.assertEqual(guard.cross_checks(current["counters"], current["parquet"]), [],
                         "the stand-in measurement must reach the update")
        with patch.dict(os.environ), patch.object(guard, "measure", return_value=current) as measured:
            os.environ.pop("VRFKIT_CORPUS_DIR", None)
            os.environ.pop("VRFKIT_REQUIRE_CORPUS", None)
            if corpus_dir is not None:
                os.environ["VRFKIT_CORPUS_DIR"] = corpus_dir
            if require is not None:
                os.environ["VRFKIT_REQUIRE_CORPUS"] = require
            code, output, _ = run_cli(guard.main, "--baseline", root / "baseline.json", "--exe",
                                      sys.executable, *extra, prog="check_export_baseline.py",
                                      merged=True)
        return code, output, measured

    def test_an_absolute_replay_is_refused_before_the_export_runs(self):
        root = self.tmp()
        (root / "match.vrf").write_bytes(b"replay")
        code, output, measured = self.run_main(
            root, "--replay", str(root / "match.vrf"), "--update")
        self.assertEqual(code, 2, output)
        self.assertIn("pass --replay match.vrf", output)
        measured.assert_not_called()
        self.assertFalse((root / "baseline.json").exists())

    def test_a_bare_replay_is_pinned_as_given_not_as_resolved(self):
        root = self.tmp()
        (root / "match.vrf").write_bytes(b"replay")
        code, output, measured = self.run_main(root, "--replay", "match.vrf", "--update",
                                               corpus_dir=str(root))
        self.assertEqual(code, 0, output)
        self.assertEqual(measured.call_args.args[1], root / "match.vrf")
        stored = json.loads((root / "baseline.json").read_text(encoding="utf-8"))
        self.assertEqual(stored["replay"], "match.vrf")

    def test_a_missing_replay_skips_unless_required(self):
        root = self.tmp()
        (root / "baseline.json").write_text(json.dumps({"replay": "missing.vrf"}),
                                            encoding="utf-8")
        for extra, require, code, marker in (
                ((), None, 0, "SKIP:"), (("--require-input",), None, 2, "REQUIRED INPUT"),
                ((), "1", 2, "REQUIRED INPUT MISSING")):
            with self.subTest(extra=extra, require=require):
                got, output, measured = self.run_main(root, *extra, require=require)
                self.assertEqual(got, code, output)
                self.assertIn(marker, output)
                measured.assert_not_called()

    def test_obsolete_baseline_fails_before_missing_input_can_skip(self):
        for name in ("export_02d4d478.json", "checkpoint_02d4d478.json"):
            with self.subTest(name=name):
                root = self.tmp()
                original = (guard.REPO / "tools/baselines" / name).read_bytes()
                baseline = root / "baseline.json"
                baseline.write_bytes(original)
                code, output, measured = self.run_main(root)
                self.assertEqual(code, 1, output)
                self.assertIn("obsolete movement counter contract", output)
                self.assertIn("genuine remeasurement", output)
                self.assertNotIn("SKIP:", output)
                measured.assert_not_called()
                self.assertEqual(baseline.read_bytes(), original)
                code, output, measured = self.run_main(root, "--update")
                self.assertEqual(code, 2, output)
                self.assertIn("REQUIRED INPUT", output)
                measured.assert_not_called()
                self.assertEqual(baseline.read_bytes(), original)


class TransactionalOutputTests(TempDirTestCase):
    SUMMARY = """
Total content blocks: 1
Fields emitted: 1
RPCs emitted: 1
Movement rows: 1
Event rows: 1
NetGUID rows: 1
Actor opens: 1
Actor closes: 0
Reward opaque: 0 empty variants
Target locations: 0 array children
Frame skips: 0 external blobs / 0 external bytes / 0 game-specific bytes
Frame times: 0 non-finite
Movement components: 0 attempted / 0 complete / 0 errors / 0 empty
Component envelopes: 0 validated / 0 payload bytes / 0 unused bits
Component headers: 0 input / 0 discrepancy
ActiveBlinds trailers: 0 empty deltas
"""

    def run_fake_export(self, *, fail: bool):
        root = self.tmp()
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
                "import json, os, shutil, sys\n"
                "from pathlib import Path\n"
                "import pyarrow as pa\n"
                "import pyarrow.parquet as pq\n"
                "out = Path(sys.argv[sys.argv.index('--out') + 1])\n"
                "stage = out.parent / (out.name + '.stage')\n"
                "backup = out.parent / (out.name + '.backup')\n"
                "stage.mkdir()\n"
                "for name in ('actors', 'fields', 'movement', 'net_guids', 'events', 'partials'):\n"
                "    pq.write_table(pa.table({'value': [1]}), stage / (name + '.parquet'))\n"
                "(stage / 'manifest.json').write_text(json.dumps({'quality': {'sink': {'tracked_rewards_opaque_empty_variants': 0, 'targeting_world_locations_decoded': 0, 'movement_rpc_errors': 0, **dict.fromkeys(('movement_component_streams', 'movement_component_complete', 'movement_component_errors', 'movement_component_empty', 'movement_component_envelopes', 'movement_component_payload_bytes', 'movement_component_unused_bits', 'movement_component_input_headers', 'movement_component_discrepancy_headers'), 0), 'active_blinds_empty_trailers': 0}, 'frame_external_data_blobs': 0, 'frame_external_data_bytes': 0, 'frame_game_specific_bytes': 0, 'frame_non_finite_times': 0}}), encoding='utf-8')\n"
                "os.replace(out, backup)\n"
                "os.replace(stage, out)\n"
                "shutil.rmtree(backup)\n"
                f"print({self.SUMMARY!r})\n",
                encoding="utf-8",
            )

        with contextlib.chdir(root):
            if fail:
                with self.assertRaises(SystemExit) as caught:
                    guard.measure(Path(sys.executable), replay, output)
                self.assertIn("exit 7", str(caught.exception))
                self.assertIn("deliberate export failure", str(caught.exception))
                result = None
            else:
                result = guard.measure(Path(sys.executable), replay, output)
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

    def test_fresh_telemetry_does_not_invent_historical_baseline_measurements(self):
        _, _, result = self.run_fake_export(fail=False)
        historical = json.loads(
            (guard.REPO / "tools/baselines/export_02d4d478.json").read_text(encoding="utf-8"))

        self.assertEqual(set(result["counters"]), set(guard.COUNTERS))
        self.assertEqual(set(historical["counters"]), set(guard.LEGACY_COUNTERS))
        self.assertNotEqual(set(result["counters"]), set(historical["counters"]))
        self.assertEqual(guard.diff(historical, result), [guard.sc.OBSOLETE_MOVEMENT_CONTRACT])
