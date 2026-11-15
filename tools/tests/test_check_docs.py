"""Guards for the doc guard: nothing else catches a wrong number in prose, so
its detection logic must not rot into something that passes everything."""
import ast
import inspect
import json
import re
import textwrap
import unittest
from unittest.mock import patch
from subprocess import CompletedProcess

from support import run_cli
import check_docs as guard


USAGE_MENTIONING_EVERYTHING = " ".join(
    p.name for p in (guard.REPO / "tools").glob("*.py")
)


class ToolCoverageTests(unittest.TestCase):
    def test_an_undocumented_tool_is_reported(self):
        problems = guard.check_tools("only_one.py is documented")
        self.assertTrue(problems)
        self.assertTrue(any("never mentions it" in p for p in problems))

    def test_documenting_a_tool_that_does_not_exist_is_reported(self):
        text = USAGE_MENTIONING_EVERYTHING + " imaginary_tool.py"
        problems = guard.check_tools(text)
        self.assertEqual(
            [p for p in problems if "imaginary_tool" in p and "does not exist" in p],
            [p for p in problems if "imaginary_tool" in p],
        )
        self.assertTrue(any("imaginary_tool.py" in p for p in problems))

    def test_documenting_every_tool_is_clean(self):
        self.assertEqual(guard.check_tools(USAGE_MENTIONING_EVERYTHING), [])

    def test_the_external_script_is_not_demanded_to_exist(self):
        """compute_metrics.py lives in valplay, which is never modified here."""
        text = USAGE_MENTIONING_EVERYTHING + " compute_metrics.py"
        self.assertEqual(
            [p for p in guard.check_tools(text) if "compute_metrics" in p], []
        )

    def test_test_files_are_not_demanded_in_the_reference(self):
        problems = guard.check_tools(USAGE_MENTIONING_EVERYTHING + " test_thing.py")
        self.assertFalse(any("test_thing" in p for p in problems))


class CrateCoverageTests(unittest.TestCase):
    def test_a_missing_crate_row_is_reported(self):
        problems = guard.check_crates("`vrf-bitio` only")
        self.assertTrue(problems)
        self.assertTrue(any("vrfkit" in p for p in problems))


class LinkTests(unittest.TestCase):
    def test_a_dead_relative_link_is_reported(self):
        problems = guard.check_links(guard.USAGE, "[x](no_such_file.md)")
        self.assertTrue(any("no_such_file.md" in p for p in problems))

    def test_external_and_anchor_links_are_skipped(self):
        text = "[a](https://example.com) [b](#section)"
        self.assertEqual(guard.check_links(guard.USAGE, text), [])

    def test_a_link_with_an_anchor_checks_only_the_file(self):
        text = "[a](../README.md#어쩌고)"
        self.assertEqual(guard.check_links(guard.USAGE, text), [])


