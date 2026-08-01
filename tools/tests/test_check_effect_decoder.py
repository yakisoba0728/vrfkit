import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_effect_decoder as checker  # noqa: E402


CASE_NAMES = (
    "rust_float_sheriff_basic",
    "rust_float_yaw_switch",
    "rust_float_shotgun",
    "rust_object_basic",
    "rust_vector_single",
    "rust_vector_shotgun_12",
    "rust_empty_float",
    "rust_empty_object",
    "rust_empty_vector",
    "reference_float_burst",
    "reference_object_effect_context_only",
    "python_truncated_float_partial",
)


class CheckEffectDecoderTests(unittest.TestCase):
    def test_every_named_corruption_is_detected(self):
        self.assertEqual(tuple(case.name for case in checker.CASES), CASE_NAMES)

        for name in CASE_NAMES:
            with self.subTest(name=name):
                failures = checker.check(name)
                self.assertGreaterEqual(len(failures), 1, name)

    def test_empty_payload_corruptions_are_detected(self):
        for name in (
            "rust_empty_float",
            "rust_empty_object",
            "rust_empty_vector",
        ):
            with self.subTest(name=name):
                self.assertTrue(checker.check(name), name)

    def test_unknown_corruption_name_is_rejected(self):
        self.assertEqual(
            checker.check("not_a_case"),
            ["unknown case for --corrupt: not_a_case"],
        )


if __name__ == "__main__":
    unittest.main()
