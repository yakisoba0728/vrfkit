"""Tests for exact pairing and conservative ability-stat dictionary checks."""

from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import extract_ability_stats as stats  # noqa: E402


def member_row(
    member: str,
    value,
    *,
    packet: int = 10,
    time_ms: int = 100,
    channel: int = 3,
    actor: int = 20,
    object_guid: int | None = 30,
    cast: int = 1,
    effect: int = 2,
) -> dict:
    return {
        "time_ms": time_ms,
        "packet_id": packet,
        "channel_index": channel,
        "actor_net_guid": actor,
        "object_net_guid": object_guid,
        "field_name": (
            f"AbilityCastsThisRound[{cast}].Effects[{effect}].{member}"
        ),
        "value_i64": value if member == "Statistic" else None,
        "value_str": value if member == "LocalizedStat" else None,
    }


def pair(statistic_id: int, name: str, **context) -> list[dict]:
    return [
        member_row("Statistic", statistic_id, **context),
        member_row("LocalizedStat", name, **context),
    ]


class PairingTests(unittest.TestCase):
    def test_full_instance_context_prevents_cross_pairing(self):
        rows = [
            member_row("Statistic", 0, object_guid=30),
            member_row("LocalizedStat", "EnemiesBlinded", object_guid=31),
        ]
        result = stats.pair_rows(rows, "fields")
        self.assertEqual(result.pairs, {})
        self.assertEqual(result.issues["missing_localized_stat"], 1)
        self.assertEqual(result.issues["missing_statistic"], 1)

    def test_repeated_snapshots_are_counted_as_observations(self):
        rows = pair(0, "EnemiesBlinded", packet=10, time_ms=100)
        rows += pair(0, "EnemiesBlinded", packet=11, time_ms=110)
        result = stats.pair_rows(rows, "fields")
        self.assertEqual(
            result.pairs[(0, "EnemiesBlinded", "fields")],
            2,
        )
        self.assertEqual(result.issues, {})

    def test_duplicate_member_is_an_issue_instead_of_last_write_wins(self):
        rows = pair(1, "DamageDealt")
        rows.append(member_row("Statistic", 1))
        result = stats.pair_rows(rows, "fields")
        self.assertEqual(result.pairs, {})
        self.assertEqual(result.issues["duplicate_statistic"], 1)
        self.assertEqual(result.samples["duplicate_statistic"][0]["values"], [1, 1])

    def test_unrelated_effect_members_are_ignored(self):
        row = member_row("Value", 3.5)
        row["value_i64"] = None
        result = stats.pair_rows([row], "fields")
        self.assertEqual(result.relevant_rows, 0)
        self.assertEqual(result.pairs, {})
        self.assertEqual(result.issues, {})

    def test_member_matching_accepts_case_variants_and_rejects_near_matches(self):
        statistic = member_row("Statistic", 0)
        statistic["field_name"] = statistic["field_name"].lower()
        localized = member_row("LocalizedStat", "EnemiesBlinded")
        localized["field_name"] = localized["field_name"].upper()
        unrelated = member_row("LocalizedStats", "EnemiesBlinded")

        result = stats.pair_rows([statistic, localized, unrelated], "fields")

        self.assertEqual(result.relevant_rows, 2)
        self.assertEqual(result.pairs[(0, "EnemiesBlinded", "fields")], 1)
        self.assertEqual(result.issues, {})


class DictionaryTests(unittest.TestCase):
    def test_dictionary_is_build_scoped(self):
        self.assertEqual(
            stats.validation_status("13.05", 27, "TimeSprinting"),
            "known",
        )
        self.assertEqual(
            stats.validation_status("13.04", 27, "TimeSprinting"),
            "unknown_statistic_id",
        )

    def test_known_id_with_a_different_name_is_a_conflict(self):
        self.assertEqual(
            stats.validation_status("13.05", 0, "DamageDealt"),
            "statistic_name_conflict",
        )

    def test_measured_dictionary_has_31_or_32_one_to_one_entries(self):
        self.assertEqual(
            {build: len(mapping) for build, mapping in stats.KNOWN_STAT_NAMES.items()},
            {"13.01": 31, "13.02": 31, "13.04": 31, "13.05": 32},
        )
        for mapping in stats.KNOWN_STAT_NAMES.values():
            self.assertEqual(len(mapping), len(set(mapping.values())))


