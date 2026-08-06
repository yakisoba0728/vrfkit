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


def interval(from_ms, to_ms, subject="gekko-uuid", pawn=576):
    return {"from_ms": from_ms, "to_ms": to_ms, "holder_kind": "player",
            "carrier_pawn_guid": pawn, "carrier_subject": subject,
            "via_proxy_class": "", "round_number": 1}


class CarrierAtTests(unittest.TestCase):
    """Who held the spike at a given instant -- the plant-time join."""

    def test_the_interval_covering_the_moment_is_the_carrier(self):
        held = [interval(0, 100), interval(100, 900), interval(900, 1000)]
        self.assertEqual(spike.carrier_at(held, 500), held[1])

    def test_an_open_ended_final_interval_still_covers_later_moments(self):
        """`to_ms` is None when the bomb actor never closed."""
        held = [interval(0, 100), interval(100, None)]
        self.assertEqual(spike.carrier_at(held, 99999), held[1])

    def test_a_moment_nobody_was_carrying_it_resolves_to_nobody(self):
        self.assertIsNone(spike.carrier_at([interval(0, 100)], 500))

    def test_no_intervals_at_all_resolves_to_nobody(self):
        self.assertIsNone(spike.carrier_at([], 500))


class UnresolvedTests(unittest.TestCase):
    """The command exited 0 whatever it failed to resolve.

    The module docstring already says what a `NO CARRIER` plant means -- "a
    plant with no carrier would mean the chain dropped something" -- and then
    printed it as one more line of output. A run that writes an empty Parquet
    and reports nobody planted the spike is not a successful extraction.
    """

    PLANTED = {"group": ["spikePlanted"], "time1": [500]}

    def test_a_plant_with_a_carrier_resolves(self):
        self.assertEqual(
            spike.unresolved([interval(0, 900)], self.PLANTED), [])

    def test_a_plant_with_no_carrier_is_a_failure(self):
        problems = spike.unresolved([interval(0, 100)], self.PLANTED)
        self.assertTrue(problems)
        self.assertIn("500", " ".join(problems))

    def test_an_extraction_with_no_custody_at_all_is_a_failure(self):
        """An empty Parquet is not an answer."""
        problems = spike.unresolved([], {"group": [], "time1": []})
        self.assertTrue(problems)
        self.assertIn("no custody", " ".join(problems).lower())

    def test_every_unresolved_plant_is_named_not_just_the_first(self):
        events = {"group": ["spikePlanted", "spikePlanted"], "time1": [500, 700]}
        self.assertEqual(len(spike.unresolved([interval(0, 100)], events)), 2)

    def test_a_replay_with_custody_and_no_plants_is_not_a_failure(self):
        """Not every replay has a plant; only a plant that lost its carrier."""
        self.assertEqual(
            spike.unresolved([interval(0, 900)], {"group": [], "time1": []}), [])


class LeafTests(unittest.TestCase):
    def test_a_class_path_reduces_to_its_last_segment(self):
        self.assertEqual(
            spike.leaf("/Game/Equippables/Bomb/BombEquippable.BombEquippable_C"),
            "BombEquippable.BombEquippable_C")

    def test_an_absent_class_is_the_empty_string(self):
        self.assertEqual(spike.leaf(None), "")


if __name__ == "__main__":
    unittest.main()
