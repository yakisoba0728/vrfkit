"""Guards for the corpus decode-error gate: every way it could print OK over
an export that failed, decoded nothing, or printed a line the gate never read.
The summary patterns themselves are test_summary_counters.py's.
"""
import ast
import contextlib
import re
import sys
import unittest
from pathlib import Path


from support import TempDirTestCase, run_cli
import check_decode_errors_corpus as guard

#: The main-pass sink lines, failure counters at zero. Values from a real 13.02
#: `--checkpoints` export log, except synthetic CNC, movement, route, FocusProjectiles trailer and walk lines.
CLEAN_SINK = """
Movement rows:     2407298
Movement errors:   0
Array decode:      25052 elements / 96076 fields / 0 errors / 0 truncations
Array residual:    0 root bits / 0 nested bits / 0 implicit ends
Array leaf errs:   0
Route children:    41 player info / 12 rewards / 30 selected / 55 kills / 380 active effects / 21 ignore actors / 9 blinds / 144 projectile path / 0 focus projectiles
Truncated RPCs:    0
RPC param walks:   338107
CNC brute force:   454 attempted / 0 unwalked
Movement components: 3 attempted / 3 complete / 0 errors / 0 empty
Component envelopes: 3 validated / 90 payload bytes / 5 unused bits
Component headers: 3 input / 2 discrepancy
ActiveBlinds trailers: 3 empty deltas
FocusProjectiles trailers: 0 empty deltas
FocusProjectiles refused: 0 declaration mismatches
"""

#: Measured on a live export: 742738 + 0 + 72644 + 171605 + 1996 = 988983
#: matches "Rows offered" only with `No field name` in the sum.
LIVE_EXPORT = """
Rows offered:      988983
Decoded OK:        742738
Decode errors:     0
Raw/Skip:          72644
Not in table:      171605
No field name:     1996
Struct blobs:      63 decoded / 0 failed
Reward opaque:     4470 empty variants
""" + CLEAN_SINK

#: The `=== Checkpoints ===` block, as summary.rs's print_checkpoints prints it.
CLEAN_WITH_CHECKPOINTS = LIVE_EXPORT + """
=== Checkpoints ===
  Checkpoints:      12
  Overlay:          500 decoded / 0 errors / 20 raw-skip / 5 not-in-table / 2 unnamed / 4 conflicts / 1 effect blobs
  Checkpoint blobs: 8 decoded / 0 failed
  Checkpoint fails: 0 array / 0 truncated RPC / 0 movement
  Checkpoint array: 44652 elements / 364594 fields / 0 truncations / 0 root bits / 0 nested bits / 0 implicit ends
  Checkpoint leaf:  0 typed decode errors
  Checkpoint route children: 40 player info / 10 rewards / 30 selected / 0 kills / 99 active effects / 0 ignore actors / 0 blinds / 0 projectile path / 0 focus projectiles
  Checkpoint RPC walks: 0
  Checkpoint reward opaque: 7 empty variants
  Checkpoint movement: 0 failures
  Checkpoint movement components: 0 attempted / 0 complete / 0 errors / 0 empty
  Checkpoint component envelopes: 0 validated / 0 payload bytes / 0 unused bits
  Checkpoint component headers: 0 input / 0 discrepancy
  Checkpoint ActiveBlinds trailers: 1 empty deltas
  Checkpoint FocusProjectiles trailers: 0 empty deltas
  Checkpoint FocusProjectiles refused: 0 declaration mismatches
  Checkpoint CNC:   3 RPC rows
  Checkpoint CNC brute force: 0 attempted / 0 unwalked
"""


