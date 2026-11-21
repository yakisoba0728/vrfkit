"""Ensure a uniform audit cannot turn missing work or preserved loss into a pass."""
from copy import deepcopy
from pathlib import Path
import unittest
from unittest.mock import patch
import json
import subprocess
import tempfile
from contextlib import ExitStack

import pyarrow as pa
import pyarrow.parquet as pq

from support import TempDirTestCase
import verify_build_corpus as audit


#: Each sink event tally and the NetStats counter it must equal.
SINK_NET_PAIRS = (("sink_rpcs_emitted", "rpcs"), ("sink_actor_opens", "actor_opens"),
                  ("sink_actor_closes", "actor_closes"),
                  ("sink_content_blocks", "content_blocks"))


def guid_counts(**changes):
    counts = dict.fromkeys(audit.baseline.GUID_CROSSCHECK_KEYS, 0)
    counts.update(changes)
    return counts


def manifest():
    net = dict.fromkeys(audit.NET_ZERO, 0)
    net.update(content_blocks=100, skipped_bits=20, rpc_stream_failures=2,
               unresolved_rpc_payloads_preserved=2, rpcs=40, actor_opens=7,
               actor_closes=5)
    sink = dict.fromkeys(audit.SINK_ZERO, 0)
    sink.update(overlay_decoded_ok=80, overlay_raw_or_skip=3, overlay_not_in_table=12,
                overlay_no_field_name=5, struct_blobs_decoded=2,
                rpc_suffix_bits_dropped=4, overlay_handle_conflicts_refused=1,
                cnc_bruteforce_payloads_attempted=6, cnc_bruteforce_payloads_unwalked=0,
                movement_component_streams=3, movement_component_complete=3,
                movement_component_errors=0, movement_component_empty=0,
                movement_component_envelopes=3, movement_component_payload_bytes=90,
                movement_component_unused_bits=5, movement_component_input_headers=3,
                movement_component_discrepancy_headers=2,
                active_blinds_empty_trailers=1,
                route_children_focus_projectiles=2, focus_projectiles_empty_trailers=3,
                focus_projectiles_declaration_refusals=4,
                sink_rpcs_emitted=40, sink_actor_opens=7, sink_actor_closes=5,
                sink_content_blocks=100)
    quality = dict.fromkeys(("content_blocks_lost", "event_trailing_bytes",
                            "replay_data_trailing_bytes", "event_layout_mismatches",
                            "overlay_error_buckets", "overlay_errors_reported",
                            "unknown_chunks"), 0)
    quality.update(checkpoints_enabled=True, net=net, sink=sink,
                   checkpoints={"net": deepcopy(net), "sink": deepcopy(sink),
                                "checkpoint_chunks": 1, "checkpoint_trailing_bytes": 0})
    return {"quality": quality}


