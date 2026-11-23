"""Guards for the spike-custody view: the owner classification, the one place
the script judges rather than reads a column (an `Owner` NetGUID is the player
carrying the spike, nobody, or a proxy walked back through its `Instigator`);
the plant-time carrier lookup; the failure conditions; and the whole join on a
synthetic export whose carrier is a reconnected player's earlier pawn."""
import json
import os
import subprocess
import sys
import unittest
from unittest import mock

import pyarrow as pa
import pyarrow.parquet as pq

from support import TempDirTestCase, read_parquet, run_cli
import extract_spike_carrier as spike


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
            "bomb_net_guid": 900, "owner_net_guid": pawn,
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

    def test_a_boundary_with_a_different_owner_is_unresolved(self):
        for kind in ("loose", "unknown", "player", "proxy"):
            with self.subTest(kind=kind):
                rows = [interval(100, 200), interval(200, 300, pawn=870)]
                rows[1]["holder_kind"] = kind
                self.assertIsNone(spike.carrier_at(rows, 200))

    def test_a_plant_tied_with_terminal_close_is_unresolved(self):
        self.assertIsNone(spike.carrier_at([interval(100, 200)], 200))

    def test_repeated_same_owner_writes_preserve_attribution(self):
        rows = [interval(100, 200), interval(200, 200), interval(200, 300)]
        self.assertEqual(spike.carrier_at(rows, 200), rows[-1])

    def test_a_zero_duration_noncarrier_preserves_boundary_ambiguity(self):
        rows = [interval(100, 200), interval(200, 200), interval(200, 300)]
        rows[1]["holder_kind"] = "loose"
        self.assertIsNone(spike.carrier_at(rows, 200))


class UnresolvedTests(unittest.TestCase):
    """No custody at all, or a plant with no carrier, fails the run."""

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

    def test_a_plant_at_a_noncarrier_transition_is_unresolved(self):
        for kind in ("loose", "unknown"):
            with self.subTest(kind=kind):
                rows = [interval(100, 200), interval(200, 300)]
                rows[1]["holder_kind"] = kind
                events = {"group": ["spikePlanted"], "time1": [200]}
                self.assertEqual(len(spike.unresolved(rows, events)), 1)


BOMB = "/Game/Equippables/Bomb/BombEquippable.BombEquippable_C"
PLAYER_STATE = "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C"
WRAITH = "/Game/Characters/Wraith/Wraith_PC.Wraith_PC_C"