class AnchorTests(unittest.TestCase):
    """`check_links` checks only that a linked file exists; these check that a
    linked or cited heading exists by GitHub's slug rules."""

    TARGET = guard.REPO / "docs" / "TARGET.md"
    SOURCE = guard.REPO / "docs" / "SOURCE.md"

    def lookup(self, text):
        anchors = frozenset(guard.heading_anchors(text))
        return lambda path: anchors if path == self.TARGET.resolve() else None

    def test_slugs_follow_githubs_rules(self):
        for heading, slug in (
                ("Regression guards -- after non-trivial changes",
                 "regression-guards----after-non-trivial-changes"),
                ("`fields.parquet`", "fieldsparquet"),
                ("Notes on F# and C++ readers",
                 "notes-on-f-and-c-readers"),
                ("`raw_bits`: SmallVec, and the rejected arena",
                 "raw_bits-smallvec-and-the-rejected-arena"),
                ("Generated files — never hand-edit", "generated-files--never-hand-edit"),
                ("5. `tools/` reference", "5-tools-reference"),
                ("See [the table](other.md#x) here", "see-the-table-here"),
                ("참조 저장소 및 PR 조사", "참조-저장소-및-pr-조사")):
            with self.subTest(heading=heading):
                self.assertEqual(guard.github_slug(heading), slug)

    def test_a_repeated_heading_gets_a_numbered_anchor(self):
        self.assertEqual(guard.heading_anchors("# A\n## A\n### A b\n#### A"),
                         {"a", "a-1", "a-b", "a-2"})

    def test_a_hash_line_in_fenced_code_is_not_a_heading(self):
        text = ("```bash\n# not a heading\n```python\n# still code\n```\n"
                "~~~\n## nor this\n~~~\n## Real ##\n#hashtag\n")
        self.assertEqual(guard.heading_anchors(text), {"real"})

    def test_a_broken_same_file_anchor_is_reported(self):
        text = "## Downstream conversion tools\n\n[x](#downstream-conversion)\n"
        problems = guard.broken_markdown_anchors(
            self.SOURCE, text, lambda path: frozenset(guard.heading_anchors(text)))
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("SOURCE.md:3", problems[0])
        self.assertIn("#downstream-conversion", problems[0])
        fixed = text.replace("(#downstream-conversion)", "(#downstream-conversion-tools)")
        self.assertEqual(guard.broken_markdown_anchors(
            self.SOURCE, fixed, lambda path: frozenset(guard.heading_anchors(fixed))), [])

    def test_a_broken_cross_file_anchor_is_reported(self):
        lookup = self.lookup("# Title\n## Name interning\n")
        good = "[a](TARGET.md#name-interning)"
        bad = "[a](TARGET.md#name-intern)"
        self.assertEqual(guard.broken_markdown_anchors(self.SOURCE, good, lookup), [])
        problems = guard.broken_markdown_anchors(self.SOURCE, bad, lookup)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("#name-intern", problems[0])

    def test_a_percent_encoded_anchor_is_decoded(self):
        lookup = self.lookup("## 어쩌고\n")
        for link in ("[a](TARGET.md#%EC%96%B4%EC%A9%8C%EA%B3%A0)", "[a](TARGET.md#어쩌고)"):
            with self.subTest(link=link):
                self.assertEqual(guard.broken_markdown_anchors(self.SOURCE, link, lookup), [])

    def test_links_in_fenced_code_and_line_anchors_are_not_heading_links(self):
        lookup = self.lookup("## Only\n")
        text = ("```\n[a](TARGET.md#nothing)\n```\n"
                "[b](../tools/check_docs.py#L10)\n[c](https://example.com/x.md#y)\n")
        checked = []
        self.assertEqual(
            guard.broken_markdown_anchors(self.SOURCE, text, lookup, checked), [])
        self.assertEqual(checked, [])

    def test_a_code_reference_to_a_missing_heading_or_doc_is_reported(self):
        # Two literals, so the repository scan does not read this file as
        # citing the headings it tests.
        lookup = self.lookup("## Name interning\n")
        target = "docs/TARGET" ".md"
        text = (f"//! {target}#name-interning.\n"
                f"// ({target}#name-intern)\n"
                f"# see docs/GONE" ".md#anything\n"
                f"// {target} with no anchor, then docs/GONE" ".md with none\n")
        checked = []
        problems = guard.broken_code_anchors("x.rs", text, lookup, checked)
        self.assertEqual(len(checked), 5, checked)
        self.assertEqual(len(problems), 3, problems)
        self.assertIn("x.rs:2", problems[0])
        self.assertIn("#name-intern", problems[0])
        self.assertIn("does not exist", problems[1])
        self.assertIn("x.rs:4", problems[2])
        self.assertIn("does not exist", problems[2])

    def test_a_scan_that_read_nothing_is_reported_not_passed(self):
        import subprocess as sp
        empty = sp.CompletedProcess([], 0, stdout="", stderr="")
        with patch.object(guard, "link_checked_docs", return_value=[]), \
                patch.object(guard.subprocess, "run", return_value=empty):
            problems = guard.anchor_problems()
        self.assertEqual(len(problems), 2, problems)
        self.assertTrue(all("read no" in p for p in problems), problems)

    def test_an_unlistable_source_tree_is_reported_not_passed(self):
        import subprocess as sp
        real = guard.subprocess.run

        def failing(cmd, *a, **kw):
            if cmd[:2] == ["git", "-C"]:
                return sp.CompletedProcess(cmd, 128, stdout="", stderr="fatal")
            return real(cmd, *a, **kw)

        with patch.object(guard.subprocess, "run", side_effect=failing):
            problems = guard.anchor_problems()
        self.assertTrue(any("went unchecked" in p for p in problems), problems)