class ManifestTests(unittest.TestCase):
    def test_obsolete_movement_contract_is_refused_in_each_pass(self):
        for scope in ("main", "checkpoint"):
            for key in audit.summary_counters.LEGACY_MOVEMENT_COUNTERS:
                with self.subTest(scope=scope, key=key):
                    data = manifest()
                    target = data["quality"] if scope == "main" else data["quality"]["checkpoints"]
                    target["sink"][key] = 0
                    with self.assertRaisesRegex(ValueError, "obsolete movement counter contract"):
                        audit.manifest_counts(data)

    def test_preserved_unknown_rpc_is_not_loss(self):
        counts, failures = audit.manifest_counts(manifest())
        self.assertEqual(failures, [])
        self.assertEqual(counts["main_rpc_loss"], 0)
        self.assertEqual(counts["main_overlay_offered"], 100)
        self.assertEqual(counts["main_skipped_bits"], 20)
        self.assertEqual(counts["checkpoint_rpc_suffix_bits_dropped"], 4)
        self.assertEqual(counts["main_route_children_focus_projectiles"], 2)
        self.assertEqual(counts["checkpoint_focus_projectiles_empty_trailers"], 3)
        self.assertEqual(counts["main_focus_projectiles_declaration_refusals"], 4)

    def test_each_failure_counter_is_observed_in_each_pass(self):
        for scope in ("main", "checkpoint"):
            for category, keys in (("net", audit.NET_ZERO), ("sink", audit.SINK_ZERO)):
                for key in keys:
                    with self.subTest(scope=scope, category=category, key=key):
                        data = manifest()
                        target = data["quality"] if scope == "main" else data["quality"]["checkpoints"]
                        target[category][key] = 1
                        self.assertIn(f"{scope}_{key}=1", audit.manifest_counts(data)[1])

    def test_package_map_export_bunches_fail_each_pass(self):
        """Named here, not read from NET_ZERO, so dropping either term fails."""
        for scope in ("main", "checkpoint"):
            for key in ("package_map_exports", "rep_layout_export_bunches"):
                with self.subTest(scope=scope, key=key):
                    data = manifest()
                    target = data["quality"] if scope == "main" else data["quality"]["checkpoints"]
                    target["net"][key] = 1
                    self.assertIn(f"{scope}_{key}=1", audit.manifest_counts(data)[1])

    def test_each_run_level_count_must_be_zero(self):
        for key in ("content_blocks_lost", "unknown_chunks", "overlay_errors_reported"):
            with self.subTest(key=key):
                data = manifest()
                data["quality"][key] = 2
                self.assertIn(f"{key}=2", audit.manifest_counts(data)[1])

    def test_invalid_counters_are_not_zero(self):
        for bad in (None, -1, True, "0"):
            with self.subTest(bad=bad):
                data = manifest()
                data["quality"]["checkpoints"]["net"]["transform_failures"] = bad
                with self.assertRaises(ValueError):
                    audit.manifest_counts(data)

    def test_every_counter_the_manifest_carries_is_recorded_and_required(self):
        """Each is recorded under its pass's prefix, and deleting any one raises
        rather than reading as 0."""
        counts, failures = audit.manifest_counts(manifest())
        self.assertEqual(failures, [])
        quality = manifest()["quality"]
        places = [((), key) for key in quality if key not in ("checkpoints_enabled", "net",
                                                                "sink", "checkpoints")]
        places += [(("checkpoints",), key) for key in ("checkpoint_chunks",
                                                          "checkpoint_trailing_bytes")]
        for scope, path in (("main", ()), ("checkpoint", ("checkpoints",))):
            for category in ("net", "sink"):
                block = quality["checkpoints"] if path else quality
                for key, value in block[category].items():
                    self.assertEqual(counts[f"{scope}_{key}"], value, f"{scope}_{key}")
                    places.append((path + (category,), key))
        for path, key in places:
            with self.subTest(path=path, key=key):
                data = manifest()
                target = data["quality"]
                for part in path:
                    target = target[part]
                del target[key]
                with self.assertRaises(KeyError):
                    audit.manifest_counts(data)

    def test_native_counter_contract_and_positive_work_are_required_in_each_pass(self):
        for scope in ("main", "checkpoint"):
            for key, value in (("movement_component_streams", 4),
                               ("movement_component_envelopes", 2),
                               ("movement_component_unused_bits", 22),
                               ("movement_component_input_headers", 2),
                               ("movement_component_discrepancy_headers", 4)):
                with self.subTest(scope=scope, key=key):
                    data = manifest()
                    target = data["quality"] if scope == "main" else data["quality"]["checkpoints"]
                    target["sink"][key] = value
                    self.assertTrue(audit.manifest_counts(data)[1])
            data = manifest()
            target = data["quality"] if scope == "main" else data["quality"]["checkpoints"]
            target["sink"].update(dict.fromkeys(audit.summary_counters.MOVEMENT_COUNTERS, 0))
            self.assertEqual(audit.manifest_counts(data)[1],
                             ["main: native movement counters: no movement component decoding evidence"]
                             if scope == "main" else [], scope)

    def test_sink_event_tallies_must_equal_the_framing_counts(self):
        """The sink counts RPCs, actor opens and closes and content blocks in
        callbacks vrf-net invokes beside its own counters, so the two must be
        equal; a difference means the sink's bookkeeping is broken."""
        counts, failures = audit.manifest_counts(manifest())
        self.assertEqual(failures, [])
        self.assertEqual(counts["checkpoint_sink_content_blocks"], 100)
        for scope in ("main", "checkpoint"):
            for sink_key, net_key in SINK_NET_PAIRS:
                for delta in (-1, 1):
                    with self.subTest(scope=scope, key=sink_key, delta=delta):
                        data = manifest()
                        target = data["quality"] if scope == "main" else data["quality"]["checkpoints"]
                        target["sink"][sink_key] += delta
                        failures = audit.manifest_counts(data)[1]
                        self.assertTrue(
                            any(f.startswith(f"{scope}_{sink_key}=") and net_key in f
                                for f in failures), failures)

    def test_checkpoint_trailing_bytes_must_be_zero(self):
        """The checkpoint twin of replay_data_trailing_bytes: a failure when
        nonzero or not a count."""
        counts, failures = audit.manifest_counts(manifest())
        self.assertEqual((counts["checkpoint_trailing_bytes"], failures), (0, []))
        data = manifest()
        data["quality"]["checkpoints"]["checkpoint_trailing_bytes"] = 7
        self.assertIn("checkpoint_trailing_bytes=7", audit.manifest_counts(data)[1])
        for bad in (None, -1, True, "0"):
            with self.subTest(bad=bad):
                data = manifest()
                data["quality"]["checkpoints"]["checkpoint_trailing_bytes"] = bad
                with self.assertRaises(ValueError):
                    audit.manifest_counts(data)

    def test_lost_or_overcounted_rpc_fails(self):
        for preserved in (1, 3):
            data = manifest()
            data["quality"]["checkpoints"]["net"]["unresolved_rpc_payloads_preserved"] = preserved
            self.assertTrue(audit.manifest_counts(data)[1])

    def test_checkpoint_flag_cannot_be_omitted(self):
        data = manifest()
        data["quality"]["checkpoints_enabled"] = False
        with self.assertRaises(ValueError):
            audit.manifest_counts(data)