class ReadCountersTests(unittest.TestCase):
    def test_focus_counters_are_required_in_each_requested_pass(self):
        for prefix in ("", "Checkpoint "):
            for name in ("FocusProjectiles trailers:", "FocusProjectiles refused:"):
                with self.subTest(prefix=prefix, name=name):
                    label = prefix + name
                    text = "\n".join(line for line in CLEAN_WITH_CHECKPOINTS.splitlines()
                                     if not line.strip().startswith(label))
                    counters, error = guard.read_counters(text, 0, require_checkpoints=True)
                    self.assertIsNone(counters)
                    self.assertIn(label, error)
        text = CLEAN_WITH_CHECKPOINTS.replace(" / 0 focus projectiles", "", 1)
        counters, error = guard.read_counters(text, 0, require_checkpoints=True)
        self.assertIsNone(counters)
        self.assertIn("Route children:", error)

    def test_a_summary_reads_as_counters_and_the_checkpoint_block_only_when_asked(self):
        counters, err = guard.read_counters(LIVE_EXPORT, 0)
        self.assertEqual(err, "")
        self.assertEqual((counters["overlay_decoded_ok"], counters["overlay_no_field_name"],
                          counters["tracked_rewards_opaque_empty_variants"]), (742738, 1996, 4470))
        self.assertNotIn("cp_overlay_decoded_ok", counters)
        counters, err = guard.read_counters(CLEAN_WITH_CHECKPOINTS, 0, require_checkpoints=True)
        self.assertEqual(err, "")
        self.assertEqual((counters["cp_overlay_decoded_ok"],
                          counters["cp_overlay_handle_conflicts_refused"]), (500, 4))

    def test_a_nonzero_exit_is_not_a_clean_replay(self):
        """The summary prints before the Parquet files are finalised."""
        counters, err = guard.read_counters(LIVE_EXPORT, 1)
        self.assertIsNone(counters)
        self.assertIn("exit 1", err)

    def test_a_missing_line_makes_the_replay_unreadable_and_is_named(self):
        text = "\n".join(l for l in LIVE_EXPORT.splitlines() if "No field name" not in l)
        counters, err = guard.read_counters(text, 0)
        self.assertIsNone(counters)
        self.assertTrue(err.startswith("no 'No field name: {}' line"), err)
        counters, err = guard.read_counters(LIVE_EXPORT, 0, require_checkpoints=True)
        self.assertIsNone(counters)
        self.assertTrue(err.startswith("no 'Overlay: "), err)


#: Corpus totals from a working run: every work counter moved.
WORKING_TOTALS = {"overlay_decoded_ok": 129000, "struct_blobs_decoded": 63,
                  "array_elements_decoded": 25052, "array_fields_emitted": 96076,
                  "movement_rows": 2407298, "rpc_param_walks": 338107,
                  "cp_overlay_decoded_ok": 500,
                  "cp_struct_blobs_decoded": 8, "cp_array_elements_decoded": 44652,
                  "cp_array_fields_emitted": 364594}

#: `(work counter, the label its failure must start with, a gate it must name)`.
WORK_CASES = (
    (False, "overlay_decoded_ok", "Decoded OK", "Decode errors"),
    (False, "struct_blobs_decoded", "Struct blobs decoded", "Struct blobs failed"),
    (False, "array_elements_decoded", "Array decode elements", "Array residual root bits"),
    (False, "array_fields_emitted", "Array decode fields", "Array leaf errs"),
    (False, "movement_rows", "Movement rows", "Movement errors"),
    (False, "rpc_param_walks", "RPC param walks", "Truncated RPCs"),
    (True, "cp_overlay_decoded_ok", "Checkpoint Overlay decoded", "Checkpoint Overlay errors"),
    (True, "cp_struct_blobs_decoded", "Checkpoint blobs decoded", "Checkpoint blobs failed"),
    (True, "cp_array_elements_decoded", "Checkpoint array elements", "Checkpoint fails array"),
    (True, "cp_array_fields_emitted", "Checkpoint array fields",
     "Checkpoint leaf typed decode errors"),
)