class TableSizeTests(unittest.TestCase):
    def test_a_stale_table_size_is_reported(self):
        docs = {"README.md": "the table has 999 entries",
                "USAGE.md": "the table has 999 entries"}
        problems = guard.check_table_sizes(docs)
        self.assertTrue(problems)

    def test_both_comma_and_plain_forms_are_accepted(self):
        table = guard.read(guard.REPO / "crates" / "vrf-decode" / "src" / "table.rs")
        import re
        n = re.search(r"OVERLAY_TABLE: \[OverlayEntry; (\d+)\]", table).group(1)
        h = re.search(r"OVERLAY_HANDLE_TABLE: \[OverlayHandleEntry; (\d+)\]",
                      table).group(1)
        plain = {"README.md": n, "USAGE.md": f"{n} {h}"}
        comma = {"README.md": f"{int(n):,}", "USAGE.md": f"{int(n):,} {h}"}
        self.assertEqual(guard.check_table_sizes(plain), [])
        self.assertEqual(guard.check_table_sizes(comma), [])


class SourceTableSizeTests(unittest.TestCase):
    """Rust prose and Cargo.toml quote the table size too."""

    LIVE = {"1188", "1,188"}

    def test_a_stale_size_is_reported_with_its_line(self):
        text = "one\n// the 1,185-entry generated table\nthree"
        self.assertEqual(guard.stale_entry_phrases(text, self.LIVE), [(2, "1,185")])

    def test_both_comma_and_plain_forms_are_accepted(self):
        for spelling in ("1,188-entry table", "1188-entry table"):
            self.assertEqual(guard.stale_entry_phrases(spelling, self.LIVE), [])

    def test_the_optional_generated_word_is_matched_either_way(self):
        """`N-entry table` and `N-entry generated table` are both in the tree."""
        for phrase in ("999-entry table", "999-entry generated table"):
            self.assertEqual(guard.stale_entry_phrases(phrase, self.LIVE),
                             [(1, "999")], phrase)

    def test_a_bare_entry_count_is_not_a_size_claim(self):
        """Dated measurements legitimately say "1,054 entries" about a table
        that no longer exists; only "N-entry table" claims the live size."""
        self.assertEqual(
            guard.stale_entry_phrases("measured over 1,054 entries", self.LIVE), [])


