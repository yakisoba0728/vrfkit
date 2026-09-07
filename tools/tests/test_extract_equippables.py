"""Equippable resolver generation preserves measured path aliases."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import extract_equippables  # noqa: E402


OLD_GUARDIAN = "/Game/Equippables/Guns/SniperRifles/Dmr/DMR.DMR_C"
NEW_GUARDIAN = "/Game/Equippables/Guns/SniperRifles/DMR/DMR.DMR_C"


class EquippableGeneratorTests(unittest.TestCase):
    def test_guardian_directory_aliases_are_exact_and_generated(self):
        definitions = [(OLD_GUARDIAN, "Guardian", "rifle")]
        namespace = {}
        exec(extract_equippables.render(definitions, "resolver.cs"), namespace)

        lookup = namespace["EQUIPPABLE_BY_PATH"]
        expected = ("Guardian", "rifle", OLD_GUARDIAN)
        self.assertEqual(lookup[OLD_GUARDIAN], expected)
        self.assertEqual(lookup[NEW_GUARDIAN], expected)
        self.assertEqual(lookup[OLD_GUARDIAN.rpartition(".")[0]], expected)
        self.assertEqual(lookup[NEW_GUARDIAN.rpartition(".")[0]], expected)
        self.assertNotIn(NEW_GUARDIAN.replace("/DMR/", "/dMR/"), lookup)


if __name__ == "__main__":
    unittest.main()