class DeadCounterTests(unittest.TestCase):
    """Corpus totals that must have moved, in both passes."""

    def test_each_work_counter_at_zero_or_absent_is_named_alone_with_its_gates(self):
        for checkpoint, key, label, gate in WORK_CASES:
            must_move = guard.PASSES[checkpoint][1]
            self.assertEqual(guard.dead_counters(WORKING_TOTALS, must_move), [])
            for totals in (dict(WORKING_TOTALS, **{key: 0}),
                           {k: v for k, v in WORKING_TOTALS.items() if k != key}):
                with self.subTest(counter=key, absent=key not in totals):
                    dead = guard.dead_counters(totals, must_move)
                    self.assertEqual(len(dead), 1, dead)
                    self.assertTrue(dead[0].startswith(f"{label} totalled 0"), dead)
                    self.assertIn(gate, dead[0])

    def test_a_pass_that_decoded_nothing_fails_on_every_work_counter(self):
        for checkpoint in (False, True):
            must_move = guard.PASSES[checkpoint][1]
            self.assertEqual(len(guard.dead_counters({}, must_move)), len(must_move))


class ReconcileTests(unittest.TestCase):
    """The five printed overlay buckets must add up to `Rows offered`."""

    def test_the_live_export_reconciles(self):
        counters, err = guard.read_counters(LIVE_EXPORT, 0)
        self.assertEqual(err, "")
        self.assertIsNone(guard.reconcile(counters))

    def test_a_mismatch_is_reported_with_both_numbers(self):
        counters, _ = guard.read_counters(LIVE_EXPORT, 0)
        problem = guard.reconcile(dict(counters, overlay_no_field_name=0))
        self.assertIn("988,983", problem)
        self.assertIn("986,987", problem)  # the wrong sum without no_field_name

    def test_an_absent_bucket_is_a_loud_error_not_a_silent_zero(self):
        counters, _ = guard.read_counters(LIVE_EXPORT, 0)
        del counters["overlay_no_field_name"]
        with self.assertRaises(KeyError):
            guard.reconcile(counters)


class ArgParsingTests(unittest.TestCase):
    def test_every_flag_is_opt_in(self):
        plain = guard.parse_args(["vrfkit.exe", "corpus"])
        for flag in ("recursive", "checkpoints", "redact_identifiers"):
            with self.subTest(flag=flag):
                self.assertFalse(getattr(plain, flag))
                given = guard.parse_args(["vrfkit.exe", "corpus", "--" + flag.replace("_", "-")])
                self.assertTrue(getattr(given, flag))


class FailureTests(unittest.TestCase):
    def test_each_failure_counter_fails_its_replay_in_its_pass(self):
        clean, err = guard.read_counters(CLEAN_WITH_CHECKPOINTS, 0, require_checkpoints=True)
        self.assertEqual(err, "")
        self.assertEqual(guard.replay_failures(clean, checkpoints=True), [])
        for key in guard.FAILURES + guard.CHECKPOINT_FAILURES:
            with self.subTest(counter=key):
                self.assertEqual(guard.replay_failures(dict(clean, **{key: 7}), checkpoints=True),
                                 [(key, 7)])

    def test_a_handle_conflict_is_read_but_is_not_a_failure(self):
        """A conflict is the overlay refusing to type a renamed handle."""
        clean, _ = guard.read_counters(CLEAN_WITH_CHECKPOINTS, 0, require_checkpoints=True)
        counters = dict(clean, cp_overlay_handle_conflicts_refused=99)
        self.assertEqual(guard.replay_failures(counters, checkpoints=True), [])

    def test_an_absent_failure_counter_is_a_loud_error_not_a_silent_zero(self):
        counters, _ = guard.read_counters(LIVE_EXPORT, 0)
        del counters["array_leaf_decode_errors"]
        with self.assertRaises(KeyError):
            guard.replay_failures(counters, checkpoints=False)

    def test_the_gates_are_verify_build_corpus_sink_zero(self):
        """A counter added to its SINK_ZERO must be gated here too. Read, not
        imported, so this module needs nothing beyond the standard library."""
        tree = ast.parse((Path(guard.__file__).parent / "verify_build_corpus.py")
                         .read_text(encoding="utf-8"))
        sink_zero = next(ast.literal_eval(node.value) for node in tree.body
                         if isinstance(node, ast.Assign) and node.targets[0].id == "SINK_ZERO")
        self.assertEqual({"overlay_decoded_err" if k == "overlay_decode_errors" else k
                          for k in guard.FAILURES}, set(sink_zero))
        self.assertEqual(guard.CHECKPOINT_FAILURES, tuple("cp_" + k for k in guard.FAILURES))