class SuiteMeasurementTests(unittest.TestCase):
    RUST = "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
    PYTHON = "Ran 4 tests in 0.01s\n\nOK\n"
    MODULES = ["test_a.py", "test_b.py"]

    def measure(self, rust=None, python=None, rust_exit=0, python_exit=0):
        def run(cmd, *a, **kw):
            if cmd[0] == "cargo":
                return CompletedProcess(cmd, rust_exit, stdout=self.RUST if rust is None else rust,
                                        stderr="")
            return CompletedProcess(cmd, python_exit, stdout="",
                                    stderr=self.PYTHON if python is None else python)
        with patch.object(guard.subprocess, "run", side_effect=run) as mock:
            result = guard.measure_tests(self.MODULES)
        return result, [c.args[0] for c in mock.call_args_list]

    def test_measures_rust_and_every_module_with_warnings_as_errors(self):
        result, cmds = self.measure(rust=self.RUST * 2)
        self.assertEqual(result, (6, 8, []))
        python = [c for c in cmds if c[0] != "cargo"]
        self.assertEqual(sorted(c[-1] for c in python), self.MODULES)
        # `-b` is a `discover` option: before it, unittest rejects -s/-p (exit 2).
        self.assertTrue(all(c[1:3] == ["-W", "error"] and "-b" in c
                            and c.index("-b") > c.index("discover") for c in python), python)

    def test_empty_output_and_zero_tests_are_not_successful_measurements(self):
        for label, output in (("rust", ""), ("python", ""),
                              ("rust", self.RUST.replace("3 passed", "0 passed")),
                              ("python", self.PYTHON.replace("4 tests", "0 tests"))):
            with self.subTest(label=label, output=output):
                self.assertTrue(self.measure(**{label: output})[0][2])

    def test_passing_text_cannot_hide_a_failed_process(self):
        for kwargs in ({"rust_exit": 1}, {"python_exit": 1}):
            with self.subTest(kwargs=kwargs):
                self.assertTrue(self.measure(**kwargs)[0][2])

    def test_skipped_python_tests_are_not_reported_as_all_passing(self):
        self.assertTrue(self.measure(python=self.PYTHON.replace("OK", "OK (skipped=1)"))[0][2])

    def test_unrelated_passing_text_is_not_a_rust_suite_summary(self):
        self.assertTrue(self.measure(rust="an example says 999 passed\n")[0][2])


class TestCountTests(unittest.TestCase):
    """A doc may not carry a stale suite size beside a live one."""

    LIVE = {"387", "120"}

    def test_a_stale_count_is_reported_with_its_line(self):
        text = "one\ncargo test --workspace **355 passing**\nthree"
        self.assertEqual(guard.stale_test_counts(text, self.LIVE), [(2, "355")])

    def test_a_live_count_elsewhere_does_not_excuse_a_stale_one(self):
        text = "- **387 tests** plus a validation suite\nverified: **355 passing**"
        self.assertEqual(guard.stale_test_counts(text, self.LIVE), [(2, "355")])

    def test_either_suite_count_is_accepted(self):
        self.assertEqual(guard.stale_test_counts("387 passing\n120 tests", self.LIVE), [])

    def test_a_number_that_is_not_a_count_claim_is_ignored(self):
        """Only "N tests" / "N passing" is a claim about the suites; README's
        "2,387 intermediate moves" is not."""
        self.assertEqual(
            guard.stale_test_counts("we recover 2,387 intermediate moves", self.LIVE), [])

    def test_a_count_that_names_its_suite_is_read(self):
        """README's highlight puts the suite between the number and the noun."""
        text = "- **355 Rust tests** plus a layered validation suite"
        self.assertEqual(guard.stale_test_counts(text, self.LIVE), [(1, "355")])
        self.assertEqual(guard.stale_test_counts("**387 Rust tests**", self.LIVE), [])

    def test_a_count_that_names_its_suite_must_be_that_suites(self):
        by_suite = {"Rust": {"387"}, "Python": {"120"}}
        self.assertEqual(guard.stale_test_counts(
            "**120 Rust tests**\n**387 Python tests**\n387 passing\n120 tests",
            self.LIVE, by_suite), [(1, "120"), (2, "387")])

    def test_a_count_is_quoted_only_by_a_count_claim_in_either_spelling(self):
        counts = {"tools": 1233}
        self.assertEqual(guard.unquoted_test_counts({"a.md": "**1,233 passing**"}, counts), [])
        self.assertEqual(guard.unquoted_test_counts({"a.md": "1233 Python tests"}, counts), [])
        # A number that merely contains the count, or is not a count claim, quotes nothing.
        for text in ("a byte count of 91,233,120", "1233 rows"):
            with self.subTest(text=text):
                self.assertEqual(len(guard.unquoted_test_counts({"a.md": text}, counts)), 1)

    def test_the_shipped_readme_highlight_is_read(self):
        claims = [suite for line in guard.read(guard.README).splitlines()
                  for _count, suite in guard.TEST_COUNT_RE.findall(line)]
        self.assertIn("Rust", claims)


