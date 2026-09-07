"""Regression coverage for evidence-labelled derived match observations."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq


import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import extract_match_observations as observations  # noqa: E402


def write_export(root: Path) -> None:
    pq.write_table(pa.table({
        "net_guid": [100, 300, 400, 401, 500, 601, 700],
        "path": ["MagazineAmmo", None, "/State/ReloadState", "/State/IdleState", None, None, "InventoryComponent"],
        "outer_net_guid": [200, 200, None, None, 600, None, 600],
    }), root / "net_guids.parquet")
    pq.write_table(pa.table({
        "group": ["roundStarted", "spikeDefused"],
        "time1": [0, 55],
    }), root / "events.parquet")
    rows = [
        (10, 1, 0, 100, "/Script/ShooterGame.AmmoComponent", "AuthResourceAmount", 30, None),
        (11, 1, 0, 100, "/Script/ShooterGame.AmmoComponent", "AuthResourceAmount", 30, None),
        (20, 2, 0, 100, "/Script/ShooterGame.AmmoComponent", "AuthResourceAmount", 28, None),
        (21, 2, 0, 0, "/Script/ShooterGame.ReplayEffectComponent_ClassNetCache", "ReplayPlayContinuousEffectAtLocation.EffectID", 7, None),
        (5, 1, 701, 700, "/Script/ShooterGame.AresInventory", "CurrentEquippable", 200, None),
        (30, 1, 701, 700, "/Script/ShooterGame.AresInventory", "NewCurrentEquippable", 201, None),
        (12, 1, 0, 300, "/Script/ShooterGame.EquippableStateMachineComponent", "CurrentState", 400, None),
        (25, 1, 0, 300, "/Script/ShooterGame.EquippableStateMachineComponent", "CurrentState", 401, None),
        (40, 1, 800, 0, "/Game/GameModes/Bomb/TimedBomb.TimedBomb_C", "DefuseProgress", None, 0.0),
        (50, 1, 800, 0, "/Game/GameModes/Bomb/TimedBomb.TimedBomb_C", "DefuseProgress", None, 1.0),
        (60, 1, 800, 0, "/Game/GameModes/Bomb/TimedBomb.TimedBomb_C", "DefuseProgress", None, 0.0),
        (70, 1, 601, 0, "/Script/ShooterGame.OwnerExclusivePlayerInfo", "RoundInfos[3].StartOfRoundMoney", 800, None),
        (71, 1, 601, 0, "/Script/ShooterGame.OwnerExclusivePlayerInfo", "RoundInfos[3].EndOfRoundMoney", 900, None),
        (69, 1, 600, 0, "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C", "Owner", 42, None),
        (69, 2, 601, 0, "/Script/ShooterGame.OwnerExclusivePlayerInfo", "Owner", 42, None),
        (82, 1, 602, 0, "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C", "Owner", 43, None),
        (82, 2, 601, 0, "/Script/ShooterGame.OwnerExclusivePlayerInfo", "Owner", 43, None),
        (90, 1, 601, 0, "/Script/ShooterGame.OwnerExclusivePlayerInfo", "RoundInfos[4].EndOfRoundMoney", 1000, None),
        (80, 1, 900, 0, "/Script/ShooterGame.BaseTeamState", "LoadoutValue", 5000, None),
        (80, 1, 900, 0, "/Script/ShooterGame.BaseTeamState", "AverageLoadoutValue", 1000, None),
        (81, 1, 901, 0, "/Game/GameModes/Bomb/BombGameState.BombGameState_C", "TeamEconomy[0].LoadoutValue", 4500, None),
        (81, 1, 901, 0, "/Game/GameModes/Bomb/BombGameState.BombGameState_C", "TeamEconomy[0].AverageLoadoutValue", 900, None),
        (90, 1, 0, 500, "/Script/ShooterGame.MoneyManagementComponent", "Money", 1000, None),
        (100, 1, 0, 500, "/Script/ShooterGame.MoneyManagementComponent", "Money", 700, None),
        # The component state arrives across packets. A later duplicate must
        # not become a second observation.
        (101, 1, 910, 911, "/Script/ShooterGame.PurchasedItemComponent", "PurchasingPlayerState", 600, None),
        (102, 2, 910, 911, "/Script/ShooterGame.PurchasedItemComponent", "Purchaseable", 200, None),
        (102, 2, 910, 911, "/Script/ShooterGame.PurchasedItemComponent", "PurchasableTransactionSource", 2, None),
        (103, 3, 910, 911, "/Script/ShooterGame.PurchasedItemComponent", "Purchaseable", 200, None),
        (102, 2, 912, 913, "/Script/ShooterGame.PurchasedItemComponent", "Purchaseable", 201, None),
    ]
    names = ("time_ms", "packet_id", "actor_net_guid", "object_net_guid",
             "group_path", "field_name", "value_i64", "value_f64")
    table = pa.table({name: [row[index] for row in rows]
                      for index, name in enumerate(names)})
    # Match vrfkit's exported schema, including dictionary-encoded names.
    for name in ("group_path", "field_name"):
        table = table.set_column(table.schema.get_field_index(name), name,
                                 table[name].dictionary_encode())
    pq.write_table(table, root / "fields.parquet")


def append_field_rows(root: Path, rows: list[tuple]) -> None:
    """Append rows using the fixture's exported Arrow schema."""
    path = root / "fields.parquet"
    table = pq.read_table(path)
    names = table.schema.names
    addition = pa.Table.from_pylist(
        [{name: row[index] for index, name in enumerate(names)} for row in rows],
        schema=table.schema,
    )
    pq.write_table(pa.concat_tables([table, addition]), path)