class CliTests(unittest.TestCase):
    def setUp(self):
        self._temp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._temp.name)
        self.addCleanup(self._temp.cleanup)

    def make_export(
        self,
        build: str,
        fields_rows: list[dict],
        checkpoint_rows: list[dict],
    ) -> Path:
        export = self.tmp / f"export-{build}"
        export.mkdir()
        (export / "manifest.json").write_text(
            json.dumps({"replay_build": f"++Ares-Core+release-{build}"}),
            encoding="utf-8",
        )
        schema = pa.schema(
            [
                pa.field("time_ms", pa.int64()),
                pa.field("packet_id", pa.int64()),
                pa.field("channel_index", pa.int64()),
                pa.field("actor_net_guid", pa.int64()),
                pa.field("object_net_guid", pa.int64()),
                pa.field("group_path", pa.string()),
                pa.field("field_name", pa.string()),
                pa.field("value_i64", pa.int64()),
                pa.field("value_str", pa.string()),
            ]
        )
        for table_name, rows in (
            ("fields", fields_rows),
            ("checkpoint_fields", checkpoint_rows),
        ):
            enriched = [{**row, "group_path": stats.GROUP} for row in rows]
            table = pa.Table.from_pylist(enriched, schema=schema)
            pq.write_table(table, export / f"{table_name}.parquet")
        return export

    def run_main(self, *argv: str) -> tuple[int, str, str]:
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            code = stats.main(list(argv))
        return code, stdout.getvalue(), stderr.getvalue()

    def test_cli_aggregates_main_and_checkpoint_observations(self):
        export = self.make_export(
            "13.05",
            pair(0, "EnemiesBlinded"),
            pair(0, "EnemiesBlinded", packet=20, time_ms=200),
        )
        out = self.tmp / "ability-stats.json"
        code, stdout, stderr = self.run_main(
            "--export", str(export), "--out", str(out)
        )
        self.assertEqual(code, 0, stderr)
        self.assertIn("2 snapshot observation(s)", stdout)
        document = json.loads(out.read_text(encoding="utf-8"))
        observed = document["observed_by_build"]["13.05"]["mappings"]
        self.assertEqual(
            observed,
            [
                {
                    "statistic_id": 0,
                    "localized_stat": "EnemiesBlinded",
                    "fields_observations": 1,
                    "checkpoint_observations": 1,
                    "total_observations": 2,
                    "validation": "known",
                }
            ],
        )
        self.assertIn("not ability casts", document["count_semantics"])
        self.assertEqual(
            document["exports"][0]["table_presence"],
            {"fields": True, "checkpoint_fields": True},
        )
        self.assertEqual(document["totals"]["exports_with_fields"], 1)
        self.assertEqual(document["totals"]["exports_with_checkpoint_fields"], 1)

    def test_main_only_export_is_supported_and_presence_is_explicit(self):
        export = self.make_export("13.05", pair(0, "EnemiesBlinded"), [])
        (export / "checkpoint_fields.parquet").unlink()
        out = self.tmp / "main-only.json"

        code, _stdout, stderr = self.run_main(
            "--export", str(export), "--out", str(out)
        )

        self.assertEqual(code, 0, stderr)
        document = json.loads(out.read_text(encoding="utf-8"))
        self.assertEqual(
            document["exports"][0]["table_presence"],
            {"fields": True, "checkpoint_fields": False},
        )
        self.assertEqual(document["totals"]["exports_with_fields"], 1)
        self.assertEqual(document["totals"]["exports_with_checkpoint_fields"], 0)
        mapping = document["observed_by_build"]["13.05"]["mappings"][0]
        self.assertEqual(mapping["fields_observations"], 1)
        self.assertEqual(mapping["checkpoint_observations"], 0)

    def test_missing_main_table_still_fails(self):
        export = self.make_export("13.05", pair(0, "EnemiesBlinded"), [])
        (export / "fields.parquet").unlink()
        out = self.tmp / "missing-main.json"

        code, _stdout, stderr = self.run_main(
            "--export", str(export), "--out", str(out)
        )

        self.assertEqual(code, 1)
        self.assertIn("no fields.parquet", stderr)
        self.assertFalse(out.exists())

    def test_unknown_id_is_written_and_returns_failure(self):
        export = self.make_export("13.05", pair(99, "FutureStat"), [])
        out = self.tmp / "unknown.json"
        code, _stdout, stderr = self.run_main(
            "--export", str(export), "--out", str(out)
        )
        self.assertEqual(code, 1)
        self.assertIn("unknown_statistic_id", stderr)
        document = json.loads(out.read_text(encoding="utf-8"))
        observed = document["observed_by_build"]["13.05"]["mappings"]
        self.assertEqual(observed[0]["validation"], "unknown_statistic_id")

    def test_observed_collision_is_explicit_even_for_unknown_ids(self):
        rows = pair(99, "FutureStat", packet=10, time_ms=100)
        rows += pair(99, "DifferentFutureStat", packet=11, time_ms=110)
        export = self.make_export("13.05", rows, [])
        out = self.tmp / "collision.json"
        code, _stdout, stderr = self.run_main(
            "--export", str(export), "--out", str(out)
        )
        self.assertEqual(code, 1)
        self.assertIn("observed statistic ID collision", stderr)
        conflicts = json.loads(out.read_text(encoding="utf-8"))[
            "observed_by_build"
        ]["13.05"]["mapping_conflicts"]
        self.assertEqual(
            conflicts["statistic_ids"],
            [
                {
                    "statistic_id": 99,
                    "localized_stats": ["DifferentFutureStat", "FutureStat"],
                }
            ],
        )

    def test_duplicate_export_alias_does_not_double_count(self):
        export = self.make_export("13.05", pair(0, "EnemiesBlinded"), [])
        out = self.tmp / "deduplicated.json"
        alias = export / "."
        code, _stdout, stderr = self.run_main(
            "--export",
            str(export),
            "--export",
            str(alias),
            "--out",
            str(out),
        )
        self.assertEqual(code, 0, stderr)
        totals = json.loads(out.read_text(encoding="utf-8"))["totals"]
        self.assertEqual(totals["paired_observations"], 1)
        self.assertEqual(totals["exports_requested"], 2)
        self.assertEqual(totals["duplicate_exports_ignored"], 1)

    def test_output_cannot_alias_an_input_export_file(self):
        export = self.make_export("13.05", pair(0, "EnemiesBlinded"), [])
        fields = export / "fields.parquet"
        original = fields.read_bytes()
        code, _stdout, stderr = self.run_main(
            "--export", str(export), "--out", str(fields)
        )
        self.assertEqual(code, 1)
        self.assertIn("refusing to overwrite input", stderr)
        self.assertEqual(fields.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
