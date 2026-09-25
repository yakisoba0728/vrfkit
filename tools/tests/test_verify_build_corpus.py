"""Ensure a uniform audit cannot turn missing work or preserved loss into a pass."""
from copy import deepcopy
from pathlib import Path
import sys
import unittest

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


if __name__ == "__main__":
    unittest.main()
