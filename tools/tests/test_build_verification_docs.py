"""A failed audit must remain visible, and table wording must have one meaning."""
from copy import deepcopy
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_docs as guard
from support import run_cli


class BuildVerificationDocsTests(unittest.TestCase):
    # The shape of crates/vrf-transform/src/lib.rs's registry block.
    REGISTRY = 'transforms! {\n    V1106 V11_06 "11.06" 0x3325_e3bd  0x3d: add 8, sbox;\n}\n'
    REPORT = {"executable_changed": False, "builds": {
        "++Ares-Core+release-11.06": {"passed": 2, "failed": 1, "replays": 3,
                                      "checkpoint_evidence": "observed",
                                      "counts": {"checkpoint_content_blocks": 5,
                                                 "checkpoint_overlay_decoded_ok": 8},
                                      "input_sha256": ["a", "b", "c"]}}}
    README = "| **11.06** | `release-11.06` | 2/3 | Validation + checkpoints + typed/raw |"
    USAGE = "| Payload transform (1 builds) | `vrf-transform` | none |"

    def check(self, readme=None, usage=None, report=None, registry=None):
        return guard.check_build_verification(
            self.README if readme is None else readme,
            self.USAGE if usage is None else usage,
            self.REGISTRY if registry is None else registry,
            self.REPORT if report is None else report)

    def test_accurate_mixed_result_is_allowed(self):
        self.assertEqual(self.check(), [])

    def test_failed_replay_cannot_be_relabelled_clean(self):
        self.assertTrue(self.check(readme=self.README.replace("2/3", "3/3")))

    def test_verification_methods_cannot_diverge(self):
        self.assertTrue(self.check(readme=self.README.replace(guard.BUILD_METHOD, "golden vectors")))
        self.assertTrue(self.check(usage=self.USAGE.replace("(1 builds)", "(8 builds)")))

    def test_missing_or_duplicate_build_is_detected(self):
        self.assertTrue(self.check(readme=""))
        self.assertTrue(self.check(readme=self.README + "\n" + self.README))

    def test_report_must_cover_registry_and_unchanged_executable(self):
        report = deepcopy(self.REPORT)
        report["builds"] = {}
        self.assertTrue(self.check(report=report))
        report = deepcopy(self.REPORT)
        del report["executable_changed"]
        self.assertTrue(self.check(report=report))

    def test_empty_or_unreadable_registry_and_inconsistent_report_cannot_pass(self):
        self.assertTrue(self.check(registry="transforms! {\n}\n"))
        self.assertTrue(self.check(registry="pub const ALL_VERSIONS: &[T] = &[V1106];"))
        report = deepcopy(self.REPORT)
        report["builds"]["++Ares-Core+release-11.06"]["failed"] = 0
        self.assertTrue(self.check(report=report))

        report = deepcopy(self.REPORT)
        report["builds"]["++Ares-Core+release-11.06"]["input_sha256"] = ["a", "a", "c"]
        self.assertTrue(self.check(report=report))

    def test_checkpoint_claim_requires_real_work_and_no_build_errors(self):
        for key in ("checkpoint_content_blocks", "checkpoint_overlay_decoded_ok"):
            for bad in (0, None, True, -1):
                with self.subTest(key=key, bad=bad):
                    report = deepcopy(self.REPORT)
                    report["builds"]["++Ares-Core+release-11.06"]["counts"][key] = bad
                    self.assertTrue(self.check(report=report))
        report = deepcopy(self.REPORT)
        report["build_errors"] = ["checkpoint work absent"]
        self.assertTrue(self.check(report=report))