class MatchObservationTests(unittest.TestCase):
    def test_build_deduplicates_and_labels_all_v1_evidence(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_export(root)
            result = observations.build(root)

        self.assertEqual(len(result["ammo_changes"]), 1)
        self.assertEqual(result["ammo_changes"][0]["delta"], -2)
        self.assertTrue(result["ammo_changes"][0]["global_effect_id_within_300ms"])
        self.assertEqual({row["source_field"] for row in result["equip_intervals"]},
                         {"CurrentEquippable", "NewCurrentEquippable"})
        self.assertTrue(all(row["duration_ms"] is None
                            for row in result["equip_intervals"]))
        self.assertEqual(result["equip_intervals"][0]["inventory_component_guid"], 700)
        self.assertEqual(result["equip_intervals"][0]["inventory_actor_guid"], 701)
        self.assertEqual(result["equip_intervals"][0]["owner_guid"], 600)
        self.assertEqual(result["reload_intervals"][0]["duration_ms"], 13)
        self.assertFalse(result["defuse_progress_transitions"][0]["authoritative_completion"])
        self.assertEqual(result["defuse_completions"][0]["time_ms"], 55)
        self.assertEqual(result["round_balances"][1]["money"], 900)
        self.assertEqual(result["round_balances"][1]["player_state_guid"], 600)
        self.assertEqual(result["round_balances"][1]["owner_controller_guid"], 42)
        self.assertEqual(result["round_balances"][1]["player_join_source"],
                         "OwnerExclusivePlayerInfo.Owner@time -> BombPlayerState.Owner@time")
        self.assertEqual(result["round_balances"][2]["owner_controller_guid"], 43)
        self.assertEqual(result["round_balances"][2]["player_state_guid"], 602)
        self.assertEqual(result["round_balances"][1]["round_info_slot"], 3)
        self.assertEqual({row["source"] for row in result["team_loadouts"]},
                         {"BaseTeamState", "BombGameState.TeamEconomy"})
        self.assertTrue(all(row["average_times_five_matches_total"]
                            for row in result["team_loadouts"]))
        self.assertEqual(result["money_decreases"][0]["amount"], 300)
        self.assertEqual(len(result["transaction_snapshots"]), 1)
        self.assertEqual(result["transaction_snapshots"][0]["source_packet_id"], 2)
        self.assertEqual(result["transaction_snapshots"][0]["round_ordinal"], 0)
        self.assertEqual(result["transaction_snapshots"][0]["nearby_money_decrease_count_2s"], 1)
        self.assertEqual(result["transaction_snapshots"][0]["transaction_source"], 2)
        self.assertIn("state transition; not a purchase ledger",
                      result["transaction_snapshots"][0]["evidence"])
        self.assertEqual(result["attribution_coverage"]["equip_owner_outer"], {
            "joined": 2,
            "total": 2,
            "source": "net_guids.outer_net_guid(InventoryComponent)",
        })
        self.assertEqual(result["attribution_coverage"]["round_balance_player"]["joined"], 3)

    def test_same_packet_conflict_is_not_value_sorted_into_a_change(self):
        changes, ambiguous = observations._changes([
            (10, 1, 0, 30),
            (10, 1, 1, 28),
            (20, 2, 2, 27),
        ])
        self.assertEqual(changes, [])
        self.assertEqual(ambiguous, 1)

    def test_reload_conflict_closes_at_an_unknown_boundary(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_export(root)
            append_field_rows(root, [
                (20, 3, 0, 300, "/Script/ShooterGame.EquippableStateMachineComponent", "CurrentState", 400, None),
                (20, 3, 0, 300, "/Script/ShooterGame.EquippableStateMachineComponent", "CurrentState", 401, None),
            ])
            result = observations.build(root)

        self.assertEqual(len(result["reload_intervals"]), 1)
        interval = result["reload_intervals"][0]
        self.assertEqual(interval["to_ms"], 20)
        self.assertIsNone(interval["duration_ms"])
        self.assertFalse(interval["closed_by_state_change"])
        self.assertEqual(interval["end_boundary"], "ambiguous_same_packet")

    def test_team_conflicts_are_null_and_row_order_independent(self):
        with tempfile.TemporaryDirectory() as temp:
            original = Path(temp) / "original"
            shuffled = Path(temp) / "shuffled"
            original.mkdir()
            shuffled.mkdir()
            write_export(original)
            write_export(shuffled)
            conflict = [
                (80, 1, 900, 0, "/Script/ShooterGame.BaseTeamState", "LoadoutValue", 5100, None),
            ]
            append_field_rows(original, conflict)
            append_field_rows(shuffled, conflict)
            table = pq.read_table(shuffled / "fields.parquet")
            reverse = pa.array(range(table.num_rows - 1, -1, -1))
            pq.write_table(table.take(reverse), shuffled / "fields.parquet")
            result = observations.build(original)
            self.assertEqual(result, observations.build(shuffled))

        base = next(row for row in result["team_loadouts"] if row["source"] == "BaseTeamState")
        self.assertIsNone(base["loadout_value"])
        self.assertEqual(base["average_loadout_value"], 1000)
        self.assertIsNone(base["average_times_five_matches_total"])
        self.assertEqual(result["ambiguous_same_packet_counts"]["team_loadout"], 1)

    def test_owner_join_does_not_look_ahead_to_a_later_same_time_packet(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_export(root)
            append_field_rows(root, [
                (82, 1, 601, 0, "/Script/ShooterGame.OwnerExclusivePlayerInfo", "RoundInfos[5].EndOfRoundMoney", 1200, None),
            ])
            result = observations.build(root)

        balance = next(row for row in result["round_balances"]
                       if row["round_info_slot"] == 5)
        self.assertEqual(balance["packet_id"], 1)
        self.assertEqual(balance["owner_controller_guid"], 42)
        self.assertEqual(balance["player_state_guid"], 600)

    def test_packet_order_is_stable_when_source_rows_are_shuffled(self):
        samples = [
            (20, 2, 4, 28),
            (10, 1, 2, 30),
            (30, 3, 8, 27),
        ]
        self.assertEqual(observations._changes(samples),
                         observations._changes(list(reversed(samples))))

    def test_build_is_stable_for_a_valid_field_row_shuffle(self):
        with tempfile.TemporaryDirectory() as temp:
            original = Path(temp) / "original"
            shuffled = Path(temp) / "shuffled"
            original.mkdir()
            shuffled.mkdir()
            write_export(original)
            write_export(shuffled)
            table = pq.read_table(shuffled / "fields.parquet")
            reverse = pa.array(range(table.num_rows - 1, -1, -1))
            pq.write_table(table.take(reverse), shuffled / "fields.parquet")
            self.assertEqual(observations.build(original), observations.build(shuffled))

    def test_round_balance_player_stays_null_without_the_owner_chain(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_export(root)
            table = pq.read_table(root / "fields.parquet")
            keep = pa.array([
                not (group.endswith("BombPlayerState_C") and name == "Owner")
                for group, name in zip(table.column("group_path").to_pylist(),
                                       table.column("field_name").to_pylist())
            ])
            pq.write_table(table.filter(keep), root / "fields.parquet")
            result = observations.build(root)

        self.assertTrue(all(row["player_state_guid"] is None
                            for row in result["round_balances"]))
        self.assertTrue(all(row["player_join_source"] == "unavailable"
                            for row in result["round_balances"]))
        self.assertEqual(result["attribution_coverage"]["round_balance_player"]["joined"], 0)

    def test_input_overwrite_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_export(root)
            for source_name in ("fields.parquet", "net_guids.parquet",
                                "events.parquet", "manifest.json"):
                with self.subTest(source_name=source_name):
                    with self.assertRaisesRegex(ValueError, "input export file"):
                        observations._reject_input_overwrite(root, root / source_name)


if __name__ == "__main__":
    unittest.main()
