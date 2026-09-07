#!/usr/bin/env python3
"""Extract conservative, joined match observations from a vrfkit export.

This is an analytical view over the exported Parquet tables.  It deliberately
publishes observations and their evidence rather than declaring game metrics
where the replay cannot support one.  In particular, PurchasedItemComponent
rows are snapshots; only Money decreases are economic events.

Usage:
    python tools/extract_match_observations.py --export out/<replay> --out observations.json
"""

from __future__ import annotations

import argparse
import bisect
import json
import re
from collections import defaultdict
from pathlib import Path

import pyarrow as pa
import pyarrow.compute as pc
import pyarrow.parquet as pq

try:
    from .atomic_io import atomic_write_text
except ImportError:
    from atomic_io import atomic_write_text


MAGAZINE_PATH = "MagazineAmmo"
SHOT_EFFECT_ID = "ReplayPlayContinuousEffectAtLocation.EffectID"
ROUND_MONEY_RE = re.compile(r"RoundInfos\[(\d+)\]\.(StartOfRoundMoney|EndOfRoundMoney)$")
TEAM_ECONOMY_RE = re.compile(
    r"TeamEconomy\[(\d+)\]\.(LoadoutValue|AverageLoadoutValue)$"
)
PURCHASE_FIELDS = {
    "PurchasingPlayerState",
    "Purchaseable",
    "PurchasableTransactionSource",
}


def _collapse(samples):
    """Keep value changes from one replicated scalar stream.

    Source row ordinal breaks real `(time_ms, packet_id)` ties. A packet that
    carries conflicting values has no observable wire ordering in this table,
    so it becomes an explicit unknown boundary rather than being sorted by
    value and turned into an invented transition.
    """
    result = []
    ambiguous = 0
    previous = object()
    ordered = sorted(samples, key=lambda row: row[:3])
    index = 0
    while index < len(ordered):
        time_ms, packet_id, _, value = ordered[index]
        end = index + 1
        values = {value}
        while end < len(ordered) and ordered[end][:2] == (time_ms, packet_id):
            values.add(ordered[end][3])
            end += 1
        if len(values) != 1:
            result.append((time_ms, packet_id, None))
            previous = object()
            ambiguous += 1
        elif value != previous:
            result.append((time_ms, packet_id, value))
            previous = value
        index = end
    return result, ambiguous


def _changes(samples):
    """Return `(time, previous, value, delta)` after replication collapse."""
    values, ambiguous = _collapse(samples)
    return [
        (time_ms, before, after, after - before)
        for (_, _, before), (time_ms, _, after) in zip(values, values[1:])
        if before is not None and after is not None
    ], ambiguous


def _round_context(round_starts: list[int], time_ms: int) -> int | None:
    """Zero-based replay round ordinal, or null before the first round event."""
    index = bisect.bisect_right(round_starts, time_ms) - 1
    return index if index >= 0 else None


def _timeline(samples):
    """Return replication-collapsed `(time, packet)` keys and values."""
    compact, _ = _collapse(samples)
    return [(sample[0], sample[1]) for sample in compact], [sample[2] for sample in compact]


def _value_at(timeline, time_ms: int, packet_id: int):
    """Return the last known scalar value at a packet, preserving unknowns."""
    keys, values = timeline
    index = bisect.bisect_right(keys, (time_ms, packet_id)) - 1
    return values[index] if index >= 0 else None


def _reject_input_overwrite(export_dir: Path, output_path: Path) -> None:
    """Refuse to overwrite any source Parquet or manifest with JSON output."""
    source = export_dir.resolve()
    output = output_path.resolve()
    inputs = {path.resolve() for path in source.glob("*.parquet")}
    inputs.add((source / "manifest.json").resolve())
    if output in inputs:
        raise ValueError(f"output path is an input export file: {output}")


def _read_columns(path: Path, columns: list[str], *, observation_fields=False) -> dict[str, list]:
    table = pq.read_table(path, columns=columns)
    if observation_fields:
        # Most physical rows are movement/input payloads. Filter while Arrow
        # still owns the columns, before expanding millions of Python objects.
        # Row order within the retained fields remains unchanged.
        names = pc.cast(table.column("field_name"), pa.string())
        scalar_names = pa.array(sorted(PURCHASE_FIELDS | {
            "AuthResourceAmount", "CurrentEquippable", "NewCurrentEquippable",
            "CurrentState", "Money", "DefuseProgress", "LoadoutValue",
            "AverageLoadoutValue", "Owner", SHOT_EFFECT_ID,
        }))
        indexed = pc.match_substring_regex(
            names, "^(?:" + ROUND_MONEY_RE.pattern + "|" + TEAM_ECONOMY_RE.pattern + ")"
        )
        table = table.filter(pc.fill_null(pc.or_kleene(
            pc.is_in(names, value_set=scalar_names), indexed), False))
    return {name: table.column(name).to_pylist() for name in columns}


