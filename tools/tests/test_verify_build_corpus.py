"""Ensure a uniform audit cannot turn missing work or preserved loss into a pass."""
from copy import deepcopy
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
import json
import subprocess
import tempfile
from contextlib import ExitStack, redirect_stdout, redirect_stderr
from io import StringIO

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import verify_build_corpus as audit


def manifest():
    net = dict.fromkeys(audit.NET_ZERO, 0)
    net.update(content_blocks=100, skipped_bits=20, rpc_stream_failures=2,
               unresolved_rpc_payloads_preserved=2)
    sink = dict.fromkeys(audit.SINK_ZERO, 0)
    sink.update(overlay_decoded_ok=80, overlay_raw_or_skip=3, overlay_not_in_table=12,
                overlay_no_field_name=5, struct_blobs_decoded=2,
                rpc_suffix_bits_dropped=4, overlay_handle_conflicts_refused=1)
    quality = dict.fromkeys(("content_blocks_lost", "event_trailing_bytes",
                            "replay_data_trailing_bytes", "event_layout_mismatches",
                            "overlay_error_buckets", "overlay_errors_reported"), 0)
    quality.update(checkpoints_enabled=True, net=net, sink=sink,
                   checkpoints={"net": deepcopy(net), "sink": deepcopy(sink),
                                "checkpoint_chunks": 1})
    return {"quality": quality}


class ManifestTests(unittest.TestCase):
    def test_preserved_unknown_rpc_is_not_loss(self):
        counts, failures = audit.manifest_counts(manifest())
        self.assertEqual(failures, [])
        self.assertEqual(counts["main_rpc_loss"], 0)
        self.assertEqual(counts["main_overlay_offered"], 100)
        self.assertEqual(counts["main_skipped_bits"], 20)
        self.assertEqual(counts["checkpoint_rpc_suffix_bits_dropped"], 4)

    def test_each_failure_counter_is_observed_in_each_pass(self):
        for scope in ("main", "checkpoint"):
            for category, keys in (("net", audit.NET_ZERO), ("sink", audit.SINK_ZERO)):
                for key in keys:
                    with self.subTest(scope=scope, category=category, key=key):
                        data = manifest()
                        target = data["quality"] if scope == "main" else data["quality"]["checkpoints"]
                        target[category][key] = 1
                        self.assertIn(f"{scope}_{key}=1", audit.manifest_counts(data)[1])

    def test_missing_and_invalid_counters_are_not_zero(self):
        for bad in (None, -1, True, "0"):
            with self.subTest(bad=bad):
                data = manifest()
                data["quality"]["checkpoints"]["net"]["transform_failures"] = bad
                with self.assertRaises(ValueError):
                    audit.manifest_counts(data)
        data = manifest()
        del data["quality"]["net"]["field_stream_failures"]
        with self.assertRaises(KeyError):
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


class AuditExecutionTests(unittest.TestCase):
    """Exercise the orchestration seam with controlled external process outputs."""
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
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

    def process(self, args, **kwargs):
        command = args[1]
        if command == "export":
            self.assertIn("--checkpoints", args)
            output = Path(args[args.index("--out") + 1])
            output.mkdir()
            if self.write_manifest:
                (output / "manifest.json").write_text(json.dumps(self.data))
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
                                            return_value={"fields": {"rows": 4, "bytes": 80}}))
            stack.enter_context(patch.object(audit.evidence, "validate", side_effect=evidence_result))
            result = audit.audit_one((self.digest, self.replay), Path("vrfkit.exe"), self.work, [])
        saved = json.loads((self.work / self.digest / "result.json").read_text())
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
        self.assertIn(repr(str(self.replay)), (self.work / self.digest / "error.txt").read_text())


class AuditCommandTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
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
        with patch.object(audit, "audit_one", side_effect=inspect), redirect_stdout(StringIO()):
            code = audit.main(self.args)
        return code, json.loads(self.output.read_text())

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
        self.output.write_text("keep me")
        with redirect_stderr(StringIO()), self.assertRaises(SystemExit) as raised:
            audit.main(self.args)
        self.assertEqual(raised.exception.code, 2)
        self.assertEqual(self.output.read_text(), "keep me")

    def test_empty_corpus_is_an_error(self):
        empty = self.root / "empty"
        empty.mkdir()
        self.args[self.args.index("--corpus") + 1] = str(empty)
        with redirect_stderr(StringIO()), self.assertRaises(SystemExit) as raised:
            audit.main(self.args)
        self.assertEqual(raised.exception.code, 2)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