class LivenessCoverageTests(unittest.TestCase):
    """Every failure gate is backed by a work counter that must move, or is
    declared unbacked with a reason -- never neither."""

    def test_every_gate_is_backed_or_declared_unbacked_exactly_once(self):
        for checkpoint, (failures, must_move, unbacked) in guard.PASSES.items():
            with self.subTest(checkpoint=checkpoint):
                claimed = [gate for _key, gates in must_move for gate in gates]
                claimed += [key for key, why in unbacked if why.strip()]
                self.assertEqual(sorted(claimed), sorted(failures))

    def test_every_work_counter_is_read_and_is_not_itself_a_failure(self):
        for checkpoint, (failures, must_move, _unbacked) in guard.PASSES.items():
            for key, gates in must_move:
                with self.subTest(work=key):
                    self.assertIn(key, guard.sc.keys("G", checkpoint))
                    self.assertNotIn(key, failures)
                    self.assertTrue(gates)


#: Stand-in for `vrfkit.exe` as `_export_one` invokes it: a file named `export`
#: run under `sys.executable`. Its output depends on the replay's filename.
FAKE_EXPORT_SCRIPT = '''\
import sys
from pathlib import Path

argv = sys.argv
out = Path(argv[argv.index("--out") + 1])
out.mkdir(parents=True, exist_ok=True)
name = Path(argv[1]).name

SINK = """
Movement rows:     100
Movement errors:   0
Array decode:      10 elements / 40 fields / 0 errors / 0 truncations
Array residual:    0 root bits / 0 nested bits / 0 implicit ends
Array leaf errs:   0
Route children:    1 player info / 1 rewards / 1 selected / 1 kills / 1 active effects / 1 ignore actors / 1 blinds / 1 projectile path / 0 focus projectiles
Truncated RPCs:    0
RPC param walks:   20
CNC brute force:   4 attempted / 0 unwalked
Movement components: 5 attempted / 5 complete / 0 errors / 0 empty
Component envelopes: 5 validated / 150 payload bytes / 7 unused bits
Component headers: 5 input / 4 discrepancy
ActiveBlinds trailers: 2 empty deltas
FocusProjectiles trailers: 0 empty deltas
FocusProjectiles refused: 0 declaration mismatches
"""

# Printed only when the gate passed --checkpoints on, as vrfkit does.
CHECKPOINTS = """
=== Checkpoints ===
  Overlay:          500 decoded / 0 errors / 20 raw-skip / 5 not-in-table / 2 unnamed / 4 conflicts / 1 effect blobs
  Checkpoint blobs: 8 decoded / 0 failed
  Checkpoint fails: 0 array / 0 truncated RPC / 0 movement
  Checkpoint array: 30 elements / 90 fields / 0 truncations / 0 root bits / 0 nested bits / 0 implicit ends
  Checkpoint leaf:  0 typed decode errors
  Checkpoint route children: 0 player info / 0 rewards / 0 selected / 0 kills / 0 active effects / 0 ignore actors / 0 blinds / 0 projectile path / 0 focus projectiles
  Checkpoint RPC walks: 0
  Checkpoint reward opaque: 0 empty variants
  Checkpoint movement components: 0 attempted / 0 complete / 0 errors / 0 empty
  Checkpoint component envelopes: 0 validated / 0 payload bytes / 0 unused bits
  Checkpoint component headers: 0 input / 0 discrepancy
  Checkpoint ActiveBlinds trailers: 1 empty deltas
  Checkpoint FocusProjectiles trailers: 0 empty deltas
  Checkpoint FocusProjectiles refused: 0 declaration mismatches
  Checkpoint CNC brute force: 0 attempted / 0 unwalked
"""

CLEAN = """
Rows offered:      100
Decoded OK:        90
Decode errors:     0
Raw/Skip:          5
Not in table:      3
No field name:     2
Struct blobs:      5 decoded / 0 failed
Reward opaque:     0 empty variants
"""


def emit(summary=CLEAN, sink=SINK, checkpoints=CHECKPOINTS):
    print(summary + sink + (checkpoints if "--checkpoints" in argv else ""))
    raise SystemExit(0)


if "badexit" in name:
    print("exporter crashed", file=sys.stderr)
    raise SystemExit(9)
if "nothingran" in name:
    # Every counter a legitimate zero.
    emit(re.sub(r"\\d+", "0", CLEAN), SINK.replace("10 elements / 40 fields", "0 elements / 0 fields"))
if "decodeerr" in name:
    emit(CLEAN.replace("Decoded OK:        90", "Decoded OK:        80")
         .replace("Decode errors:     0", "Decode errors:     10"))
if "missingcounter" in name:
    emit(CLEAN.replace("No field name:     2", ""))
if "mismatch" in name:
    # Every line present, but the five buckets undercount Rows offered by one.
    emit(CLEAN.replace("No field name:     2", "No field name:     1"))
if "cpleaf" in name:
    emit(checkpoints=CHECKPOINTS.replace("Checkpoint leaf:  0", "Checkpoint leaf:  4"))
# The array walker never reached; every other counter is what a good run
# prints. The checkpoint name is tested first because it contains the other.
if "cpnoarray" in name:
    emit(checkpoints=CHECKPOINTS.replace("30 elements / 90 fields", "0 elements / 0 fields"))
if "noarray" in name:
    emit(sink=SINK.replace("10 elements / 40 fields", "0 elements / 0 fields"))
if "nowalk" in name:
    emit(sink=SINK.replace("RPC param walks:   20", "RPC param walks:   0"))
emit()
'''.replace("import sys\n", "import re\nimport sys\n", 1)