class ValidationTests(unittest.TestCase):
    TEXT = "Branch: ++Ares-Core+release-11.06\nORACLE PASS RATE: 100.000000% (10 / 10 blocks passed)"

    def test_valid_run_has_positive_exact_oracle_counts(self):
        branch, counts = audit.validation_counts(self.TEXT, 0)
        self.assertEqual(branch, "++Ares-Core+release-11.06")
        self.assertEqual(counts["oracle_scored_blocks"], 10)

    def test_exit_code_missing_counts_and_rounded_rate_cannot_pass(self):
        for text, code in ((self.TEXT, 1), ("", 0),
                           (self.TEXT.replace("10 / 10", "9 / 10"), 0),
                           (self.TEXT.replace("10 / 10", "0 / 0"), 0)):
            with self.subTest(text=text, code=code), self.assertRaises(ValueError):
                audit.validation_counts(text, code)

    def test_summary_retains_failures_and_absent_checkpoint_evidence(self):
        rows = [dict(branch="a", sha256="x", failures=[], counts={"typed_values_compared": 4}),
                dict(branch="a", sha256="y", failures=["bad"], counts={})]
        build = audit.summarize(rows)["a"]
        self.assertEqual((build["replays"], build["passed"], build["failed"]), (2, 1, 1))
        self.assertEqual(build["checkpoint_evidence"], "absent")


