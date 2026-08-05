"""Guards for the spike-custody derived view.

The join itself is checked by running the script against a real export. What is
pinned here is the owner classification, which is the one place the script makes
a judgement rather than reading a column: an `Owner` NetGUID has to come out as
the player carrying the spike, nobody at all, or a proxy carrier walked back
through its `Instigator`.

An earlier version also unpacked `SerializeIntPacked` out of `raw_bits`, because
`Owner` arrived untyped on this group. The overlay now resolves it by name, so
that decoder and its vectors are gone.
"""
import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import extract_spike_carrier as spike  # noqa: E402


PAWNS = {576: "gekko-uuid", 870: "other-uuid"}
GROUND = "/Game/Equippables/EquippableGroundPickup.EquippableGroundPickup_C"
PROJECTILE = "/Game/Equippables/EquippablePickupProjectile.EquippablePickupProjectile_C"
WINGMAN = "/Game/Characters/AggroBot/Pawn_Aggrobot_SeekerNade.Pawn_Aggrobot_SeekerNade_C"


class ClassifyOwnerTests(unittest.TestCase):
    def test_a_manifest_character_carries_it_itself(self):
        self.assertEqual(
            spike.classify_owner(576, "/Game/Whatever.Pawn_C", PAWNS, {}),
            ("player", 576, ""))

    def test_a_ground_pickup_means_nobody_has_it(self):
        self.assertEqual(
            spike.classify_owner(999, GROUND, PAWNS, {}), ("loose", None, ""))

    def test_a_drop_projectile_also_means_nobody_has_it(self):
        self.assertEqual(
            spike.classify_owner(999, PROJECTILE, PAWNS, {}),
            ("loose", None, ""))

    def test_a_proxy_resolves_through_its_own_instigator(self):
        """Gekko's Wingman really does carry and plant the spike."""
        kind, carrier, proxy = spike.classify_owner(
            5924, WINGMAN, PAWNS, {5924: 576})
        self.assertEqual((kind, carrier), ("proxy", 576))
        self.assertIn("Pawn_Aggrobot_SeekerNade", proxy)

    def test_a_proxy_whose_instigator_is_not_a_player_stays_unknown(self):
        """No guessing: an unrecognised chain is reported, not attributed."""
        self.assertEqual(
            spike.classify_owner(5924, WINGMAN, PAWNS, {5924: 4242}),
            ("unknown", None, ""))

    def test_an_owner_with_no_class_and_no_instigator_stays_unknown(self):
        self.assertEqual(
            spike.classify_owner(999, None, PAWNS, {}), ("unknown", None, ""))

    def test_the_player_check_wins_over_a_loose_looking_class(self):
        """A pawn in the manifest is the carrier whatever its class path says."""
        self.assertEqual(
            spike.classify_owner(576, GROUND, PAWNS, {}), ("player", 576, ""))


class LeafTests(unittest.TestCase):
    def test_a_class_path_reduces_to_its_last_segment(self):
        self.assertEqual(
            spike.leaf("/Game/Equippables/Bomb/BombEquippable.BombEquippable_C"),
            "BombEquippable.BombEquippable_C")

    def test_an_absent_class_is_the_empty_string(self):
        self.assertEqual(spike.leaf(None), "")


if __name__ == "__main__":
    unittest.main()
