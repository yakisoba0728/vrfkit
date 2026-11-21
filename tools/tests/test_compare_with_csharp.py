"""Guards for the C# comparison report: a report, not a gate, but one that
must not read an empty C# side as total coverage, and must exit nonzero when
it measured nothing.
"""
import json
import unittest
from unittest.mock import patch

import pyarrow as pa
import pyarrow.parquet as pq

from support import TempDirTestCase, retained_parquet_readers, run_cli
import compare_with_csharp as guard


PAIR_A = ("/Script/ShooterGame.Thing", "Health")
PAIR_B = ("/Script/ShooterGame.Thing", "Armor")


class TotalCountTests(unittest.TestCase):
    def bunch_row(self, csharp_stats, vrfkit_count):
        report = guard.compare_totals({"stats": csharp_stats},
                                      {"stats": {"bunch_count": vrfkit_count}})
        return next(line for line in report.splitlines() if line.startswith("Bunches"))

    def test_bunches_use_bunch_count_not_packets_containing_bunches(self):
        # Both names are emitted independently by the reference schema v9.
        row = self.bunch_row({"packets_with_bunches": 8, "bunch_count": 21}, 21)
        self.assertRegex(row, r"Bunches\s+21\s+21\s+\u2713")

    def test_packets_with_bunches_cannot_substitute_for_missing_bunch_count(self):
        row = self.bunch_row({"packets_with_bunches": 8}, 8)
        self.assertRegex(row, r"Bunches\s+\?\s+8\s+\?")
        self.assertNotIn("\u2713", row)

    def test_zero_bunch_count_is_not_treated_as_missing(self):
        row = self.bunch_row({"packets_with_bunches": 0, "bunch_count": 0}, 0)
        self.assertRegex(row, r"Bunches\s+0\s+0\s+\u2713")


class CoverageProblemTests(unittest.TestCase):
    def test_an_overlapping_comparison_has_no_problems(self):
        self.assertEqual(guard.coverage_problems({PAIR_A}, {PAIR_A, PAIR_B}), [])

    def test_an_empty_csharp_side_is_not_full_coverage(self):
        problems = guard.coverage_problems(set(), {PAIR_A})
        self.assertTrue(problems)
        self.assertIn("C#", " ".join(problems))

    def test_an_empty_vrfkit_side_is_not_a_comparison_either(self):
        self.assertTrue(guard.coverage_problems({PAIR_A}, set()))

    def test_two_sides_sharing_nothing_is_total_disagreement(self):
        problems = guard.coverage_problems({PAIR_A}, {PAIR_B})
        self.assertTrue(problems)
        self.assertIn("no", " ".join(problems).lower())

    def test_missing_pairs_alone_are_not_reported_here(self):
        """C#-only pairs are the report's subject, listed under INVESTIGATE,
        not a gate: an acceptable miss cannot be decided without the corpus."""
        self.assertEqual(guard.coverage_problems({PAIR_A, PAIR_B}, {PAIR_A}), [])


class CoverageTextTests(TempDirTestCase):
    def test_unattributed_rows_are_counted_without_becoming_named_coverage(self):
        root = self.tmp()
        events = root / "events.ndjson"
        events.write_text("\n".join(json.dumps(row) for row in [
            {"type": "export_group_received", "export_group_path": PAIR_A[0],
             "payload": {"Health": 100}},
            {"type": "export_group_received", "export_group_path": None,
             "payload": {"Health": 100}},
        ]), encoding="utf-8")
        parquet = root / "fields.parquet"
        pq.write_table(pa.table({
            "group_path": [PAIR_A[0], PAIR_A[0], None, PAIR_A[0]],
            "field_name": ["Health", None, "Health",
                           guard.UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME],
        }), parquet)
        report, problems = guard.compare_group_field_coverage(events, parquet)
        self.assertEqual(problems, [])
        self.assertIn("Distinct (group, field) pairs from vrfkit: 1", report)
        self.assertIn("vrfkit rows without a group/name: 2 (excluded)", report)
        self.assertIn("C# payload fields without a group/name: 1 (excluded)", report)

    def test_full_coverage_is_only_claimed_when_something_was_compared(self):
        lines = guard.coverage_lines({PAIR_A}, {PAIR_A, PAIR_B})
        self.assertIn("covers everything", " ".join(lines))

    def test_nothing_compared_never_claims_full_coverage(self):
        lines = guard.coverage_lines(set(), {PAIR_A})
        self.assertNotIn("covers everything", " ".join(lines))

    def test_a_real_miss_is_still_listed_for_investigation(self):
        lines = guard.coverage_lines({PAIR_A, PAIR_B}, {PAIR_A})
        joined = " ".join(lines)
        self.assertIn("INVESTIGATE", joined)
        self.assertIn("Armor", joined)