class AuditExecutionTests(TempDirTestCase):
    """Exercise the orchestration seam with controlled external process outputs."""
    def setUp(self):
        self.root = self.tmp()
        self.replay = self.root / "private.vrf"
        self.replay.write_bytes(b"replay fixture")
        self.digest = audit.sha256_file(self.replay)
        self.work = self.root / "work"
        self.work.mkdir()
        self.data = manifest()
        self.data["replay_build"] = "++Ares-Core+release-11.06"
        self.evidence = {"fields": {"test field": {"rows": 4}}, "failure_count": 0,
                         "typed_mismatch_count": 0, "missing": []}
        self.codes = [0, 0]
        self.write_manifest = True
        self.guid_counts = guid_counts(indexed_joined=5, indexed_path_equal=5)

    def process(self, args, **kwargs):
        command = args[1]
        if command == "export":
            self.assertIn("--checkpoints", args)
            output = Path(args[args.index("--out") + 1])
            output.mkdir()
            if self.write_manifest:
                (output / "manifest.json").write_text(json.dumps(self.data), encoding="utf-8")
        return subprocess.CompletedProcess(args, self.codes[command == "export"],
                                           stdout=ValidationTests.TEXT if command == "validate" else "export", stderr="")

    def run_audit(self, *, error=None, compare_error=None, mutate_input=False):
        def evidence_result(*args, **kwargs):
            self.assertTrue(kwargs["compare_typed"])
            if mutate_input:
                self.replay.write_bytes(b"changed")
            return self.evidence
        with ExitStack() as stack:
            run = stack.enter_context(patch.object(audit.subprocess, "run", side_effect=error or self.process))
            stack.enter_context(patch.object(audit, "check_export", side_effect=compare_error,
                                            return_value=({"fields": {"rows": 4, "bytes": 80}},
                                                          self.guid_counts)))
            stack.enter_context(patch.object(audit.evidence, "validate", side_effect=evidence_result))
            result = audit.audit_one((self.digest, self.replay), Path("vrfkit.exe"), self.work, [])
        saved = json.loads((self.work / self.digest / "result.json").read_text(encoding="utf-8"))
        self.assertEqual(result, saved)
        return result, run

    def test_success_runs_validation_checkpoint_export_and_independent_comparison(self):
        result, run = self.run_audit()
        self.assertEqual(result["failures"], [])
        self.assertEqual(result["counts"]["typed_values_compared"], 4)
        self.assertEqual([call.args[0][1] for call in run.call_args_list], ["validate", "export"])

    def test_failed_validation_does_not_continue_to_export(self):
        self.codes[0] = 1
        result, run = self.run_audit()
        self.assertTrue(result["failures"])
        self.assertEqual(run.call_count, 1)

    def test_failed_export_is_recorded(self):
        self.codes[1] = 1
        self.assertTrue(self.run_audit()[0]["failures"])

    def test_branch_mismatch_is_recorded(self):
        self.data["replay_build"] = "++Ares-Core+release-13.06"
        self.assertTrue(self.run_audit()[0]["failures"])

    def test_missing_manifest_cannot_pass(self):
        self.write_manifest = False
        self.assertTrue(self.run_audit()[0]["failures"])

    def test_array_errors_survive_successful_processes(self):
        self.data["quality"]["sink"]["array_errors"] = 2
        self.data["quality"]["checkpoints"]["sink"]["array_leaf_decode_errors"] = 1
        result, _ = self.run_audit()
        self.assertIn("main_array_errors=2", result["failures"])
        self.assertIn("checkpoint_array_leaf_decode_errors=1", result["failures"])

    def test_a_sink_tally_mismatch_fails_the_replay(self):
        self.data["quality"]["checkpoints"]["sink"]["sink_actor_closes"] = 4
        result, _ = self.run_audit()
        self.assertTrue(any(f.startswith("checkpoint_sink_actor_closes=4")
                            for f in result["failures"]), result["failures"])

    def test_missing_counter_cannot_pass(self):
        del self.data["quality"]["sink"]["array_errors"]
        self.assertTrue(self.run_audit()[0]["failures"])

    def test_no_observed_values_cannot_pass(self):
        self.evidence["fields"] = {}
        self.assertIn("no observed evidence values to compare", self.run_audit()[0]["failures"])

    def test_mismatching_values_cannot_pass(self):
        self.evidence["typed_mismatch_count"] = 1
        self.assertIn("typed/raw comparison failed", self.run_audit()[0]["failures"])

    def test_invalid_evidence_width_cannot_pass(self):
        self.evidence["failure_count"] = 1
        self.assertIn("typed/raw comparison failed", self.run_audit()[0]["failures"])

    def test_input_mutation_cannot_pass(self):
        self.assertIn("input changed during verification", self.run_audit(mutate_input=True)[0]["failures"])

    def test_export_cross_check_failure_cannot_pass(self):
        self.assertTrue(self.run_audit(compare_error=ValueError("row mismatch"))[0]["failures"])

    def test_timeout_is_recorded_without_leaking_the_private_path(self):
        error = subprocess.TimeoutExpired([str(self.replay)], 1)
        result, _ = self.run_audit(error=error)
        self.assertTrue(result["failures"])
        self.assertNotIn(str(self.root), json.dumps(result))
        self.assertIn(repr(str(self.replay)), (self.work / self.digest / "error.txt").read_text(encoding="utf-8"))

    def test_guid_crosscheck_counts_reach_the_result_zeros_included(self):
        result, _ = self.run_audit()
        self.assertEqual(result["failures"], [])
        self.assertEqual(result["tables"], {"fields": {"rows": 4, "bytes": 80}})
        self.assertEqual(result["counts"]["guid_crosscheck_indexed_joined"], 5)
        self.assertEqual(result["counts"]["guid_crosscheck_indexed_path_differs"], 0)
        self.assertEqual(
            {key for key in result["counts"] if key.startswith("guid_crosscheck_")},
            {f"guid_crosscheck_{key}" for key in audit.baseline.GUID_CROSSCHECK_KEYS})


