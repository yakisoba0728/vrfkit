import sys
import tempfile
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import find_skips as guard  # noqa: E402


class SummaryPatternTests(unittest.TestCase):
    def test_malformed_framing_label_is_parsed(self):
        match = guard.MALFORMED.search("Malformed framing:  17")
        self.assertIsNotNone(match)
        self.assertEqual(match.group(1), "17")


class DiscoveryTests(unittest.TestCase):
    def test_uppercase_vrf_extension_is_discovered(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "lower.vrf").write_bytes(b"")
            (root / "UPPER.VRF").write_bytes(b"")

            files = guard.find_replays(root, limit=None)

        self.assertEqual({path.name for path in files}, {"UPPER.VRF", "lower.vrf"})


if __name__ == "__main__":
    unittest.main()