class PlatformTestCountTests(unittest.TestCase):
    DOCS = {
        "README.md": "387 Rust tests (Windows)\n386 Rust tests (non-Windows)\n120 Python tests",
        "USAGE.md": "387 passing (Windows)\n386 passing (non-Windows)\n120 passing",
    }

    def test_each_platform_checks_its_own_live_suite_counts(self):
        for platform, rust in (("Windows", 387), ("non-Windows", 386)):
            with self.subTest(platform=platform):
                by_suite = {"Rust": {str(rust)}, "Python": {"120"}}
                live = by_suite["Rust"] | by_suite["Python"]
                for text in self.DOCS.values():
                    self.assertEqual(guard.stale_test_counts(
                        text, live, by_suite, platform=platform), [])
                self.assertEqual(guard.unquoted_test_counts(
                    self.DOCS, {"rust": rust, "tools": 120}, platform=platform), [])

    def test_a_stale_count_on_the_selected_platform_is_not_excused(self):
        text = "387 Rust tests (Windows)\n385 Rust tests (non-Windows)\n120 Python tests"
        self.assertEqual(guard.stale_test_counts(
            text, {"386", "120"}, platform="non-Windows"), [(2, "385")])
        self.assertEqual(guard.stale_test_counts(
            text, {"387", "120"}, platform="Windows"), [])

    def test_an_unqualified_count_still_applies_on_every_platform(self):
        self.assertEqual(guard.stale_test_counts(
            "385 Rust tests\n386 Rust tests (non-Windows)",
            {"386", "120"}, platform="non-Windows"), [(1, "385")])

    def test_another_platforms_count_cannot_satisfy_a_missing_local_claim(self):
        self.assertEqual(len(guard.unquoted_test_counts(
            {"a.md": "387 Rust tests (Windows)\n120 Python tests"},
            {"rust": 386, "tools": 120}, platform="non-Windows")), 1)

    def test_two_platform_specific_rust_counts_are_not_a_contradiction(self):
        self.assertEqual(guard.contradicting_test_counts(self.DOCS), [])

    def test_a_third_count_within_one_platform_is_a_contradiction(self):
        docs = dict(self.DOCS)
        docs["README.md"] += "\n385 Rust tests (non-Windows)"
        self.assertTrue(guard.contradicting_test_counts(docs))


class TableSizeClaimTests(unittest.TestCase):
    """Every number that claims to BE a table size must be the live one, even
    one line from the correct one."""

    LENGTHS = ("1255", "84")

    def test_a_stale_entry_count_beside_the_live_one_is_caught(self):
        docs = {"README.md": "the overlay table (1,255 entries, 84 handles)\n"
                             "elsewhere: 1,185 entries"}
        problems = guard.stale_table_size_claims(docs, self.LENGTHS)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("1,185", problems[0])
        self.assertIn("README.md:2", problems[0])

    def test_a_stale_handle_count_is_caught(self):
        problems = guard.stale_table_size_claims(
            {"USAGE.md": "overlay table 1,255 + 63 handles"}, self.LENGTHS)
        self.assertTrue(any("63" in p for p in problems), problems)

    def test_both_spellings_of_the_live_numbers_pass(self):
        docs = {"a.md": "1,255 entries and 84 handles",
                "b.md": "1255 entries and 84 handles"}
        self.assertEqual(guard.stale_table_size_claims(docs, self.LENGTHS), [])

    def test_punctuation_before_word_handles_is_not_a_size_claim(self):
        docs = {"USAGE.md": "identifiers, handles, checksums, and payloads"}
        self.assertEqual(guard.stale_table_size_claims(docs, self.LENGTHS), [])