class CheckExportTests(unittest.TestCase):
    """`check_export` runs every export check with checkpoints on and surfaces
    the GUID cross-check's counts; the checks are the baseline guard's."""

    def run_check_export(self, counts, errors=(), lies=()):
        with tempfile.TemporaryDirectory() as temp, ExitStack() as stack:
            directory = Path(temp)
            printed = {key: i for i, key in enumerate(audit.summary_counters.WHERE, 1)}
            text = "\n".join(audit.summary_counters.render(line, printed)
                             for line in audit.summary_counters.SPEC)
            for name in audit.baseline.PARQUET_FILES + audit.baseline.CHECKPOINT_PARQUET_FILES:
                pq.write_table(pa.table({"value": [1]}), directory / f"{name}.parquet")
            stack.enter_context(patch.object(audit.overlay, "reconcile", return_value=None))
            stack.enter_context(patch.object(audit.baseline, "cross_checks",
                                             return_value=list(lies)))
            checks = stack.enter_context(patch.object(
                audit.baseline, "export_errors", return_value=(list(errors), counts)))
            try:
                return audit.check_export(text, directory)
            finally:
                checks.assert_called_once_with(directory, printed, True)

    def test_passing_checks_return_the_crosscheck_counts_with_the_tables(self):
        counts = guid_counts(indexed_joined=7, indexed_path_equal=7)
        tables, returned = self.run_check_export(counts)
        self.assertEqual(returned, counts)
        self.assertEqual(tables["checkpoint_guid_entries"]["rows"], 1)

    def test_a_failing_check_fails_the_export_and_keeps_the_counts(self):
        counts = guid_counts(indexed_joined=7, indexed_path_equal=5, indexed_path_differs=2)
        for errors, lies in ((["2 indexed entries differ"], []), ([], ["2 indexed entries differ"])):
            with self.subTest(lies=lies), self.assertRaises(ValueError) as raised:
                self.run_check_export(counts, errors, lies)
            self.assertIn("2 indexed entries differ", str(raised.exception))
            self.assertIn("indexed path differs 2", str(raised.exception))


class AuditCommandTests(TempDirTestCase):
    def setUp(self):
        self.root = self.tmp()
        self.corpus = self.root / "corpus"
        self.corpus.mkdir()
        (self.corpus / "a.vrf").write_bytes(b"sample")
        self.exe = self.root / "parser.exe"
        self.exe.write_bytes(b"executable")
        self.output = self.root / "report.json"
        self.args = ["--exe", str(self.exe), "--corpus", str(self.corpus),
                     "--work-dir", str(self.root / "work"), "--output", str(self.output)]

    def run_command(self, *, checkpoint=True, failure=False, mutate_exe=False):
        def inspect(entry, *args):
            if mutate_exe:
                self.exe.write_bytes(b"changed executable")
            return {"sha256": entry[0], "branch": "++Ares-Core+release-11.06",
                    "failures": ["array error"] if failure else [],
                    "counts": {"checkpoint_content_blocks": int(checkpoint),
                               "checkpoint_overlay_decoded_ok": int(checkpoint)}}
        with patch.object(audit, "audit_one", side_effect=inspect):
            code = audit.main(self.args)
        return code, json.loads(self.output.read_text(encoding="utf-8"))

    def test_content_duplicates_are_checked_once_including_nested_uppercase_vrf(self):
        nested = self.corpus / "nested"
        nested.mkdir()
        (nested / "duplicate.VRF").write_bytes(b"sample")
        (nested / "ignored.txt").write_bytes(b"other")
        code, report = self.run_command()
        self.assertEqual(code, 0)
        self.assertEqual(report["provenance"]["unique_replays"], 1)
        self.assertEqual(report["provenance"]["duplicates"], 1)
        self.assertEqual(report["build_errors"], [])

    def test_missing_checkpoint_work_fails_the_entire_command(self):
        code, report = self.run_command(checkpoint=False)
        self.assertEqual(code, 1)
        self.assertTrue(report["build_errors"])

    def test_failure_report_is_written_before_nonzero_exit(self):
        code, report = self.run_command(failure=True)
        self.assertEqual(code, 1)
        self.assertEqual(report["failures"][0]["errors"], ["array error"])

    def test_executable_mutation_fails_the_entire_command(self):
        code, report = self.run_command(mutate_exe=True)
        self.assertEqual(code, 1)
        self.assertTrue(report["executable_changed"])

    def test_existing_output_is_never_overwritten(self):
        self.output.write_text("keep me", encoding="utf-8")
        with self.assertRaises(SystemExit) as raised:
            audit.main(self.args)
        self.assertEqual(raised.exception.code, 2)
        self.assertEqual(self.output.read_text(encoding="utf-8"), "keep me")

    def test_empty_corpus_is_an_error(self):
        empty = self.root / "empty"
        empty.mkdir()
        self.args[self.args.index("--corpus") + 1] = str(empty)
        with self.assertRaises(SystemExit) as raised:
            audit.main(self.args)
        self.assertEqual(raised.exception.code, 2)
        self.assertFalse(self.output.exists())
