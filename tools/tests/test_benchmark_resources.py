"""Resource measurements must describe equivalent work, including failure paths."""

import contextlib
import io
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest import mock

from support import TempDirTestCase
import benchmark_resources as bench
import summary_counters as sc


def valid_manifest(checkpoints=False):
    def scope():
        net = dict.fromkeys(bench.NET_ZERO, 0)
        net.update(content_blocks=1, rpc_stream_failures=0, unresolved_rpc_payloads_preserved=0)
        sink = dict.fromkeys(bench.SINK_ZERO, 0)
        sink.update(overlay_decoded_ok=1, movement_envelope_trailers=1, movement_envelope_trailer_bits=24)
        for a, b in bench.TALLIES:
            sink[a] = net[b] = 1
        return {"net": net, "sink": sink}

    quality = {**dict.fromkeys(bench.QUALITY_ZERO, 0), **scope(),
               "checkpoints_enabled": checkpoints, "checkpoints": None}
    if checkpoints:
        quality["checkpoints"] = {**scope(), "checkpoint_chunks": 1,
                                  "checkpoint_trailing_bytes": 0, "checkpoint_frame_non_finite_times": 0}
    return {"source_file": "/private/example.vrf", "elapsed_ms": 1,
            "source_size_bytes": 6, "quality": quality,
            "players": [{"subject": "private-player-marker"}], "replay_build": "synthetic"}


def valid_log(checkpoints=False):
    counters = dict.fromkeys(sc.WHERE, 0)
    counters["overlay_decoded_ok"] = counters["overlay_rows_offered"] = 1
    return "\n".join(sc.render(line, counters) for line in sc.SPEC
                     if checkpoints or not line.keys[0].startswith("cp_"))