class ReconnectedCarrierTests(TempDirTestCase):
    """39c2bb2c (13.05): PlayerState 256's SpawnedCharacter goes 1510 -> 0 ->
    45530 and the manifest keeps 45530. Pawn 1510 carried and planted the
    spike in round 5; a join on the manifest's last pawn alone reads that
    custody as `unknown` and the plant as `NO CARRIER`."""

    def export(self):
        root = self.tmp()
        # PlayerState 300 has a subject and no character, as some real ones do.
        (root / "manifest.json").write_text(json.dumps({"players": [
            {"actor_net_guid": 256, "subject": "reconnected", "character_net_guid": 45530},
            {"actor_net_guid": 300, "subject": "no-character", "character_net_guid": None}]}),
            encoding="utf-8")

        def field(time_ms, actor, group, name, value):
            return {"time_ms": time_ms, "packet_id": time_ms, "actor_net_guid": actor,
                    "object_net_guid": None, "group_path": group, "field_name": name,
                    "value_i64": value, "raw_bits": None}

        fields = [field(66, 256, PLAYER_STATE, "SpawnedCharacter", 1510),
                  field(100, 900, BOMB, "Owner", 1510),
                  field(300, 900, BOMB, "Owner", 950),
                  field(1851838, 256, PLAYER_STATE, "SpawnedCharacter", 0),
                  field(1948245, 256, PLAYER_STATE, "SpawnedCharacter", 45530),
                  field(1948300, 900, BOMB, "Owner", 45530)]
        pq.write_table(pa.Table.from_pylist(fields, schema=pa.schema([
            ("time_ms", pa.uint32()), ("packet_id", pa.uint32()),
            ("actor_net_guid", pa.uint32()), ("object_net_guid", pa.uint32()),
            ("group_path", pa.string()), ("field_name", pa.string()),
            ("value_i64", pa.int64()), ("raw_bits", pa.binary())])), root / "fields.parquet")
        actors = [{"time_ms": 66, "actor_net_guid": 1510, "event": "open", "class_path": WRAITH},
                  {"time_ms": 90, "actor_net_guid": 900, "event": "open", "class_path": BOMB},
                  {"time_ms": 290, "actor_net_guid": 950, "event": "open", "class_path": GROUND}]
        pq.write_table(pa.Table.from_pylist(actors, schema=pa.schema([
            ("time_ms", pa.uint32()), ("actor_net_guid", pa.uint32()),
            ("event", pa.string()), ("class_path", pa.string())])), root / "actors.parquet")
        events = {"group": ["roundStarted", "spikePlanted"], "time1": [50, 250],
                  "metadata": ["5", None]}
        pq.write_table(pa.table(events), root / "events.parquet")
        return root

    def test_an_earlier_pawn_of_a_reconnected_player_is_the_carrier(self):
        rows = spike.build(self.export())[0]
        first = rows[0]
        self.assertEqual((first["owner_net_guid"], first["holder_kind"]), (1510, "player"))
        self.assertEqual(first["carrier_subject"], "reconnected")
        self.assertIn("SpawnedCharacter history", first["carrier_identity_provenance"])
        self.assertEqual(rows[-1]["carrier_identity_provenance"],
                         "manifest.players.character_net_guid (SpawnedCharacter)")
        self.assertIsNone(rows[1]["carrier_identity_provenance"])

    def test_the_plant_by_the_earlier_pawn_resolves(self):
        rows, events = spike.build(self.export())[:2]
        self.assertEqual(spike.unresolved(rows, events), [])

    def test_a_loose_spike_has_no_carrier_subject(self):
        """The manifest-only map held a None key for a player with no
        character, so `pawn_subject.get(None)` gave every loose interval that
        player's subject: 86 and 46 intervals on 4b8191e8 and 8cda0666."""
        loose = spike.build(self.export())[0][1]
        self.assertEqual((loose["holder_kind"], loose["carrier_pawn_guid"]), ("loose", None))
        self.assertIsNone(loose["carrier_subject"])

    def test_input_paths_and_symlink_aliases_are_refused_before_build(self):
        for name in ("fields.parquet", "actors.parquet", "manifest.json", "events.parquet"):
            for alias in (False, True):
                with self.subTest(name=name, alias=alias):
                    root = self.export()
                    source = root / name
                    before = source.read_bytes()
                    target = root / "alias.parquet" if alias else source
                    if alias:
                        try:
                            target.symlink_to(source)
                        except OSError:
                            self.skipTest("symlink creation unavailable on this platform")
                    with mock.patch.object(spike, "build", wraps=spike.build) as build:
                        code = run_cli(spike.main, "--export", root, "--out", target,
                                       prog="extract_spike_carrier.py")[0]
                    self.assertEqual(code, 1)
                    self.assertEqual(source.read_bytes(), before)
                    build.assert_not_called()

    def test_existing_analysis_and_hard_link_outputs_can_be_replaced(self):
        for hard_link in (False, True):
            with self.subTest(hard_link=hard_link):
                root = self.export()
                source, target = root / "fields.parquet", root / "spike_carrier.parquet"
                before = source.read_bytes()
                if hard_link:
                    os.link(source, target)
                else:
                    target.write_bytes(b"previous analysis")
                code = run_cli(spike.main, "--export", root, "--out", target,
                               prog="extract_spike_carrier.py")[0]
                self.assertEqual(code, 0)
                self.assertEqual(source.read_bytes(), before)
                self.assertEqual(read_parquet(target).column_names, spike.SCHEMA.names)

    def test_cli_reports_unresolved_custody_at_and_after_the_drop(self):
        for plant_ms, expected in ((299, 0), (300, 1), (301, 1)):
            with self.subTest(plant_ms=plant_ms):
                root = self.export()
                pq.write_table(pa.table({"group": ["spikePlanted"],
                                        "time1": [plant_ms], "metadata": [None]}),
                               root / "events.parquet")
                target = root / "spike_carrier.parquet"
                result = subprocess.run(
                    [sys.executable, "-B", "-W", "error", spike.__file__,
                     "--export", str(root), "--out", str(target)],
                     capture_output=True, text=True, encoding="utf-8", errors="strict",
                     check=False)
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertEqual(read_parquet(target)["owner_net_guid"].to_pylist(),
                                 [1510, 950, 45530])

    def test_equal_time_writes_keep_packet_then_physical_row_order(self):
        for packets in ((10, 20, 30), (10, 20, 20), (10, 30, 20)):
            with self.subTest(packets=packets):
                root = self.export()
                table = read_parquet(root / "fields.parquet")
                fields = table.to_pylist()
                for row, time, packet, owner in zip(
                        (fields[1], fields[2], fields[5]), (100, 200, 200),
                        packets, (1510, 45530, 1510)):
                    row.update(time_ms=time, packet_id=packet, value_i64=owner)
                pq.write_table(pa.Table.from_pylist(fields, schema=table.schema),
                               root / "fields.parquet")
                rows = spike.build(root)[0]
                final_owner = 45530 if packets == (10, 30, 20) else 1510
                self.assertEqual(spike.carrier_at(rows, 250)["owner_net_guid"], final_owner)
                self.assertEqual([r["owner_net_guid"] for r in rows],
                                 [1510, 1510, 45530] if final_owner == 45530
                                 else [1510, 45530, 1510])


class LeafTests(unittest.TestCase):
    def test_a_class_path_reduces_to_its_last_segment(self):
        self.assertEqual(
            spike.leaf("/Game/Equippables/Bomb/BombEquippable.BombEquippable_C"),
            "BombEquippable.BombEquippable_C")

    def test_an_absent_class_is_the_empty_string(self):
        self.assertEqual(spike.leaf(None), "")