class MeasurementFailureTests(unittest.TestCase):
    """A measurement that could not be taken is reported, not left to skip
    every claim that depends on it."""

    def test_a_working_measurement_covers_every_measured_phrase(self):
        problems = []
        counts = guard.measured_counts(problems)
        self.assertEqual(set(counts), set(guard.MEASURED_RE))
        self.assertEqual(problems, [])
        self.assertEqual(counts["golden"], 88)
        self.assertGreater(counts["metrics_builds"], 0)

    def test_a_failed_measurement_is_reported_rather_than_skipped(self):
        real = guard.read

        def unreadable_golden(path):
            return "" if path.name == "golden_vectors.rs" else real(path)

        problems = []
        with patch.object(guard, "read", side_effect=unreadable_golden):
            counts = guard.measured_counts(problems)
        self.assertNotIn("golden", counts)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("golden", problems[0])


class ContradictingCountTests(unittest.TestCase):
    """The half of the count check that survives `--fast`: with exactly two
    suites, a third distinct value is a contradiction."""

    def test_a_third_distinct_count_is_reported(self):
        docs = {"README.md": "- **394 tests**\nverified: **355 passing**",
                "USAGE.md": "# 133 passing"}
        problems = guard.contradicting_test_counts(docs)
        self.assertTrue(problems)
        self.assertIn("355", " ".join(problems))

    def test_the_two_live_suites_are_not_a_contradiction(self):
        docs = {"README.md": "394 tests and the Python suite has 133 tests",
                "USAGE.md": "394 passing\n133 passing"}
        self.assertEqual(guard.contradicting_test_counts(docs), [])

    def test_one_count_everywhere_is_not_a_contradiction(self):
        docs = {"README.md": "394 tests", "USAGE.md": "394 passing"}
        self.assertEqual(guard.contradicting_test_counts(docs), [])

    def test_a_stale_highlight_is_a_third_count(self):
        docs = {"README.md": "- **390 Rust tests** plus\n394 passing",
                "USAGE.md": "# 133 passing"}
        self.assertIn("390", " ".join(guard.contradicting_test_counts(docs)))

    def test_the_report_names_every_site_so_the_stale_one_can_be_found(self):
        docs = {"README.md": "394 tests\n355 passing", "USAGE.md": "133 passing"}
        joined = " ".join(guard.contradicting_test_counts(docs))
        for expected in ("README.md:1", "README.md:2", "USAGE.md:1"):
            self.assertIn(expected, joined)


class MeasuredCountTests(unittest.TestCase):
    """Counts produced by something runnable, quoted in prose that rots."""

    def test_three_different_correction_counts_are_all_caught(self):
        text = "(85 corrections)\nsays 86 corrections\n# 49 corrections present"
        problems = guard.stale_measured_counts({"x.md": text}, {"corrections": 85})
        self.assertEqual(len(problems), 2, problems)

    def test_a_stale_golden_vector_count_is_caught(self):
        text = "66 mechanically extracted golden vectors (11 staging"
        problems = guard.stale_measured_counts({"x.md": text}, {"golden": 77})
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("66", problems[0])

    def test_a_metrics_build_count_counts_only_beside_its_guard(self):
        live = {"metrics_builds": 7}
        unrelated = "The transform is shared across 7 builds (5 builds before)."
        self.assertEqual(guard.stale_measured_counts({"x.md": unrelated}, live), [])
        stale = "| `check_metrics_baseline.py` | rounds, score, K/D/A (5 builds) |"
        self.assertEqual(len(guard.stale_measured_counts({"x.md": stale}, live)), 1)

    def test_live_correction_count_is_pinned(self):
        # Pinned on purpose: a verified typing change must change it visibly.
        self.assertEqual(guard.measured_counts()["corrections"], 225)