def _near(sorted_times: list[int], time_ms: int, window_ms: int) -> bool:
    position = bisect.bisect_left(sorted_times, time_ms - window_ms)
    return position < len(sorted_times) and sorted_times[position] <= time_ms + window_ms


def _stable_rows(rows: list[dict]) -> list[dict]:
    """Sort output independently of Parquet's physical row order."""
    return sorted(
        rows,
        key=lambda row: (
            row.get("time_ms", row.get("from_ms")),
            json.dumps(row, sort_keys=True, separators=(",", ":")),
        ),
    )


def build(export_dir: Path) -> dict:
    """Build evidence-labelled observations for one export directory."""
    required = ("fields.parquet", "net_guids.parquet", "events.parquet")
    missing = [name for name in required if not (export_dir / name).is_file()]
    if missing:
        raise ValueError(f"missing required export tables: {', '.join(missing)}")

    net = _read_columns(export_dir / "net_guids.parquet",
                        ["net_guid", "path", "outer_net_guid"])
    path_of = {guid: path for guid, path in zip(net["net_guid"], net["path"])
               if path}
    outer_of = {guid: outer for guid, outer in zip(net["net_guid"], net["outer_net_guid"])
                if outer is not None}

    events = _read_columns(export_dir / "events.parquet", ["group", "time1"])
    round_starts = sorted(time for group, time in zip(events["group"], events["time1"])
                          if group == "roundStarted")
    defuse_events = sorted(time for group, time in zip(events["group"], events["time1"])
                           if group == "spikeDefused")

    fields = _read_columns(
        export_dir / "fields.parquet",
        ["time_ms", "packet_id", "actor_net_guid", "object_net_guid",
         "group_path", "field_name", "value_i64", "value_f64"],
        observation_fields=True,
    )

    magazine = defaultdict(list)
    inventory = defaultdict(list)
    state_machine = defaultdict(list)
    money = defaultdict(list)
    defuse_progress = defaultdict(list)
    balances = defaultdict(list)
    team_samples = defaultdict(list)
    purchase_updates = defaultdict(list)
    owner_info_controllers = defaultdict(list)
    player_controllers = defaultdict(list)
    shot_times = []

    for row in range(len(fields["time_ms"])):
        time_ms = fields["time_ms"][row]
        packet_id = fields["packet_id"][row]
        actor = fields["actor_net_guid"][row]
        obj = fields["object_net_guid"][row]
        group = fields["group_path"][row] or ""
        name = fields["field_name"][row]
        integer = fields["value_i64"][row]
        floating = fields["value_f64"][row]

        if group.endswith("AmmoComponent") and name == "AuthResourceAmount" and integer is not None:
            if path_of.get(obj) == MAGAZINE_PATH:
                magazine[obj].append((time_ms, packet_id, row, integer))
        elif group.endswith("AresInventory") and name in ("CurrentEquippable", "NewCurrentEquippable"):
            if integer is not None:
                # Current and New are distinct replicated fields. Their order
                # inside a packet is not a documented transition order.
                # AresInventory replicates as the object (component) below
                # its containing actor. Both domains are retained; using the
                # actor as the component would make an outer join point at
                # the wrong level.
                inventory[(obj, actor, name)].append((time_ms, packet_id, row, integer))
        elif group.endswith("EquippableStateMachineComponent") and name == "CurrentState":
            if integer is not None:
                # The component is the object being replicated. Actor is its
                # containing equippable in the observed exports and can own
                # several state machines, so it cannot identify one scalar
                # state stream.
                state_machine[obj].append((time_ms, packet_id, row, integer))
        elif group.endswith("MoneyManagementComponent") and name == "Money" and integer is not None:
            money[obj].append((time_ms, packet_id, row, integer))
        elif group.endswith("TimedBomb_C") and name == "DefuseProgress":
            value = floating if floating is not None else integer
            if value is not None:
                defuse_progress[actor].append((time_ms, packet_id, row, value))
        elif integer is not None and (match := ROUND_MONEY_RE.fullmatch(name or "")):
            slot, value_kind = match.groups()
            balances[(actor, int(slot), value_kind)].append((time_ms, packet_id, row, integer))
        elif group.endswith("OwnerExclusivePlayerInfo") and name == "Owner" and integer:
            # The descriptor identifies Owner as the owning controller.
            owner_info_controllers[actor].append((time_ms, packet_id, row, integer))
        elif group.endswith("BombPlayerState_C") and name == "Owner" and integer:
            # BombPlayerState.Owner gives the inverse controller -> player
            # state link needed by OwnerExclusivePlayerInfo.
            player_controllers[actor].append((time_ms, packet_id, row, integer))
        elif group.endswith("BaseTeamState") and name in ("LoadoutValue", "AverageLoadoutValue"):
            if integer is not None:
                team_samples[(
                    actor, "BaseTeamState", None, time_ms, packet_id, name
                )].append(integer)
        elif integer is not None and (match := TEAM_ECONOMY_RE.fullmatch(name or "")):
            slot, value_kind = match.groups()
            team_samples[(
                actor, "BombGameState.TeamEconomy", int(slot), time_ms, packet_id, value_kind
            )].append(integer)
        elif group.endswith("PurchasedItemComponent") and name in PURCHASE_FIELDS:
            purchase_updates[(actor, obj)].append((time_ms, packet_id, row, name, integer))
        elif (group.endswith("ReplayEffectComponent_ClassNetCache")
              and name == SHOT_EFFECT_ID):
            shot_times.append(time_ms)

    shot_times.sort()
    owner_info_controller_timelines = {
        info: _timeline(samples) for info, samples in owner_info_controllers.items()
    }
    player_controller_timelines = {
        player: _timeline(samples) for player, samples in player_controllers.items()
    }
    team_values = defaultdict(lambda: defaultdict(dict))
    ambiguous_team_loadout_packets = 0
    for (team_guid, source, slot, time_ms, packet_id, value_kind), samples in sorted(team_samples.items()):
        values = set(samples)
        if len(values) == 1:
            value = values.pop()
        else:
            value = None
            ambiguous_team_loadout_packets += 1
        team_values[(team_guid, source, slot)][(time_ms, packet_id)][value_kind] = value
    ammo_changes = []
    ambiguous_ammo_packets = 0
    for component, samples in magazine.items():
        weapon = outer_of.get(component)
        changes, ambiguous = _changes(samples)
        ambiguous_ammo_packets += ambiguous
        for time_ms, before, after, delta in changes:
            if not delta:
                continue
            ammo_changes.append({
                "time_ms": time_ms,
                "magazine_component_guid": component,
                "weapon_guid": weapon,
                "before": before,
                "after": after,
                "delta": delta,
                "kind": "decrease" if delta < 0 else "increase",
                "round_start_within_150ms": _near(round_starts, time_ms, 150),
                # This is deliberately global timing evidence. The EffectID
                # row has no demonstrated weapon join at this seam.
                "global_effect_id_within_300ms": (
                    _near(shot_times, time_ms, 300) if delta < 0 else None
                ),
            })

    equip_intervals = []
    ambiguous_inventory_packets = 0
    for (inventory_component, inventory_actor, source_field), samples in inventory.items():
        compact, ambiguous = _collapse(samples)
        ambiguous_inventory_packets += ambiguous
        for index, (start_ms, _, weapon) in enumerate(compact):
            if weapon is None:
                continue
            end_ms = compact[index + 1][0] if index + 1 < len(compact) else None
            equip_intervals.append({
                "inventory_component_guid": inventory_component,
                "inventory_actor_guid": inventory_actor,
                "owner_guid": outer_of.get(inventory_component),
                "source_field": source_field,
                "weapon_guid": weapon,
                "from_ms": start_ms,
                "to_ms": end_ms,
                "duration_ms": end_ms - start_ms if end_ms is not None else None,
            })

    reload_intervals = []
    ambiguous_reload_packets = 0
    for component, samples in state_machine.items():
        compact, ambiguous = _collapse(samples)
        ambiguous_reload_packets += ambiguous
        open_start = None
        open_state = None
        for time_ms, _, state in compact:
            if state is None:
                if open_start is not None:
                    reload_intervals.append({
                        "state_machine_guid": component,
                        "weapon_guid": outer_of.get(component),
                        "state_guid": open_state,
                        "from_ms": open_start,
                        "to_ms": time_ms,
                        "duration_ms": None,
                        "closed_by_state_change": False,
                        "end_boundary": "ambiguous_same_packet",
                    })
                    open_start = open_state = None
                continue
            is_reload = (path_of.get(state) or "").endswith("ReloadState")
            if is_reload and open_start is None:
                open_start, open_state = time_ms, state
            elif not is_reload and open_start is not None:
                reload_intervals.append({
                    "state_machine_guid": component,
                    "weapon_guid": outer_of.get(component),
                    "state_guid": open_state,
                    "from_ms": open_start,
                    "to_ms": time_ms,
                    "duration_ms": time_ms - open_start,
                    "closed_by_state_change": True,
                    "end_boundary": "state_change",
                })
                open_start = open_state = None
        if open_start is not None:
            reload_intervals.append({
                "state_machine_guid": component,
                "weapon_guid": outer_of.get(component),
                "state_guid": open_state,
                "from_ms": open_start,
                "to_ms": None,
                "duration_ms": None,
                "closed_by_state_change": False,
                "end_boundary": "end_of_stream",
            })

    progress_transitions = []
    ambiguous_defuse_packets = 0
    for bomb_guid, samples in defuse_progress.items():
        changes, ambiguous = _changes(samples)
        ambiguous_defuse_packets += ambiguous
        for time_ms, before, after, delta in changes:
            if not delta:
                continue
            progress_transitions.append({
                "time_ms": time_ms,
                "bomb_guid": bomb_guid,
                "before_seconds": before,
                "after_seconds": after,
                "delta_seconds": delta,
                "kind": "increase" if delta > 0 else "decrease",
                # Completion is represented below by the replay event. A
                # progress threshold is deliberately never promoted to truth.
                "authoritative_completion": False,
            })

    round_balances = []
    ambiguous_balance_packets = 0
    for (info_guid, slot, value_kind), samples in balances.items():
        compact, ambiguous = _collapse(samples)
        ambiguous_balance_packets += ambiguous
        for time_ms, packet_id, value in compact:
            if value is not None:
                controller = _value_at(
                    owner_info_controller_timelines.get(info_guid, ([], [])), time_ms, packet_id
                )
                players = [
                    player for player, timeline in player_controller_timelines.items()
                    if controller is not None and _value_at(timeline, time_ms, packet_id) == controller
                ]
                player = players[0] if len(players) == 1 else None
                join_source = (
                    "OwnerExclusivePlayerInfo.Owner@time -> BombPlayerState.Owner@time"
                    if player is not None else "unavailable"
                )
                round_balances.append({
                    "time_ms": time_ms,
                    "packet_id": packet_id,
                    "owner_exclusive_info_guid": info_guid,
                    "owner_controller_guid": controller,
                    "player_state_guid": player,
                    "player_join_source": join_source,
                    "round_info_slot": slot,
                    "value_kind": value_kind,
                    "money": value,
                })

    team_loadouts = []
    for (team_guid, source, slot), by_time in team_values.items():
        for (time_ms, _), values in sorted(by_time.items()):
            total = values.get("LoadoutValue")
            average = values.get("AverageLoadoutValue")
            team_loadouts.append({
                "time_ms": time_ms,
                "team_state_guid": team_guid,
                "team_slot": slot,
                "loadout_value": total,
                "average_loadout_value": average,
                "average_times_five_matches_total": (
                    average * 5 == total if average is not None and total is not None else None
                ),
                "source": source,
            })

    money_decreases = []
    ambiguous_money_packets = 0
    for component, samples in money.items():
        player = outer_of.get(component)
        changes, ambiguous = _changes(samples)
        ambiguous_money_packets += ambiguous
        for time_ms, before, after, delta in changes:
            if delta < 0:
                money_decreases.append({
                    "time_ms": time_ms,
                    "money_component_guid": component,
                    "player_state_guid": player,
                    "before": before,
                    "after": after,
                    "amount": -delta,
                    "evidence": "MoneyManagementComponent.Money decrease",
                })

    money_by_player = defaultdict(list)
    for event in money_decreases:
        if event["player_state_guid"] is not None:
            money_by_player[event["player_state_guid"]].append(event)
    transaction_snapshots = []
    ambiguous_purchase_packets = 0
    for (actor, component), updates in purchase_updates.items():
        state = {}
        previous_pair = object()
        ordered = sorted(updates)
        index = 0
        while index < len(ordered):
            time_ms, packet_id = ordered[index][:2]
            end = index + 1
            while end < len(ordered) and ordered[end][:2] == (time_ms, packet_id):
                end += 1
            packet_updates = ordered[index:end]
            # Apply every source-field update in row order. A field that has
            # two values inside one packet is ambiguous, so leave it out of
            # the forward-filled state instead of selecting a numeric winner.
            names = {update[3] for update in packet_updates}
            for name in sorted(names):
                values = {update[4] for update in packet_updates if update[3] == name}
                if len(values) != 1:
                    ambiguous_purchase_packets += 1
                    state.pop(name, None)
                else:
                    state[name] = values.pop()
            buyer = state.get("PurchasingPlayerState")
            item = state.get("Purchaseable")
            pair = (buyer, item)
            if buyer is None or item is None:
                previous_pair = object()
            elif pair != previous_pair:
                previous_pair = pair
                nearby = [event for event in money_by_player.get(buyer, [])
                          if abs(event["time_ms"] - time_ms) <= 2000]
                nearest = min(nearby, key=lambda event: abs(event["time_ms"] - time_ms), default=None)
                transaction_snapshots.append({
                    "time_ms": time_ms,
                    "source_packet_id": packet_id,
                    "round_ordinal": _round_context(round_starts, time_ms),
                    "purchased_item_component_actor_guid": actor,
                    "purchased_item_component_object_guid": component,
                    "purchasing_player_state_guid": buyer,
                    "purchaseable_guid": item,
                    "transaction_source": state.get("PurchasableTransactionSource"),
                    "nearby_money_decrease_count_2s": len(nearby),
                    "nearest_money_decrease_ms": nearest["time_ms"] if nearest else None,
                    "nearest_money_decrease_amount": nearest["amount"] if nearest else None,
                    "evidence": "PurchasedItemComponent state transition; not a purchase ledger",
                })
            index = end

    observations = {
        "schema_version": 1,
        "evidence_rules": {
        "scalar_dedup": "consecutive equal values collapse by time_ms, packet_id",
            "same_packet_conflict": "conflicting values become unknown boundaries, never value-sorted transitions",
            "ammo_shot_join": "decrease carries only a global EffectID observation within +/-300ms",
            "defuse_completion": "events.spikeDefused is authoritative; progress is never completion",
            "purchase": "PurchasedItemComponent rows are snapshots; Money decreases are separate events",
        },
        "ammo_changes": _stable_rows(ammo_changes),
        "equip_intervals": _stable_rows(equip_intervals),
        "reload_intervals": _stable_rows(reload_intervals),
        "defuse_progress_transitions": _stable_rows(progress_transitions),
        "defuse_completions": [
            {"time_ms": time_ms, "source": "events.spikeDefused", "authoritative": True}
            for time_ms in defuse_events
        ],
        "round_balances": _stable_rows(round_balances),
        "team_loadouts": _stable_rows(team_loadouts),
        "money_decreases": _stable_rows(money_decreases),
        "transaction_snapshots": _stable_rows(transaction_snapshots),
        "attribution_coverage": {
            "equip_owner_outer": {
                "joined": sum(row["owner_guid"] is not None for row in equip_intervals),
                "total": len(equip_intervals),
                "source": "net_guids.outer_net_guid(InventoryComponent)",
            },
            "round_balance_player": {
                "joined": sum(row["player_state_guid"] is not None for row in round_balances),
                "total": len(round_balances),
                "source": "OwnerExclusivePlayerInfo.Owner@time -> BombPlayerState.Owner@time",
            },
        },
        "ambiguous_same_packet_counts": {
            "ammo": ambiguous_ammo_packets,
            "inventory": ambiguous_inventory_packets,
            "reload": ambiguous_reload_packets,
            "defuse": ambiguous_defuse_packets,
            "round_balance": ambiguous_balance_packets,
            "team_loadout": ambiguous_team_loadout_packets,
            "money": ambiguous_money_packets,
            "purchased_item": ambiguous_purchase_packets,
        },
    }
    observations["quality_gaps"] = [
        "RoundInfos money is unavailable in this export" if not round_balances else None,
        "Team loadout values are unavailable in this export" if not team_loadouts else None,
        "No firing EffectID observations were present" if not shot_times else None,
        "No authoritative spikeDefused events were present" if not defuse_events else None,
    ]
    observations["quality_gaps"] = [gap for gap in observations["quality_gaps"] if gap]
    return observations


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--export", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    _reject_input_overwrite(args.export, args.out)
    result = build(args.export)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    atomic_write_text(args.out, json.dumps(result, indent=2) + "\n")
    print(f"wrote {args.out}")
    for name in ("ammo_changes", "equip_intervals", "reload_intervals",
                 "defuse_progress_transitions", "defuse_completions",
                 "round_balances", "team_loadouts", "money_decreases",
                 "transaction_snapshots"):
        print(f"  {name}: {len(result[name])}")
    for gap in result["quality_gaps"]:
        print(f"  quality gap: {gap}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