class MainWiringTests(TempDirTestCase):
    def setUp(self):
        self.root = self.tmp()
        (self.root / "export").write_text(FAKE_EXPORT_SCRIPT, encoding="utf-8")
        self.corpus = self.root / "corpus"
        self.corpus.mkdir()
        self.enterContext(contextlib.chdir(self.root))

    def run_main(self, *names, extra_args=()):
        for name in names:
            (self.corpus / name).write_bytes(b"not a real replay")
        code, out, _ = run_cli(guard.main, sys.executable, self.corpus, "--jobs", "1", *extra_args,
                               prog="check_decode_errors_corpus.py", merged=True)
        return code, out

    def test_a_clean_checkpoint_run_prints_every_total_and_what_backs_its_zeros(self):
        """Every total prints, zeros included, and the OK line separates the
        gates work backs from the ones nothing does."""
        code, output = self.run_main("a.vrf", extra_args=["--checkpoints"])
        self.assertEqual(code, 0, output)
        for line in (
            "Decode errors: 0", "Movement rows: 100", "Movement errors: 0",
            "Array decode: 10 elements / 40 fields / 0 errors / 0 truncations",
            "Array residual: 0 root bits / 0 nested bits / 0 implicit ends",
            "Array leaf errs: 0", "Truncated RPCs: 0", "RPC param walks: 20",
            "Route children: 1 player info / 1 rewards / 1 selected / 1 kills / "
            "1 active effects / 1 ignore actors / 1 blinds / 1 projectile path / 0 focus projectiles",
            "CNC brute force: 4 attempted / 0 unwalked",
            "Movement components: 5 attempted / 5 complete / 0 errors / 0 empty",
            "Component envelopes: 5 validated / 150 payload bytes / 7 unused bits",
            "Component headers: 5 input / 4 discrepancy", "ActiveBlinds trailers: 2 empty deltas",
            "Overlay: 500 decoded / 0 errors / 20 raw-skip / 5 not-in-table / 2 unnamed / "
            "4 conflicts / 1 effect blobs",
            "Checkpoint array: 30 elements / 90 fields / 0 truncations / "
            "0 root bits / 0 nested bits / 0 implicit ends",
            "Checkpoint leaf: 0 typed decode errors",
            "Checkpoint CNC brute force: 0 attempted / 0 unwalked",
            "Checkpoint movement components: 0 attempted / 0 complete / 0 errors / 0 empty",
            "Checkpoint component envelopes: 0 validated / 0 payload bytes / 0 unused bits",
            "Checkpoint component headers: 0 input / 0 discrepancy",
            "Checkpoint ActiveBlinds trailers: 1 empty deltas",
        ):
            with self.subTest(line=line):
                self.assertRegex(output, rf"(?m)^  {re.escape(line)}$")
        self.assertRegex(
            output, r"(?m)^  unbacked gates: CNC brute force unwalked \(.+\)$")
        self.assertRegex(
            output, r"(?m)^  checkpoint unbacked gates: Checkpoint fails movement \(.+\); "
                    r"Checkpoint fails truncated RPC \(.+\); Checkpoint movement components errors "
                    r"\(.+\); Checkpoint CNC brute force unwalked \(.+\)$")
        ok = [line for line in output.splitlines() if line.startswith("OK:")]
        self.assertEqual(len(ok), 1, output)
        self.assertIn("11 backed by work that moved (Decoded OK 90, Struct blobs decoded 5, "
                      "Array decode elements 10, Array decode fields 40, Movement rows 100, "
                      "RPC param walks 20), 1 with no work counter (CNC brute force unwalked)", ok[0])
        self.assertIn("8 backed by work that moved (Checkpoint Overlay decoded 500, "
                      "Checkpoint blobs decoded 8, Checkpoint array elements 30, Checkpoint "
                      "array fields 90), 4 with no work counter (Checkpoint fails movement, "
                      "Checkpoint fails truncated RPC, Checkpoint movement components errors, "
                      "Checkpoint CNC brute force unwalked)",
                      ok[0])

    def test_a_failure_counter_fails_the_run_in_each_pass(self):
        """Asserted on the failure block's line: the totals print `Decode
        errors` on every run."""
        for name, extra, line in (
                ("decodeerr.vrf", (), "decodeerr.vrf: Decode errors=10"),
                ("cpleaf.vrf", ("--checkpoints",),
                 "cpleaf.vrf: Checkpoint leaf typed decode errors=4")):
            with self.subTest(name):
                code, output = self.run_main(name, extra_args=extra)
                (self.corpus / name).unlink()
                self.assertEqual(code, 1, output)
                self.assertIn(line, output)
                self.assertIn("Struct blob err:", output)
                self.assertNotRegex(output, "(?m)^OK:")

    def test_a_work_counter_that_never_moved_fails_the_run_in_each_pass(self):
        for name, extra, dead in (
                ("nothingran.vrf", (), "Decoded OK totalled 0"),
                ("noarray.vrf", (), "Array decode elements totalled 0"),
                ("nowalk.vrf", (), "RPC param walks totalled 0"),
                ("cpnoarray.vrf", ("--checkpoints",), "Checkpoint array elements totalled 0")):
            with self.subTest(name):
                code, output = self.run_main(name, extra_args=extra)
                (self.corpus / name).unlink()
                self.assertEqual(code, 1, output)
                self.assertIn(dead, output)
                self.assertIn("never moved", output)

    def test_one_replay_that_did_the_work_keeps_the_corpus_alive(self):
        """Corpus totals: a replay whose walker found no array beside one
        whose walker did is not a dead counter."""
        code, output = self.run_main("a.vrf", "noarray.vrf")
        self.assertEqual(code, 0, output)

    def test_an_unreadable_or_unreconciled_replay_fails_the_run(self):
        for name, expected in (("missingcounter.vrf", "did not report the counter"),
                               ("badexit.vrf", "did not report the counter"),
                               ("mismatch.vrf", "do not reconcile")):
            with self.subTest(name):
                code, output = self.run_main(name)
                (self.corpus / name).unlink()
                self.assertEqual(code, 1, output)
                self.assertIn(expected, output)

    def test_no_vrf_files_is_a_controlled_failure(self):
        code, output = self.run_main()
        self.assertEqual(code, 2, output)
