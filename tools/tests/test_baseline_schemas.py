import json
from pathlib import Path


from support import TempDirTestCase
import check_baseline_schemas as schemas


class BaselineSchemaTests(TempDirTestCase):
    def test_focus_and_native_movement_counts_are_required_in_committed_baselines(self):
        names = ("route_children_focus_projectiles", "focus_projectiles_empty_trailers",
                 "focus_projectiles_declaration_refusals", *schemas.sc.MOVEMENT_COUNTERS)
        for baseline in ("export_02d4d478.json", "checkpoint_02d4d478.json"):
            path = schemas.BASELINES / baseline
            data = json.loads(path.read_text(encoding="utf-8"))
            self.assertEqual(schemas.validate_export_baseline(path, data), [])
            prefixes = ("", "cp_") if baseline.startswith("checkpoint_") else ("",)
            for key in (prefix + name for prefix in prefixes for name in names):
                with self.subTest(baseline=baseline, key=key):
                    missing = json.loads(json.dumps(data))
                    del missing["counters"][key]
                    self.assertIn(key, " ".join(schemas.validate_export_baseline(path, missing)))

    def test_committed_baselines_are_schema_valid_and_cross_consistent(self):
        self.assertEqual(schemas.validate_repository(), [])

    def test_obsolete_movement_contract_is_named_and_refused(self):
        for name in ("export_02d4d478.json", "checkpoint_02d4d478.json"):
            path = schemas.BASELINES / name
            prefixes = ("", "cp_") if name.startswith("checkpoint_") else ("",)
            for prefix in prefixes:
                with self.subTest(name=name, prefix=prefix):
                    data = json.loads(path.read_text(encoding="utf-8"))
                    for key in schemas.sc.MOVEMENT_COUNTERS:
                        del data["counters"][prefix + key]
                    data["counters"].update(
                        dict.fromkeys((prefix + key for key in schemas.sc.LEGACY_MOVEMENT_COUNTERS), 0))
                    problems = " ".join(schemas.validate_export_baseline(path, data))
                    self.assertIn("obsolete movement counter contract", problems)
                    # A single legacy key beside the current set is refused too.
                    data = json.loads(path.read_text(encoding="utf-8"))
                    data["counters"][prefix + "movement_envelope_trailers"] = 0
                    problems = " ".join(schemas.validate_export_baseline(path, data))
                    self.assertIn("obsolete movement counter contract", problems)

    def test_an_export_without_a_measured_sha256_fails(self):
        for digest in (None, "not-measured"):
            with self.subTest(digest=digest):
                record = {"rows": 0, "bytes": 0}
                if digest is not None:
                    record["sha256"] = digest
                data = {"replay": "sample.vrf",
                        "counters": {key: 0 for key in schemas.MAIN_COUNTERS},
                        "parquet": {name: dict(record) for name in schemas.MAIN_PARQUET}}
                problems = schemas.validate_export_baseline(Path("export_sample.json"), data)
                self.assertTrue(any("sha256" in problem for problem in problems), problems)

    def test_checkpoint_baseline_requires_actor_and_guid_tables_and_counters(self):
        data = {
            "replay": "sample.vrf",
            "counters": {key: 0 for key in schemas.MAIN_COUNTERS | schemas.CHECKPOINT_ONLY_COUNTERS},
            "parquet": {
                name: {"rows": 0, "bytes": 0, "sha256": "a" * 64}
                for name in (*schemas.MAIN_PARQUET, *schemas.CHECKPOINT_PARQUET_FILES)
            },
        }
        path = Path("checkpoint_sample.json")
        self.assertEqual(schemas.validate_export_baseline(path, data), [])

        missing_table = json.loads(json.dumps(data))
        del missing_table["parquet"]["checkpoint_actors"]
        problems = schemas.validate_export_baseline(path, missing_table)
        self.assertTrue(any("checkpoint_actors" in problem for problem in problems), problems)

        missing_counter = json.loads(json.dumps(data))
        del missing_counter["counters"]["cp_net_guid_rows_written"]
        problems = schemas.validate_export_baseline(path, missing_counter)
        self.assertTrue(any("cp_net_guid_rows_written" in problem for problem in problems), problems)

    def test_unknown_baseline_json_fails_closed(self):
        root = self.tmp()
        (root / "unvalidated.json").write_text("{}", encoding="utf-8")
        problems = schemas.validate_repository(root)
        self.assertTrue(any("unvalidated.json" in p and "unknown" in p for p in problems),
                        problems)

    def test_bench_rejects_boolean_timing_and_non_replay_name(self):
        problems = schemas.validate_bench_baseline(
            Path("bench.json"), {"export": True, "replay": 7}
        )
        self.assertTrue(any("export" in p for p in problems), problems)
        self.assertTrue(any("replay" in p for p in problems), problems)

    def test_bench_accepts_the_checkpoint_slot_and_no_other_key(self):
        """Key sets are compared for equality, not as a subset."""
        base = {"export": 1.0, "replay": "m.vrf"}
        bench = Path("bench.json")
        self.assertEqual(schemas.validate_bench_baseline(
            bench, dict(base, export_checkpoints=2.0)), [])
        problems = schemas.validate_bench_baseline(bench, dict(base, export_checkpoints=0.0))
        self.assertIn("export_checkpoints must be a positive finite float", " ".join(problems))
        problems = schemas.validate_bench_baseline(bench, dict(base, export_debug=2.0))
        self.assertIn("export_debug", " ".join(problems))

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
