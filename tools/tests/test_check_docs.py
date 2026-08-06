"""Guards for the doc guard.

check_docs.py catches stale documentation, which nothing else can: a wrong
number in prose compiles and passes every test. Its own detection logic is
therefore the thing that must not rot into something that passes everything.
"""
import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_docs as guard  # noqa: E402


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

    def test_the_real_usage_doc_covers_every_crate(self):
        usage = guard.read(guard.USAGE)
        self.assertEqual(guard.check_crates(usage), [])


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

    def test_the_shipped_docs_have_no_dead_links(self):
        for path in (guard.README, guard.USAGE):
            self.assertEqual(guard.check_links(path, guard.read(path)), [], path.name)


class TableSizeTests(unittest.TestCase):
    def test_a_stale_table_size_is_reported(self):
        docs = {"README.md": "the table has 999 entries",
                "USAGE.md": "the table has 999 entries"}
        problems = guard.check_table_sizes(docs)
        self.assertTrue(problems)

    def test_the_shipped_docs_quote_the_live_sizes(self):
        docs = {"README.md": guard.read(guard.README),
                "USAGE.md": guard.read(guard.USAGE)}
        self.assertEqual(guard.check_table_sizes(docs), [])

    def test_both_comma_and_plain_forms_are_accepted(self):
        """1187 and 1,187 are the same claim; neither should fail."""
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
    """The sixth check: Rust prose and Cargo.toml quote the size too.

    Three places said 1,185 after the table reached 1,188 and nothing read
    them, which is the whole argument for check_docs.py happening one directory
    outside its reach.
    """

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
        """Scoped to the exact phrasing so the check has no judgement to make.

        Dated measurements elsewhere legitimately say things like "1,054
        entries" about a table that no longer exists; only "N-entry table" is
        read as a claim about the live one.
        """
        self.assertEqual(
            guard.stale_entry_phrases("measured over 1,054 entries", self.LIVE), [])

    def test_the_shipped_crates_quote_the_live_size(self):
        self.assertEqual(guard.check_source_table_size(), [])


class TestCountTests(unittest.TestCase):
    """The seventh check: a doc may not carry a stale suite size beside a live one.

    The count check used to ask only whether the live number appeared somewhere
    in the file, so a README saying 387 in one place and 355 in another passed
    it -- and did, for twelve commits. Presence is not agreement.
    """

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
        """Only "N tests" / "N passing" is read as a claim about the suites.

        Prose legitimately quotes unrelated figures -- README recovers "2,387
        intermediate moves", which is how the old presence check was satisfied
        by accident.
        """
        self.assertEqual(
            guard.stale_test_counts("we recover 2,387 intermediate moves", self.LIVE), [])


class ContradictingCountTests(unittest.TestCase):
    """The half of the count check that survives `--fast`, and so the half CI runs.

    Without running the suites there is no way to know which number is right.
    But the repo has exactly two of them, so a third distinct value is a
    contradiction on its face -- which is precisely the shape the real bug had:
    387 and 355 sitting in one file, both about `cargo test`.
    """

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

    def test_the_report_names_every_site_so_the_stale_one_can_be_found(self):
        docs = {"README.md": "394 tests\n355 passing", "USAGE.md": "133 passing"}
        joined = " ".join(guard.contradicting_test_counts(docs))
        for expected in ("README.md:1", "README.md:2", "USAGE.md:1"):
            self.assertIn(expected, joined)

    def test_the_shipped_docs_do_not_contradict_themselves(self):
        docs = {"README.md": guard.read(guard.README),
                "USAGE.md": guard.read(guard.USAGE)}
        self.assertEqual(guard.contradicting_test_counts(docs), [])


if __name__ == "__main__":
    unittest.main()


class MeasuredCountTests(unittest.TestCase):
    """Counts that live somewhere runnable, quoted in prose that rots.

    `check_ascii`'s file count and `apply_type_corrections`'s correction count
    were both quoted in README and USAGE -- files this guard already read --
    and both went stale anyway, because nothing here knew those two numbers
    existed. USAGE said 85, 86 and 49 corrections in one file.
    """

    def test_a_stale_ascii_count_is_caught(self):
        problems = guard.stale_measured_counts(
            {"x.md": "`check_ascii` on 999 files."}, {"ascii": 115})
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("999", problems[0])

    def test_the_live_ascii_count_passes(self):
        text = "`check_ascii` on 115 files, and 115 files, ASCII only"
        self.assertEqual(
            guard.stale_measured_counts({"x.md": text}, {"ascii": 115}), [])

    def test_three_different_correction_counts_are_all_caught(self):
        text = "(85 corrections)\nsays 86 corrections\n# 49 corrections present"
        problems = guard.stale_measured_counts({"x.md": text}, {"corrections": 85})
        self.assertEqual(len(problems), 2, problems)

    def test_a_corpus_file_count_is_not_an_ascii_claim(self):
        """README says "all 215 files" about replays, not about the sweep."""
        text = "- **Framing** (`validate_corpus.py`, all 215 files) -- framing."
        self.assertEqual(
            guard.stale_measured_counts({"x.md": text}, {"ascii": 115}), [])

    def test_the_repository_is_clean(self):
        live = guard.measured_counts()
        docs = {name: guard.read(guard.REPO / name) for name in guard.ALL_DOCS}
        self.assertEqual(guard.stale_measured_counts(docs, live), [])


class DocCoverageTests(unittest.TestCase):
    """DATA.md and CONTRIBUTING.md were read by nothing at all."""

    def test_data_md_and_contributing_are_covered(self):
        self.assertIn("docs/DATA.md", guard.ALL_DOCS)
        self.assertIn("CONTRIBUTING.md", guard.ALL_DOCS)

    def test_every_covered_doc_has_resolving_links(self):
        for name in guard.ALL_DOCS:
            path = guard.REPO / name
            self.assertEqual(guard.check_links(path, guard.read(path)), [], name)
