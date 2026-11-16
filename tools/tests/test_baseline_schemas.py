import json
from pathlib import Path


from support import TempDirTestCase
import check_baseline_schemas as schemas


class BaselineSchemaTests(TempDirTestCase):
    def test_focus_counts_are_required_currently_and_never_invented_in_archives(self):
        names = ("route_children_focus_projectiles", "focus_projectiles_empty_trailers")
        archived_path = schemas.BASELINES / "export_02d4d478.json"
        archived = json.loads(archived_path.read_text(encoding="utf-8"))
        self.assertEqual(schemas.validate_export_baseline(archived_path, archived), [])
        self.assertTrue(all(name not in archived["counters"] for name in names))
        current = {"replay": "synthetic.vrf",
                   "counters": dict.fromkeys(schemas.MAIN_COUNTERS, 0),
                   "parquet": {name: {"rows": 0, "bytes": 0, "sha256": "a" * 64}
                               for name in schemas.MAIN_PARQUET}}
        path = Path("export_synthetic.json")
        self.assertEqual(schemas.validate_export_baseline(path, current), [])
        for name in names:
            missing = json.loads(json.dumps(current))
            del missing["counters"][name]
            self.assertIn(name, " ".join(schemas.validate_export_baseline(path, missing)))
            invented = json.loads(json.dumps(archived))
            invented["counters"][name] = 0
            self.assertTrue(schemas.validate_export_baseline(archived_path, invented))

    def test_committed_baselines_are_schema_valid_and_cross_consistent(self):
        self.assertEqual(schemas.validate_repository(), [])

    def test_archived_movement_contract_cannot_mix_current_or_missing_keys(self):
        for name in ("export_02d4d478.json", "checkpoint_02d4d478.json"):
            path = schemas.BASELINES / name
            data = json.loads(path.read_text(encoding="utf-8"))
            self.assertEqual(schemas.validate_export_baseline(path, data), [])
            data["counters"]["movement_component_streams"] = 0
            self.assertTrue(schemas.validate_export_baseline(path, data))
            del data["counters"]["movement_component_streams"]
            del data["counters"]["movement_envelope_trailer_bits"]
            self.assertTrue(schemas.validate_export_baseline(path, data))

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
