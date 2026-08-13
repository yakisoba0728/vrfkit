import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import find_skips as guard  # noqa: E402


class SummaryPatternTests(unittest.TestCase):
    def test_malformed_framing_label_is_parsed(self):
        match = guard.MALFORMED.search("Malformed framing:  17")
        self.assertIsNotNone(match)
        self.assertEqual(match.group(1), "17")


if __name__ == "__main__":
    unittest.main()
