"""A failed audit must remain visible, and table wording must have one meaning."""
from copy import deepcopy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_docs as guard


class BuildVerificationDocsTests(unittest.TestCase):
    REGISTRY = "pub const ALL_VERSIONS: &[TransformVersion] = &[TransformVersion::V1106];"
    REPORT = {"executable_changed": False, "builds": {
        "++Ares-Core+release-11.06": {"passed": 2, "failed": 1, "replays": 3,
                                      "input_sha256": ["a", "b", "c"]}}}
    README = "| **11.06** | `release-11.06` | 2/3 | Validation + checkpoints + typed/raw |"
    USAGE = "| 11.06 | 2/3 | Validation + checkpoints + typed/raw |"

    def check(self, readme=None, usage=None, report=None):
        return guard.check_build_verification(
            self.README if readme is None else readme,
            self.USAGE if usage is None else usage,
            self.REGISTRY, self.REPORT if report is None else report)

    def test_accurate_mixed_result_is_allowed(self):
        self.assertEqual(self.check(), [])

    def test_failed_replay_cannot_be_relabelled_clean(self):
        self.assertTrue(self.check(readme=self.README.replace("2/3", "3/3")))

    def test_verification_methods_cannot_diverge(self):
        self.assertTrue(self.check(usage=self.USAGE.replace(guard.BUILD_METHOD, "golden vectors")))
        self.assertTrue(self.check(readme=self.README + "\nPayload transform (8 builds)"))

    def test_missing_or_duplicate_build_is_detected(self):
        self.assertTrue(self.check(usage=""))
        self.assertTrue(self.check(readme=self.README + "\n" + self.README))

    def test_report_must_cover_registry_and_unchanged_executable(self):
        report = deepcopy(self.REPORT)
        report["builds"] = {}
        self.assertTrue(self.check(report=report))
        report = deepcopy(self.REPORT)
        del report["executable_changed"]
        self.assertTrue(self.check(report=report))

    def test_empty_registry_and_inconsistent_report_cannot_pass(self):
        self.assertTrue(guard.check_build_verification(
            "", "", "ALL_VERSIONS: &[T] = &[];", {"builds": {}}))
        report = deepcopy(self.REPORT)
        report["builds"]["++Ares-Core+release-11.06"]["failed"] = 0
        self.assertTrue(self.check(report=report))
        report = deepcopy(self.REPORT)
        report["builds"]["++Ares-Core+release-11.06"]["input_sha256"] = ["a", "a", "c"]
        self.assertTrue(self.check(report=report))


if __name__ == "__main__":
    unittest.main()
