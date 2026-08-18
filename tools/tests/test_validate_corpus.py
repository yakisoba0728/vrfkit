"""Guards for the corpus oracle sweep.

The accumulator carries a comment saying "A counter the oracle stopped printing
must not read as zero. That is precisely how the malformed figure stayed a
vacuous 0 for the whole corpus while its pattern was wrong" -- and then the
absent counter was printed as a WARNING and the run exited 0 anyway. Writing
the argument down is not the same as acting on it.
"""
import collections
import contextlib
import io
import os
import sys
import tempfile
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import validate_corpus as guard  # noqa: E402


class ProblemTests(unittest.TestCase):
    def test_a_clean_sweep_has_no_problems(self):
        self.assertEqual(guard.problems([], collections.Counter()), [])

    def test_a_replay_the_oracle_could_not_validate_is_a_problem(self):
        found = guard.problems([("a.vrf", "exit 101")], collections.Counter())
        self.assertTrue(found)
        self.assertIn("a.vrf", " ".join(found))

    def test_a_counter_the_oracle_stopped_printing_is_a_problem(self):
        """The defect: this was a WARNING beside an exit 0."""
        found = guard.problems([], collections.Counter({"malformed": 3}))
        self.assertTrue(found)
        joined = " ".join(found)
        self.assertIn("malformed", joined)
        self.assertIn("3", joined)

    def test_every_absent_counter_is_named_not_just_the_first(self):
        found = guard.problems(
            [], collections.Counter({"malformed": 3, "skipped": 1}))
        self.assertEqual(len(found), 2, found)

    def test_failures_and_absent_counters_are_both_reported(self):
        found = guard.problems([("a.vrf", "timeout")],
                               collections.Counter({"malformed": 1}))
        self.assertEqual(len(found), 2, found)


class PatternTests(unittest.TestCase):
    """The regexes are shared with check_corpus_baseline.py so they cannot drift."""

    def test_the_malformed_pattern_matches_the_label_the_oracle_prints(self):
        m = guard.PATTERNS["malformed"].search("Malformed framing:  0")
        self.assertIsNotNone(m)
        self.assertEqual(m.group(1), "0")

    def test_every_accumulated_counter_has_a_pattern(self):
        for key in ("blocks", "malformed", "skipped", "fields", "rpcs"):
            self.assertIn(key, guard.PATTERNS)

    def test_missing_branch_is_a_controlled_parse_failure(self):
        text = """
Total content blocks: 10
Fields emitted: 20
RPCs emitted: 5
Malformed framing: 0
Skipped bits: 0
ORACLE PASS RATE: 100.000000%
"""
        parsed, error = guard.parse_oracle_output(text)
        self.assertIsNone(parsed)
        self.assertIn("Branch", error)


class ArgParsingTests(unittest.TestCase):
    """Defect 1 wiring: discovery now goes through corpus_scan.py, and the
    recursion choice is an explicit, opt-in flag rather than a hardcoded glob.
    """

    def test_recursive_defaults_to_false(self):
        args = guard.parse_args(["validate_corpus.py", "vrfkit.exe", "corpus"])
        self.assertFalse(args.recursive)

    def test_recursive_flag_is_readable(self):
        args = guard.parse_args(
            ["validate_corpus.py", "vrfkit.exe", "corpus", "--recursive"])
        self.assertTrue(args.recursive)

    def test_the_optional_limit_still_parses_positionally(self):
        """Backward compatibility: `<exe> <corpus> [limit]` must keep working."""
        args = guard.parse_args(["validate_corpus.py", "vrfkit.exe", "corpus", "5"])
        self.assertEqual(args.limit, 5)

    def test_limit_is_optional(self):
        args = guard.parse_args(["validate_corpus.py", "vrfkit.exe", "corpus"])
        self.assertIsNone(args.limit)


#: Stand-in for `vrfkit.exe`, invoked exactly as `_run_one` invokes the real
#: one -- `[str(exe), "validate", str(path)]`. Run under `sys.executable`, the
#: first argv token becomes the script Python executes (the same trick
#: `test_check_export_baseline.py`'s `TransactionalOutputTests` uses for its
#: fake `export`), so a file literally named `validate`, with no extension, in
#: the process's cwd stands in for the real binary. What it prints depends on
#: the replay's own filename, so one script can play every scenario below.
FAKE_VALIDATE_SCRIPT = '''\
import sys
from pathlib import Path

name = Path(sys.argv[1]).name

if "badexit" in name:
    print("oracle blew up", file=sys.stderr)
    raise SystemExit(3)

print("Branch: ++Ares-Core+release-13.01")
print("Total content blocks: 100")
if "missingmalformed" not in name:
    print("Malformed framing:  0")
print("Skipped bits:  0")
print("Fields emitted: 50")
print("RPCs emitted: 10")
print("ORACLE PASS RATE: 100.000000%")
'''


class MainWiringTests(unittest.TestCase):
    """`ProblemTests` above pins what `problems()` returns; nothing pinned
    that `main()` actually reads it before choosing an exit code. That is
    precisely the layer where the recorded defect lived: the absent-counter
    case was computed, printed as a WARNING, and the process exited 0 anyway.
    A helper that is provably correct in isolation says nothing about the
    `if found: return 1` a few lines later in `main()` -- these tests are
    that line.
    """

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        (self.root / "validate").write_text(FAKE_VALIDATE_SCRIPT, encoding="utf-8")
        self.corpus = self.root / "corpus"
        self.corpus.mkdir()
        self._previous_cwd = Path.cwd()
        os.chdir(self.root)
        self.addCleanup(os.chdir, self._previous_cwd)

    def make_replay(self, name: str) -> None:
        (self.corpus / name).write_bytes(b"not a real replay")

    def run_main(self, limit: str | None = None):
        argv = ["validate_corpus.py", sys.executable, str(self.corpus)]
        if limit is not None:
            argv.append(limit)
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            code = guard.main(argv)
        return code, out.getvalue()

    def test_a_clean_sweep_exits_zero(self):
        self.make_replay("a.vrf")
        self.make_replay("b.vrf")
        code, output = self.run_main()
        self.assertEqual(code, 0, output)
        self.assertIn("OK:", output)

    def test_an_oracle_that_could_not_validate_a_replay_fails_the_run(self):
        self.make_replay("badexit.vrf")
        code, output = self.run_main()
        self.assertNotEqual(code, 0, output)
        self.assertIn("FAILED", output)

    def test_a_counter_the_oracle_stopped_printing_fails_the_run(self):
        """The recorded defect, reproduced end to end: `missingmalformed.vrf`
        exits 0 from the fake oracle and every OTHER counter is present, so
        the only thing that can catch it is `main()` reading `problems()`'s
        report on the absent `malformed` counter -- not a helper being
        correct, but `main()` acting on what the helper says."""
        self.make_replay("missingmalformed.vrf")
        code, output = self.run_main()
        self.assertNotEqual(code, 0, output)
        self.assertIn("FAILED", output)
        self.assertIn("malformed", output)

    def test_an_empty_corpus_is_a_controlled_failure_not_a_silent_pass(self):
        with contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaises(SystemExit) as caught:
                guard.main(["validate_corpus.py", sys.executable, str(self.corpus)])
        self.assertIn("no .vrf under", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