class InputLifetimeTests(TempDirTestCase):
    def inputs(self):
        root = self.tmp()
        events, parquet = root / "events.ndjson", root / "fields.parquet"
        events.write_text(json.dumps({
            "type": "export_group_received", "export_group_path": PAIR_A[0],
            "payload": {PAIR_A[1]: 100},
        }) + "\n", encoding="utf-8")
        pq.write_table(pa.table({
            "group_path": [PAIR_A[0]], "field_name": [PAIR_A[1]],
        }), parquet)
        return events, parquet

    def test_field_inputs_are_closed_before_comparisons_return(self):
        events, parquet = self.inputs()
        real_reader = pq.ParquetFile
        for compare in (
            lambda: guard.compare_group_paths({}, {}, parquet),
            lambda: guard.compare_group_field_coverage(events, parquet),
            lambda: guard.vrfkit_rpc_names(parquet),
        ):
            with self.subTest(compare=compare):
                readers = []

                def retain_reader(*args, **kwargs):
                    reader = real_reader(*args, **kwargs)
                    readers.append(reader)
                    self.addCleanup(reader.close)
                    return reader

                with patch.object(guard.pq, "ParquetFile", side_effect=retain_reader):
                    compare()

                self.assertTrue(readers)
                self.assertTrue(all(reader.closed for reader in readers))

    def test_field_input_is_closed_when_reading_raises(self):
        events, parquet = self.inputs()
        real_reader = pq.ParquetFile
        readers = []
        failure = OSError("input read failed")

        def failing_reader(*args, **kwargs):
            reader = real_reader(*args, **kwargs)
            readers.append(reader)
            self.addCleanup(reader.close)
            read = patch.object(reader, "read", side_effect=failure)
            read.start()
            self.addCleanup(read.stop)
            return reader

        with patch.object(guard.pq, "ParquetFile", side_effect=failing_reader):
            with self.assertRaises(OSError) as raised:
                guard.compare_group_field_coverage(events, parquet)

        self.assertIs(raised.exception, failure)
        self.assertTrue(readers)
        self.assertTrue(all(reader.closed for reader in readers))


class RpcNameTests(TempDirTestCase):
    """vrfkit writes no RPC-name column or manifest key: its RPC names are the
    `Function.` prefixes of its ClassNetCache rows, the rows
    to_valplay_bundle.py builds rpc_received from."""

    CNC = "/Script/ShooterGame.Thing_ClassNetCache"

    def section_4(self) -> str:
        root = self.tmp()
        cs, vk = root / "cs", root / "vk"
        cs.mkdir()
        vk.mkdir()
        (cs / "manifest.json").write_text("{}", encoding="utf-8")
        (vk / "manifest.json").write_text("{}", encoding="utf-8")
        (cs / "events.ndjson").write_text("".join(json.dumps(row) + "\n" for row in [
            {"type": "export_group_received", "export_group_path": PAIR_A[0],
             "payload": {"Health": 100}},
            {"type": "rpc_received", "function_name": "MulticastShared"},
            {"type": "rpc_received", "function_name": "MulticastShared"},
            {"type": "rpc_received", "function_name": "ClientCsharpOnly"},
        ]), encoding="utf-8")
        rows = [
            (PAIR_A[0], "Health"),
            (self.CNC, "MulticastShared.Damage"),
            (self.CNC, "MulticastShared.Target"),
            (self.CNC, "ZeroParamOnly"),        # a zero-parameter RPC is its bare name
            (self.CNC, guard.UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME),
            (self.CNC, None),
            (PAIR_A[0], "Rounds[3].Score"),     # an array leaf, not an RPC
        ]
        pq.write_table(pa.table({"group_path": [g for g, _ in rows],
                                 "field_name": [f for _, f in rows]}),
                       vk / "fields.parquet")
        _, report, _ = run_cli(guard.main, cs, vk, prog="compare_with_csharp.py")
        return report.split("## 4. RPC name comparison", 1)[1].split("## 5.", 1)[0]

    def test_vrfkit_rpc_names_come_from_class_net_cache_prefixes(self):
        section = self.section_4()
        self.assertIn("vrfkit RPC distinct names: 2", section)
        self.assertIn("C# only: 1", section)
        self.assertIn("vrfkit only: 1", section)
        self.assertIn("Both: 1", section)
        self.assertRegex(section, r"C# only, INVESTIGATE \(1\):\n\s+ClientCsharpOnly")
        self.assertRegex(section, r"vrfkit only, sample \(1\):\n\s+ZeroParamOnly")
        self.assertNotIn("Rounds[3]", section)
        self.assertNotIn("rpcs_by_name", section)