class GeneratedInventoryTests(unittest.TestCase):
    def test_a_missing_checksum_table_is_reported(self):
        docs = {
            name: " ".join(
                value
                for target, generator in guard.GENERATED_INVENTORY.items()
                if not target.endswith("checksum_table.rs")
                for value in (target, generator)
            )
            for name in guard.GENERATED_INVENTORY_DOCS
        }
        problems = guard.check_generated_inventory(docs)
        self.assertTrue(any("checksum_table.rs" in p for p in problems), problems)


class BaselineFigureTests(unittest.TestCase):
    def test_a_stale_export_measurement_is_reported(self):
        tables = guard.baseline_table_figures()
        live = f"{tables['fields.parquet'][0]:,}"
        stale = f"{tables['fields.parquet'][0] + 1:,}"
        rows = "\n".join(f"| `{name}` | {n:,} | {size:,} |"
                         for name, (n, size) in tables.items())
        docs = {"README.md": rows.replace(live, stale, 1)}
        problems = guard.check_baseline_figures(docs, tables)
        self.assertTrue(any("fields.parquet" in p for p in problems), problems)

    def test_a_missing_baseline_table_is_reported(self):
        tables = guard.baseline_table_figures()
        problems = guard.check_baseline_figures(
            {"README.md": "No measured export table here."}, tables
        )
        self.assertEqual(len(problems), len(tables), problems)

    def test_every_checkpoint_table_is_checked_against_its_own_baseline(self):
        checkpoint = json.loads(guard.read(
            guard.REPO / "tools" / "baselines" / "checkpoint_02d4d478.json"))
        expected = {
            f"{name}.parquet": (int(values["rows"]), int(values["bytes"]))
            for name, values in checkpoint["parquet"].items()
            if name.startswith("checkpoint_")
        }
        self.assertGreater(len(expected), 1, "the baseline lost its checkpoint tables")
        tables = guard.baseline_table_figures()
        self.assertEqual({name: tables.get(name) for name in expected}, expected)


class DocCoverageTests(unittest.TestCase):
    def test_data_md_contributing_and_claude_md_are_covered(self):
        self.assertLessEqual({"docs/DATA.md", "CONTRIBUTING.md", "CLAUDE.md"},
                             set(guard.ALL_DOCS))


class CheckCountTests(unittest.TestCase):
    """The printed `N checks` is the number of results main() combines, read
    from its source: the elements of the `checks` list plus each
    `checks.append` (the suite measurement)."""

    def checks_in_main(self) -> tuple[int, int]:
        tree = ast.parse(textwrap.dedent(inspect.getsource(guard.main)))
        listed = [node.value for node in ast.walk(tree)
                  if isinstance(node, ast.Assign)
                  and [getattr(t, "id", None) for t in node.targets] == ["checks"]
                  and isinstance(node.value, ast.List)]
        self.assertEqual(len(listed), 1, "main() must combine its checks in one "
                         "`checks` list, or its printed count is not derived from them")
        appended = sum(1 for node in ast.walk(tree)
                       if isinstance(node, ast.Call)
                       and isinstance(node.func, ast.Attribute)
                       and node.func.attr == "append"
                       and getattr(node.func.value, "id", None) == "checks")
        return len(listed[0].elts), appended

    def printed_count(self, *argv: str) -> int:
        with patch.object(guard, "measure_tests", return_value=(1, 1, [])):
            _, output, _ = run_cli(guard.main, *argv, prog="check_docs.py")
        found = re.search(r"(\d+) checks$", output, re.M)
        self.assertIsNotNone(found, output)
        return int(found.group(1))

    def test_the_printed_count_is_the_number_of_checks_combined(self):
        listed, appended = self.checks_in_main()
        self.assertEqual(self.printed_count("--fast"), listed)
        self.assertEqual(self.printed_count(), listed + appended)


if __name__ == "__main__":
    unittest.main()