class MeasurementTests(TempDirTestCase):
    def test_known_rss_units_and_missing_resources(self):
        usage = SimpleNamespace(ru_utime=2.0, ru_stime=1.0, ru_maxrss=42,
                                ru_inblock=7, ru_oublock=9)
        linux = bench.resource_metrics(usage, 2.0, "Linux")
        self.assertEqual(linux["peak_rss_bytes"], 42 * 1024)
        self.assertEqual(linux["cpu_total_seconds"], 3.0)
        self.assertEqual(linux["cpu_utilization_percent"], 150.0)
        self.assertEqual(bench.resource_metrics(usage, 2.0, "Darwin")["peak_rss_bytes"], 42)
        self.assertIsNone(bench.resource_metrics(usage, 2.0, "Unknown")["peak_rss_bytes"])
        self.assertTrue(all(v is None for v in bench.resource_metrics(None, 2.0, "Windows").values()))

    def test_worker_captures_private_output_and_process_failure(self):
        directory = self.tmp()
        result = bench.run_worker([sys.executable, "-c", "print('PRIVATE'); raise SystemExit(7)"], directory, 10)
        self.assertEqual(result["exit_code"], 7)
        self.assertNotIn("PRIVATE", json.dumps(result))
        self.assertIn("PRIVATE", (directory / "parser.log").read_text(encoding="utf-8"))
        self.assertGreater(result["measurements"]["wall_seconds"], 0)

    def test_worker_timeout_fails_and_reaps_the_process(self):
        result = bench.run_worker([sys.executable, "-c", "import time; time.sleep(10)"], self.tmp(), 0.1)
        self.assertEqual(result["status"], "timeout")
        self.assertNotEqual(result["exit_code"], 0)
        self.assertLess(result["measurements"]["wall_seconds"], 5)

    def test_worker_missing_executable_is_explicit(self):
        result = bench.run_worker([str(self.tmp() / "absent")], self.tmp(), 10)
        self.assertEqual(result["status"], "launch_error")
        self.assertIsNone(result["exit_code"])

    def test_worker_rss_is_not_a_previous_childs_high_water_mark(self):
        high = bench.run_worker([sys.executable, "-c", "x=bytearray(64*1024*1024); print(len(x))"], self.tmp(), 10)
        low = bench.run_worker([sys.executable, "-c", "print('small')"], self.tmp(), 10)
        high_rss, low_rss = (x["measurements"]["peak_rss_bytes"] for x in (high, low))
        if high_rss is not None and low_rss is not None:
            self.assertGreater(high_rss - low_rss, 32 * 1024 * 1024)
        else:
            self.assertIsNone(high_rss)
            self.assertIsNone(low_rss)

    def test_paired_order_warmups_and_statistics(self):
        self.assertEqual(list(bench.plan(1, 2)), [
            ("warmup", 0, "baseline"), ("warmup", 0, "candidate"),
            ("measured", 0, "baseline"), ("measured", 0, "candidate"),
            ("measured", 1, "candidate"), ("measured", 1, "baseline")])
        rows = [{"phase": phase, "round": n, "variant": variant,
                 "measurements": {"wall_seconds": wall, "cpu_total_seconds": 0, "peak_rss_bytes": None}}
                for phase, n, variant, wall in (
                    ("warmup", 0, "baseline", 1000), ("measured", 0, "baseline", 4),
                    ("measured", 0, "candidate", 2), ("measured", 1, "candidate", 4),
                    ("measured", 1, "baseline", 8))]
        summary = bench.summarize(rows)
        self.assertEqual(summary["baseline"]["wall_seconds"]["median"], 6)
        self.assertEqual(summary["candidate"]["wall_seconds"]["median_absolute_deviation"], 1)
        ratios = bench.paired_ratios(rows)
        self.assertEqual(ratios["wall_seconds"]["candidate_over_baseline"], [0.5, 0.5])
        self.assertIsNone(ratios["cpu_total_seconds"])
        self.assertIsNone(ratios["peak_rss_bytes"])

    def test_invalid_measurement_and_partial_cpu_values_are_rejected(self):
        values = {"wall_seconds": 1.0, **bench.resource_metrics(None, 1.0, "Windows")}
        bench.validate_measurements(values)
        for key, value in (("wall_seconds", float("nan")), ("wall_seconds", 0),
                           ("peak_rss_bytes", -1), ("cpu_total_seconds", 1.0),
                           ("io_input_blocks", True)):
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                bench.validate_measurements({**values, key: value})