class MovementMultiplicityTests(TempDirTestCase):
    def compare(self, csharp_rows: list[dict], vrfkit_rows: list[dict]) -> str:
        root = self.tmp()
        csharp = root / "movement.ndjson"
        parquet = root / "movement.parquet"
        csharp.write_text(
            "".join(json.dumps(row) + "\n" for row in csharp_rows),
            encoding="utf-8",
        )
        table = pa.table(
            {
                "time_ms": pa.array([r["time_ms"] for r in vrfkit_rows], pa.uint32()),
                "character_net_guid": pa.array(
                    [r["character_net_guid"] for r in vrfkit_rows], pa.uint32()
                ),
                "pos_x": pa.array([r.get("pos_x", 0.0) for r in vrfkit_rows]),
                "pos_y": pa.array([r.get("pos_y", 0.0) for r in vrfkit_rows]),
                "pos_z": pa.array([r.get("pos_z", 0.0) for r in vrfkit_rows]),
                "yaw": pa.array([r.get("yaw", 0.0) for r in vrfkit_rows]),
                "pitch": pa.array([r.get("pitch", 0.0) for r in vrfkit_rows]),
                "vel_x": pa.array([r.get("vel_x", 0.0) for r in vrfkit_rows]),
                "vel_y": pa.array([r.get("vel_y", 0.0) for r in vrfkit_rows]),
                "vel_z": pa.array([r.get("vel_z", 0.0) for r in vrfkit_rows]),
            }
        )
        for name, kind in (("packet_id", pa.uint64()), ("timestamp", pa.float64())):
            if any(name in row for row in vrfkit_rows):
                table = table.append_column(name, pa.array([row.get(name) for row in vrfkit_rows], type=kind))
        pq.write_table(table, parquet)
        inputs, real_read = [], pq.read_table

        def retain_input(source, *args, **kwargs):
            inputs.append(source)
            return real_read(source, *args, **kwargs)

        with retained_parquet_readers() as readers:
            with patch.object(guard.pq, "read_table", side_effect=retain_input):
                report = guard.compare_movement(csharp, parquet)
            self.assertTrue(readers)
            self.assertTrue(all(reader.closed for reader in readers))
            self.assertTrue(inputs)
            self.assertTrue(all(source.closed for source in inputs))
        return report

    @staticmethod
    def csharp_row(x: float) -> dict:
        return {
            "time_ms": 100,
            "shooter_character_net_guid": 42,
            "position": {"x": x, "y": 0, "z": 0},
            "velocity": {"x": 0, "y": 0, "z": 0},
            "yaw": 0,
            "pitch": 0,
        }

    def test_one_vrfkit_row_cannot_satisfy_two_duplicate_references(self):
        report = self.compare(
            [self.csharp_row(1), self.csharp_row(2)],
            [{"time_ms": 100, "character_net_guid": 42, "pos_x": 1}],
        )
        self.assertIn("Joined: 1 / 2", report)
        self.assertRegex(report, r"Missed .*: 1")

    def test_legacy_duplicate_rows_are_reported_as_ambiguous(self):
        report = self.compare(
            [self.csharp_row(1), self.csharp_row(2)],
            [
                {"time_ms": 100, "character_net_guid": 42, "pos_x": 1},
                {"time_ms": 100, "character_net_guid": 42, "pos_x": 2},
            ],
        )
        self.assertIn("Joined: 0 / 2", report)
        self.assertIn("Ambiguous (multiple candidate rows): 2", report)

    def test_a_value_the_reference_lacks_is_not_compared_as_zero(self):
        row = self.csharp_row(1)
        del row["yaw"]
        report = self.compare([row], [{"time_ms": 100, "character_net_guid": 42,
                                       "pos_x": 1, "yaw": 170.0}])
        self.assertIn("Yaw: no data", report)
        self.assertIn("Pitch (1 rows)", report)

    def test_earliest_reference_row_can_join_one_millisecond_earlier(self):
        # Synthetic boundary case: not a claim about an observed replay error.
        report = self.compare([self.csharp_row(1)], [
            {"time_ms": 99, "character_net_guid": 42, "pos_x": 1}])
        self.assertIn("Joined: 1 / 1", report)
        self.assertIn("Position (max axis) (1 rows): max=0.0000", report)

    def test_zero_timestamp_remains_valid_with_unsigned_parquet_time(self):
        row = self.csharp_row(1)
        row["time_ms"] = 0
        report = self.compare([row], [{"time_ms": 0, "character_net_guid": 42, "pos_x": 1}])
        self.assertIn("Joined: 1 / 1", report)

    def test_earliest_row_outside_one_millisecond_window_remains_unmatched(self):
        report = self.compare([self.csharp_row(1)], [
            {"time_ms": 98, "character_net_guid": 42, "pos_x": 1}])
        self.assertIn("Joined: 0 / 1", report)

    def test_packet_identity_disambiguates_rows_with_the_same_display_time(self):
        row = {**self.csharp_row(2), "packet_id": 2, "timestamp": 10.0}
        report = self.compare([row], [
            {"time_ms": 100, "character_net_guid": 42, "pos_x": 1, "packet_id": 1, "timestamp": 10.0},
            {"time_ms": 100, "character_net_guid": 42, "pos_x": 2, "packet_id": 2, "timestamp": 10.0}])
        self.assertIn("Joined: 1 / 1", report)
        self.assertIn("Position (max axis) (1 rows): max=0.0000", report)

    def test_exact_identity_is_not_filtered_by_display_time(self):
        # Display time may disagree even when the packet and move identity agree.
        for rust_time, packet_id in ((0, 7), (10_000, 7), (10_000, 2**64 - 1)):
            with self.subTest(rust_time=rust_time, packet_id=packet_id):
                row = {**self.csharp_row(2), "packet_id": packet_id, "timestamp": 10.0}
                report = self.compare([row], [
                    {"time_ms": rust_time, "character_net_guid": 42, "pos_x": 9,
                     "packet_id": packet_id, "timestamp": 10.0},
                    {"time_ms": 100, "character_net_guid": 42, "pos_x": 2,
                     "packet_id": 6, "timestamp": 10.0}])
                self.assertIn("Joined: 1 / 1", report)
                self.assertIn("Unmatched exact identities: 0", report)
                # The identical position on a different packet must not win.
                self.assertIn("Position (max axis) (1 rows): max=7.0000", report)

    def test_movement_timestamp_selects_the_sample_within_a_packet(self):
        row = {**self.csharp_row(2), "packet_id": 1, "timestamp": 11.0}
        report = self.compare([row], [
            {"time_ms": 100, "character_net_guid": 42, "pos_x": 1, "packet_id": 1, "timestamp": 10.0},
            {"time_ms": 100, "character_net_guid": 42, "pos_x": 2, "packet_id": 1, "timestamp": 11.0}])
        self.assertIn("Joined: 1 / 1", report)
        self.assertIn("Position (max axis) (1 rows): max=0.0000", report)

    def test_present_but_unmatched_identity_never_falls_back_to_display_time(self):
        row = {**self.csharp_row(1), "packet_id": 2, "timestamp": 10.0}
        report = self.compare([row], [
            {"time_ms": 100, "character_net_guid": 42, "pos_x": 1, "packet_id": 1, "timestamp": 10.0}])
        self.assertIn("Joined: 0 / 1", report)
        self.assertIn("Unmatched exact identities: 1", report)

    def test_ambiguous_exact_identity_is_not_selected_by_matching_value(self):
        row = {**self.csharp_row(2), "packet_id": 1, "timestamp": 10.0}
        report = self.compare([row], [
            {"time_ms": 100, "character_net_guid": 42, "pos_x": 2, "packet_id": 1, "timestamp": 10.0},
            {"time_ms": 100, "character_net_guid": 42, "pos_x": 7, "packet_id": 1, "timestamp": 10.0}])
        self.assertIn("Joined: 0 / 1", report)
        self.assertIn("Ambiguous (multiple candidate rows): 1", report)

    def test_ambiguous_identity_candidates_cannot_be_reused_by_legacy_rows(self):
        strong = {**self.csharp_row(2), "packet_id": 1, "timestamp": 10.0}
        legacy = self.csharp_row(9)
        report = self.compare([legacy, strong], [
            {"time_ms": 100, "character_net_guid": 42, "pos_x": 2,
             "packet_id": 1, "timestamp": 10.0},
            {"time_ms": 101, "character_net_guid": 42, "pos_x": 7,
             "packet_id": 1, "timestamp": 10.0}])
        self.assertIn("Joined: 0 / 2", report)
        self.assertIn("Ambiguous (multiple candidate rows): 1", report)
        self.assertRegex(report, r"Missed .*: 1")
        self.assertIn("Position (max axis): no data", report)

    def test_ambiguous_identity_reservation_keeps_unrelated_legacy_candidates(self):
        strong = {**self.csharp_row(2), "packet_id": 1, "timestamp": 10.0}
        legacy = self.csharp_row(9)
        report = self.compare([legacy, strong, strong], [
            {"time_ms": 100, "character_net_guid": 42, "pos_x": 2,
             "packet_id": 1, "timestamp": 10.0},
            {"time_ms": 101, "character_net_guid": 42, "pos_x": 7,
             "packet_id": 1, "timestamp": 10.0},
            {"time_ms": 100, "character_net_guid": 42, "pos_x": 9,
             "packet_id": 2, "timestamp": 11.0}])
        self.assertIn("Joined: 1 / 3", report)
        self.assertIn("Ambiguous (multiple candidate rows): 2", report)
        self.assertIn("Unmatched exact identities: 0", report)
        self.assertIn("Position (max axis) (1 rows): max=0.0000", report)

    def test_missing_identity_metadata_uses_legacy_fallback(self):
        row = {**self.csharp_row(1), "packet_id": 2}  # no movement timestamp
        report = self.compare([row], [
            {"time_ms": 99, "character_net_guid": 42, "pos_x": 1, "packet_id": 1, "timestamp": 10.0}])
        self.assertIn("Joined: 1 / 1", report)
        self.assertIn("Legacy rows (identity metadata absent): 1", report)

    def test_both_neighboring_legacy_candidates_are_ambiguous(self):
        report = self.compare([self.csharp_row(1)], [
            {"time_ms": 99, "character_net_guid": 42, "pos_x": 1},
            {"time_ms": 101, "character_net_guid": 42, "pos_x": 2}])
        self.assertIn("Joined: 0 / 1", report)
        self.assertIn("Ambiguous (multiple candidate rows): 1", report)

    def test_exact_identity_rows_are_reserved_before_legacy_rows(self):
        legacy = self.csharp_row(2)
        strong = {**self.csharp_row(1), "packet_id": 1, "timestamp": 10.0}
        report = self.compare([legacy, strong], [
            {"time_ms": 100, "character_net_guid": 42, "pos_x": 1, "packet_id": 1, "timestamp": 10.0},
            {"time_ms": 100, "character_net_guid": 42, "pos_x": 2, "packet_id": 2, "timestamp": 10.0}])
        self.assertIn("Joined: 2 / 2", report)
        self.assertIn("Position (max axis) (2 rows): max=0.0000", report)