class TransformEvidenceTests(unittest.TestCase):
    """docs/BUILD_VERIFICATION.md's arithmetic-evidence paragraph against the
    live registry and vector files, then each claim broken in turn."""
    DOC = guard.read(guard.REPO / "docs/BUILD_VERIFICATION.md")
    REGISTRY = guard.read(guard.REPO / "crates/vrf-transform/src/lib.rs")
    VECTORS = {path.name: guard.read(path) for path in guard.TRANSFORM_VECTORS.glob("*.rs")}

    def check(self, doc=None, registry=None, vectors=None):
        return guard.check_transform_evidence(
            self.DOC if doc is None else doc, self.REGISTRY if registry is None else registry,
            self.VECTORS if vectors is None else vectors)

    def mutated(self, old, new):
        doc = " ".join(self.DOC.split("## Arithmetic evidence", 1)[1].split())
        self.assertIn(old, doc)
        return "## Arithmetic evidence\n" + doc.replace(old, new, 1) + "\n"

    def test_the_committed_paragraph_matches_the_registry_and_vectors(self):
        self.assertEqual(self.check(), [])
        data = guard.vector_cases(self.VECTORS)
        self.assertEqual({source: len(builds) for source, builds in data.items()},
                         {"native": 18, "golden": 8, "recovered": 1})

    def test_each_miscounted_or_misranged_claim_fails(self):
        for old, new in (
                ("cover all 27 registered", "cover all 26 registered"),
                ("The eighteen builds", "The seventeen builds"),
                ("79 independently", "80 independently"),
                ("the eight builds", "the nine builds"),
                ("12.10--13.06 have", "12.10--13.05 have"),
                ("13.08 and 13.09", "13.08"),
                ("eleven golden", "twelve golden"),
                ("13.10 has eleven", "13.10 has ten"),
                ("13.10 has eleven fixed boundary vectors", "13.10 has eleven vectors"),
                ("registered builds", "builds")):
            with self.subTest(old=old):
                self.assertEqual(self.check(doc=self.mutated(old, old)), [])
                self.assertTrue(self.check(doc=self.mutated(old, new)))

    def test_a_build_moved_between_groups_fails(self):
        """Counts and coverage intact, but 13.06 has no native vectors and
        13.09 no golden ones."""
        doc = self.mutated("13.08 and 13.09", "13.06 and 13.08")
        doc = doc.replace("12.10--13.06 have", "12.10--13.05 and 13.09 have", 1)
        problems = self.check(doc=doc)
        self.assertEqual(len(problems), 2, problems)
        self.assertIn("['13.06'] have [None]", problems[0])
        self.assertIn("['13.09'] have [None]", problems[1])

    def test_a_registered_build_without_vectors_fails_even_when_counted(self):
        registry = self.REGISTRY.replace(
            '    V1310 V13_10', '    V1311 V13_11 "13.11" 0x0 0x0: not;\n    V1310 V13_10', 1)
        doc = self.mutated("cover all 27", "cover all 28")
        self.assertEqual(self.check(doc=doc, registry=registry),
                         ["BUILD_VERIFICATION.md: no group lists ['13.11']; listed twice: []"])

    def test_main_fails_on_a_miscounted_paragraph(self):
        real = guard.read

        def miscounted(path):
            text = real(path)
            return (text.replace("cover all 27", "cover all 26")
                    if path.name == "BUILD_VERIFICATION.md" else text)

        with patch.object(guard, "read", side_effect=miscounted):
            code, output, err = run_cli(guard.main, "--fast", prog="check_docs.py",
                                        merged=True)
        self.assertEqual(code, 1, output)
        self.assertIn("transform tests must claim all 27 registered builds", output)

    def test_a_missing_section_or_registry_cannot_pass(self):
        self.assertTrue(self.check(doc=self.DOC.replace("## Arithmetic evidence", "## Arithmetic")))
        self.assertTrue(self.check(registry="transforms! {\n}\n"))

    def test_a_new_build_or_a_lost_vector_fails_the_unchanged_paragraph(self):
        registry = self.REGISTRY.replace(
            '    V1310 V13_10', '    V1311 V13_11 "13.11" 0x0 0x0: not;\n    V1310 V13_10', 1)
        self.assertTrue(self.check(registry=registry))
        native = self.VECTORS["native_vectors.rs"]
        first = native.index('    ("++Ares-Core+release-13.09"')
        vectors = dict(self.VECTORS, **{
            "native_vectors.rs": native[:first] + native[native.index("\n", first) + 1:]})
        self.assertTrue(self.check(vectors=vectors))
        self.assertTrue(self.check(vectors={k: v for k, v in self.VECTORS.items()
                                            if not k.startswith("recovered_")}))


if __name__ == "__main__":
    unittest.main()