class EquivalenceTests(TempDirTestCase):
    def make_export(self, checkpoints=False):
        directory = self.tmp()
        for table in bench.MAIN_TABLES + (bench.CHECKPOINT_TABLES if checkpoints else ()):
            (directory / (table + ".parquet")).write_bytes(b"PAR1syntheticPAR1")
        (directory / "manifest.json").write_text(json.dumps(valid_manifest(checkpoints)), encoding="utf-8")
        log = self.tmp() / "parser.log"
        log.write_text(valid_log(checkpoints), encoding="utf-8")
        return directory, log

    def test_hashes_cover_raw_tables_and_private_manifest_values(self):
        directory, log = self.make_export()
        original = bench.fingerprint(directory, log, False, 6)
        self.assertNotIn("private-player-marker", json.dumps(original))
        self.assertNotIn("example.vrf", json.dumps(original))
        manifest = valid_manifest()
        manifest["source_file"] = "/another/private/path.vrf"
        manifest["elapsed_ms"] = 200
        (directory / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        changed = bench.fingerprint(directory, log, False, 6)
        self.assertTrue(bench.same_output(original, changed))
        manifest["players"][0]["subject"] = "changed-private-player"
        (directory / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        self.assertFalse(bench.same_output(original, bench.fingerprint(directory, log, False, 6)))
        (directory / "fields.parquet").write_bytes(b"PAR1MUTATION!PAR1")
        self.assertNotEqual(original["tables"]["fields.parquet"], bench.fingerprint(directory, log, False, 6)["tables"]["fields.parquet"])

    def test_missing_tables_and_invalid_envelopes_are_rejected(self):
        for action in ("missing", "magic", "extra"):
            with self.subTest(action=action):
                directory, log = self.make_export()
                if action == "missing":
                    (directory / "fields.parquet").unlink()
                elif action == "magic":
                    (directory / "fields.parquet").write_bytes(b"not parquet")
                else:
                    (directory / "unexpected.txt").write_text("extra", encoding="utf-8")
                with self.assertRaises(ValueError):
                    bench.fingerprint(directory, log, False, 6)

    def test_missing_invalid_and_error_counters_are_rejected(self):
        for mutation in ("missing", "negative", "boolean", "error", "lost_rpc", "no_work", "trailer", "tally"):
            with self.subTest(mutation=mutation):
                manifest = valid_manifest()
                net, sink = manifest["quality"]["net"], manifest["quality"]["sink"]
                if mutation == "missing":
                    del net["malformed_packets"]
                elif mutation == "negative":
                    net["malformed_packets"] = -1
                elif mutation == "boolean":
                    net["malformed_packets"] = False
                elif mutation == "error":
                    sink["array_errors"] = 1
                elif mutation == "lost_rpc":
                    net["rpc_stream_failures"] = 1
                elif mutation == "no_work":
                    sink["overlay_decoded_ok"] = 0
                elif mutation == "trailer":
                    sink["movement_envelope_trailer_bits"] = 23
                elif mutation == "tally":
                    sink["sink_rpcs_emitted"] = 2
                with self.assertRaises((ValueError, KeyError)):
                    bench.quality_check(manifest, False)

    def test_checkpoint_evidence_and_mode_are_required(self):
        manifest = valid_manifest(True)
        bench.quality_check(manifest, True)
        with self.assertRaises(ValueError):
            bench.quality_check(manifest, False)
        manifest["quality"]["checkpoints"]["checkpoint_chunks"] = 0
        with self.assertRaises(ValueError):
            bench.quality_check(manifest, True)

    def test_summary_missing_line_or_unreconciled_counters_fail(self):
        directory, log = self.make_export()
        original = valid_log()
        for text in ("\n".join(original.splitlines()[1:]),
                     original.replace("Rows offered: 1", "Rows offered: 2"),
                     original.replace("Array leaf errs: 0", "Array leaf errs: 1"),
                     original + "\nArray leaf errs: 1"):
            log.write_text(text, encoding="utf-8")
            with self.assertRaises(ValueError):
                bench.fingerprint(directory, log, False, 6)

    def test_nonfinite_and_duplicate_json_are_rejected(self):
        for text in ('{"x":NaN}', '{"x":Infinity}', '{"x":1,"x":2}'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                bench.strict_json(text)


class MainTests(TempDirTestCase):
    def setUp(self):
        self.root = self.tmp()
        self.replay = self.root / "private-replay-marker.vrf"
        self.replay.write_bytes(b"replay")

    def executable(self, name, mutation=""):
        script = self.root / (name + ".py")
        script.write_text(
            "import json, pathlib, sys\n"
            "out=pathlib.Path(sys.argv[sys.argv.index('--out')+1]); out.mkdir()\n"
            f"manifest={valid_manifest()!r}\n"
            f"tables={bench.MAIN_TABLES!r}\n"
            "for table in tables: (out/(table+'.parquet')).write_bytes(b'PAR1syntheticPAR1')\n"
            f"text={valid_log()!r}\n" + mutation + "\n"
            "(out/'manifest.json').write_text(json.dumps(manifest),encoding='utf-8')\n"
            "print(text)\n", encoding="utf-8")
        return script

    def run_benchmark(self, baseline, candidate, **kwargs):
        output = self.root / "report.json"
        work = self.root / "private-work"
        real_worker = bench.run_worker

        def python_script(command, directory, timeout):
            return real_worker([sys.executable, *command], directory, timeout)

        argv = ["--baseline-exe", str(baseline), "--candidate-exe", str(candidate),
                "--replay", str(self.replay), "--work-dir", str(work), "--output", str(output),
                "--warmups", "1", "--repeats", "2"]
        with mock.patch.object(bench, "run_worker", side_effect=python_script), contextlib.redirect_stdout(io.StringIO()) as stdout:
            result = bench.main([*argv, *kwargs.get("extra", [])])
        return result, json.loads(output.read_text(encoding="utf-8")), stdout.getvalue()

    def test_complete_run_is_paired_private_and_preserves_input(self):
        result, report, stdout = self.run_benchmark(self.executable("baseline"), self.executable("candidate"))
        self.assertEqual(result, 0, stdout)
        self.assertTrue(report["equivalent"])
        self.assertEqual(report["status"], "completed")
        self.assertEqual(len(report["samples"]), 6)
        self.assertEqual(report["summary"]["baseline"]["wall_seconds"]["samples"], 2)
        self.assertEqual(report["provenance_before"], report["provenance_after"])
        self.assertEqual(self.replay.read_bytes(), b"replay")
        combined = json.dumps(report) + stdout
        for marker in (str(self.root), "private-replay-marker", "private-player-marker"):
            self.assertNotIn(marker, combined)
        self.assertFalse(list((self.root / "private-work").glob("*/export")))

    def test_changed_raw_output_invalidates_even_equal_counters(self):
        mutation = "(out/'fields.parquet').write_bytes(b'PAR1MUTATION!PAR1')"
        result, report, _ = self.run_benchmark(self.executable("baseline"), self.executable("candidate", mutation))
        self.assertEqual(result, 1)
        self.assertFalse(report["equivalent"])
        self.assertIsNone(report["comparison"])
        self.assertNotIn("reference_output", report)

    def test_real_failed_export_does_not_become_a_fast_sample(self):
        result, report, _ = self.run_benchmark(self.executable("baseline"), self.executable("candidate", "raise SystemExit(9)"))
        self.assertEqual(result, 1)
        self.assertFalse(report["equivalent"])
        self.assertIsNone(report["comparison"])
        self.assertEqual(report["samples"][-1]["exit_code"], 9)

    def test_missing_summary_counter_prevents_comparison(self):
        result, report, _ = self.run_benchmark(self.executable("baseline"), self.executable("candidate", "text='missing summaries'"))
        self.assertEqual(result, 1)
        self.assertFalse(report["equivalent"])
        self.assertIsNone(report["comparison"])

    def test_input_mutation_is_detected_after_measurements(self):
        mutation = "pathlib.Path(sys.argv[2]).write_bytes(b'CHANGED')"
        result, report, _ = self.run_benchmark(self.executable("baseline"), self.executable("candidate", mutation))
        self.assertEqual(result, 1)
        self.assertFalse(report["equivalent"])
        self.assertNotEqual(report["provenance_before"]["input"], report["provenance_after"]["input"])
        self.assertIsNone(report["comparison"])

    def test_invalid_arguments_never_start_an_export(self):
        exe = self.executable("baseline")
        common = ["--baseline-exe", str(exe), "--candidate-exe", str(exe),
                  "--replay", str(self.replay), "--output", str(self.root / "report.json"),
                  "--work-dir", str(self.root / "private-work")]
        for extra in (["--repeats", "0"], ["--warmups", "-1"], ["--timeout", "nan"],
                      ["--timeout", "0"], ["--output", str(self.replay)],
                      ["--output", str(self.root / "private-work" / "report.json")],
                      ["--output", str(self.root / "new-root"), "--work-dir", str(self.root / "new-root" / "private")],
                      ["--output", str(self.root / "private-work" / "warmup-000-baseline" / "request.json")]):
            with self.subTest(extra=extra), mock.patch.object(bench, "run_worker") as run, contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as exc:
                    bench.main([*common, *extra])
                self.assertEqual(exc.exception.code, 2)
                run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
