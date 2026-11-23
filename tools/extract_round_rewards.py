#!/usr/bin/env python3
"""Derive round_rewards.parquet from an export: one row per TrackedRewards element.

`OwnerExclusivePlayerInfo.TrackedRewards` is the per-player list of economy
rewards of the current round. Its `Rewards` member is nine 32-bit windows the
overlay leaves raw; this view reads them from the parent array window
(`wire_bits.parse_array`), so it does not depend on the child rows the export
splits out (11.06 emits none). Each window is read as a little-endian int32.
Index 1 is credits; what the other indices mean is in docs/DATA.md.

A row is one element's life: it starts when a slot appears or its RewardName
changes, and ends when the array is truncated below it, the name changes, or
the export ends. Slots are kept per index, and each window updates only the
leaves it carries, as Unreal sends only changed properties.

The credits are checked, not assumed: at each `EndOfRoundBeforeRewardsMoney`
write E, the player's next `MoneyManagementComponent.Money` must equal
min(9000, E + the element credits paid at round end), and every round end is
counted into exactly one category below, zeros included. The player is joined
through the `Owner` value the two actors share. A child `Rewards` row that
differs from its parent window's leaf fails the run.
"""

from __future__ import annotations

import bisect
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path

import pyarrow as pa
import pyarrow.compute as pc

from extract_rounds import parquet_cli
from player_identity import PLAYER_STATE_GROUPS
from validate_type_evidence import Bits, fname
from wire_bits import InputError, iter_selected, parse_array, text

OEPI = "/Script/ShooterGame.OwnerExclusivePlayerInfo"
MONEY_GROUP = "/Script/ShooterGame.MoneyManagementComponent"
ARRAY = "TrackedRewards"
EORB = "EndOfRoundBeforeRewardsMoney"
PLAYERS = "AllPlayersObfuscatedPlayerInformation"
REWARDS_LEN = 9
CREDITS_INDEX = 1
#: The credit cap: every capped round end in the 60-export sample lands on it.
MAX_CREDITS = 9000
#: RewardGrantStrategy 1 is paid at round end (Kill credits, WinRound,
#: LoseRound, StrikeRecovery); 0 is paid when granted.
PAID_AT_ROUND_END = 1
#: Set the balance rather than add to it: 489 round ends land on exactly the
#: element's credits, whatever E was.
RESETS = frozenset({"GameStart", "OvertimeStart"})
#: AllPlayersObfuscatedPlayerInformation.ConnectionStatus that comes with
#: bIsAfk; 4 is the common value, and viewers disagree on 2 and 3.
DISCONNECTED = 1

#: More unexplained round ends than this share of the checked ones fails the
#: run: 0 of 8,624 on the 60-export sample, and over 94% when another
#: index or big-endian is read as credits.
MAX_UNEXPLAINED_PERCENT = 1

MEMBERS = ("RewardName", "InstancesOfReward", "RewardGrantStrategy", "Source")
SCHEMA = pa.schema([
    ("player_state_net_guid", pa.int64()), ("subject", pa.string()),
    ("info_net_guid", pa.int64()), ("slot", pa.int32()),
    ("first_ms", pa.int64()), ("last_ms", pa.int64()), ("end", pa.string()),
    ("reward_name", pa.string()), ("instances", pa.int64()),
    ("grant_strategy", pa.int32()), ("source", pa.int32()),
    ("credits", pa.int64()), ("rewards", pa.list_(pa.int64())),
])

ROUND_KEYS = (
    "round ends",
    "round ends: equal",
    "round ends: equal, a round-start grant added after E",
    "round ends: equal, the round-start grant already in E",
    "round ends: equal, reset to the GameStart/OvertimeStart credits",
    "round ends: of the equal, capped at 9000",
    "round ends: balance already the expected value, no Money row",
    "round ends: player not connected or AFK",
    "round ends: unexplained",
    "round ends: no Money at or after E",
    "round ends: an element without Rewards",
    "round ends: info not joined to a PlayerState",
)
COUNT_KEYS = (
    "TrackedRewards windows", "windows with a zero trailer", "Rewards leaves",
    "child Rewards rows", "child Rewards rows equal to their leaf", "parent leaves without a child row",
    "element lives", "lives ended by truncation", "lives ended by rename", "lives open at the end",
    "info actors", "info actors joined to a PlayerState", "players without a manifest subject",
    "SubjectUniqueId leaves not read", "child status rows equal to their leaf",
    *ROUND_KEYS,
)


def array_window(raw: bytes, bit_count: int):
    """parse_array, plus the optional zero IntPacked after the index
    terminator that the Rust walker accepts when exactly 8 bits remain."""
    try:
        return parse_array(raw, bit_count), False
    except InputError as exc:
        if str(exc) != "array has residual root bits" or bit_count < 16:
            raise
    value = int.from_bytes(raw, "little")
    if value >> (bit_count - 8):
        raise InputError("array has residual root bits")
    width = bit_count - 8
    return parse_array((value & ((1 << width) - 1)).to_bytes((width + 7) // 8, "little"), width), True


def leaf_int(payload: bytes, width: int, signed: bool) -> int:
    value = int.from_bytes(payload, "little") & ((1 << width) - 1)
    return value - (1 << width) if signed and value >> (width - 1) else value


def subject_unique_id(raw: bytes, bit_count: int) -> str:
    """An FUniqueNetIdRepl as replicated here: one byte, the type FString
    ("Riot"), the id FString; all bits consumed."""
    bits = Bits(raw, bit_count)
    bits.bits(8)
    bits.fstring()
    subject = bits.fstring()
    if bits.remaining():
        raise ValueError("SubjectUniqueId has residual bits")
    return subject


def _mask(batch):
    group, name = text(batch, "group_path"), text(batch, "field_name")
    ours = pc.and_(pc.equal(group, OEPI), pc.or_(
        pc.or_(pc.starts_with(name, ARRAY), pc.is_in(name, value_set=pa.array([EORB, "Owner"]))),
        pc.starts_with(name, PLAYERS)))
    state_owner = pc.and_(pc.is_in(group, value_set=pa.array(PLAYER_STATE_GROUPS)), pc.equal(name, "Owner"))
    money = pc.and_(pc.equal(group, MONEY_GROUP), pc.equal(name, "Money"))
    return pc.or_(pc.or_(ours, state_owner), money)


def _declarations(manifest: dict) -> dict[int, str]:
    for group in manifest.get("net_field_export_groups", []):
        if group["path"] == OEPI:
            return {f["handle"]: f["name"] for f in group["fields"]}
    return {}


def build(export: Path) -> tuple[list[dict], Counter, list[str]]:
    """`(rows, counts, problems)`; any problem means the rows are not trusted."""
    counts, problems = Counter(dict.fromkeys(COUNT_KEYS, 0)), []
    manifest = json.loads((export / "manifest.json").read_text(encoding="utf-8"))
    declared = _declarations(manifest)
    rewards = sorted(h for h, n in declared.items() if n == "Rewards")
    if declared and rewards != list(range(rewards[0], rewards[0] + REWARDS_LEN) if rewards else []):
        problems.append(f"Rewards is declared at handles {rewards}, not {REWARDS_LEN} consecutive ones")
        return [], counts, problems
    subjects = {p["actor_net_guid"]: p["subject"] for p in manifest.get("players", [])}

    windows, children = defaultdict(list), {}
    eorb, owners, state_owners, money = defaultdict(list), defaultdict(set), defaultdict(set), defaultdict(list)
    slot_subject, status, parent_status, child_status = {}, defaultdict(list), {}, []
    columns = ["time_ms", "packet_id", "actor_net_guid", "group_path", "field_name", "handle",
               "bit_count", "raw_bits", "value_i64", "value_bool"]
    for ordinal, row in iter_selected(export / "fields.parquet", columns, _mask):
        group, name, actor, t = row["group_path"], row["field_name"], row["actor_net_guid"], row["time_ms"]
        if group == MONEY_GROUP:
            if row["value_i64"] is not None:
                money[actor].append((t, row["packet_id"], ordinal, row["value_i64"]))
        elif group != OEPI:
            if row["value_i64"]:
                state_owners[row["value_i64"]].add(actor)
        elif name == ARRAY:
            windows[actor].append((t, row["packet_id"], ordinal, row["raw_bits"], row["bit_count"]))
        elif name == EORB:
            if row["value_i64"] is not None:
                eorb[actor].append((t, row["value_i64"]))
        elif name == "Owner":
            if row["value_i64"]:
                owners[actor].add(row["value_i64"])
        elif name == PLAYERS:
            (_, _, leaves), _ = array_window(row["raw_bits"], row["bit_count"])
            for index, handle, width, payload in leaves:
                member, key = declared.get(handle), (actor, index)
                if member == "SubjectUniqueId":
                    try:
                        slot_subject[key] = subject_unique_id(payload, width)
                    except ValueError:
                        counts["SubjectUniqueId leaves not read"] += 1
                elif member in ("ConnectionStatus", "bIsAfk"):
                    value = leaf_int(payload, width, signed=False)
                    status[key].append((t, member, value))
                    parent_status[(actor, row["packet_id"], t, index, member)] = value
        elif name.startswith(PLAYERS + "["):
            slot, _, member = name[len(PLAYERS) + 1:].partition("].")
            if member in ("ConnectionStatus", "bIsAfk"):
                value = row["value_i64"] if member == "ConnectionStatus" else row["value_bool"]
                child_status.append(((actor, row["packet_id"], t, int(slot), member), value))
        elif declared.get(row["handle"]) == "Rewards":
            counts["child Rewards rows"] += 1
            children[(actor, row["packet_id"], t, name.partition("]")[0], row["handle"])] = \
                (row["raw_bits"], row["bit_count"])

    for key, value in child_status:
        if key not in parent_status or parent_status[key] != int(value):
            problems.append(f"child {key[4]} row at {key[2]} ms differs from its parent leaf")
        else:
            counts["child status rows equal to their leaf"] += 1
    # A player's connection state: (time, member, value) from every viewer's slot naming them.
    by_subject = defaultdict(list)
    for key, events in status.items():
        if key in slot_subject:
            by_subject[slot_subject[key]].extend((t, key, member, value) for t, member, value in events)
    for events in by_subject.values():
        events.sort(key=lambda e: e[0])

    rows, base = [], rewards[0] if rewards else None
    counts["info actors"] = len(windows)
    for actor, actor_windows in windows.items():
        joined = set().union(*(state_owners[o] for o in owners[actor])) if owners[actor] else set()
        state = joined.pop() if len(joined) == 1 else None
        counts["info actors joined to a PlayerState"] += state is not None
        subject = subjects.get(state)
        if state is not None and subject is None:
            counts["players without a manifest subject"] += 1
        slots, snapshots = [], []

        def close(element, how):
            counts[f"lives {how}" if how != "open" else "lives open at the end"] += 1
            counts["element lives"] += 1
            rows.append({"player_state_net_guid": state, "subject": subject, "info_net_guid": actor,
                         "slot": element["slot"], "first_ms": element["first_ms"], "last_ms": element["last_ms"],
                         "end": how, "reward_name": element["RewardName"], "instances": element["InstancesOfReward"],
                         "grant_strategy": element["RewardGrantStrategy"], "source": element["Source"],
                         "credits": element["rewards"][CREDITS_INDEX], "rewards": list(element["rewards"])})

        for t, packet, _, raw, bit_count in sorted(actor_windows, key=lambda w: (w[0], w[1], w[2])):
            counts["TrackedRewards windows"] += 1
            (length, _, leaves), trailer = array_window(raw, bit_count)
            counts["windows with a zero trailer"] += trailer
            while len(slots) > length:
                close(slots.pop(), "ended by truncation")
            while len(slots) < length:
                slots.append(None)
            changed = defaultdict(dict)
            for index, handle, width, payload in leaves:
                member = declared.get(handle)
                if member == "Rewards":
                    counts["Rewards leaves"] += 1
                    if width != 32:
                        raise InputError(f"Rewards leaf of {width} bits at {t} ms")
                    child = children.pop((actor, packet, t, f"{ARRAY}[{index}", handle), None)
                    if child is None:
                        counts["parent leaves without a child row"] += 1
                    elif child != (payload, width):
                        problems.append(f"child Rewards row at {t} ms differs from its parent leaf")
                    else:
                        counts["child Rewards rows equal to their leaf"] += 1
                    changed[index][handle - base] = leaf_int(payload, width, signed=True)
                elif member == "RewardName":
                    bits = Bits(payload, width)
                    changed[index][member] = fname(bits)
                    if bits.remaining():
                        raise InputError(f"RewardName leaf has residual bits at {t} ms")
                elif member in MEMBERS:
                    changed[index][member] = leaf_int(payload, width, signed=member == "InstancesOfReward")
            for index, values in sorted(changed.items()):
                element = slots[index]
                if element is not None and "RewardName" in values and values["RewardName"] != element["RewardName"]:
                    close(element, "ended by rename")
                    # Unchanged leaves are not resent: the new life starts from the old values.
                    element = dict(element, rewards=list(element["rewards"]), first_ms=t)
                elif element is None:
                    element = {"slot": index, "first_ms": t, "rewards": [None] * REWARDS_LEN,
                               **dict.fromkeys(MEMBERS)}
                element["last_ms"] = t
                for key, value in values.items():
                    if isinstance(key, int):
                        element["rewards"][key] = value
                    else:
                        element[key] = value
                slots[index] = element
            snapshots.append((t, [dict(e, rewards=list(e["rewards"])) for e in slots if e is not None]))
        for element in slots:
            if element is not None:
                close(element, "open")
        _check_round_ends(counts, eorb.get(actor, []), snapshots, state,
                          money.get(state, []), by_subject.get(subject, []))

    if children:
        problems.append(f"{len(children)} child Rewards row(s) have no leaf in their parent window")
    checked = sum(counts[key] for key in ROUND_KEYS[1:9] if "of the equal" not in key)
    if counts["round ends: unexplained"] * 100 > checked * MAX_UNEXPLAINED_PERCENT:
        problems.append(f"{counts['round ends: unexplained']} of {checked} checked round ends are "
                        f"unexplained, over {MAX_UNEXPLAINED_PERCENT}%")
    return rows, counts, problems


def _disconnected_at(events, t) -> bool:
    """Whether any viewer's latest slot for the player says DISCONNECTED or
    bIsAfk at `t`. Viewers disagree on 2 and 3, so only these two count."""
    latest = {}
    for when, viewer, member, value in events:
        if when > t:
            break
        latest[(viewer, member)] = value
    return any(value == DISCONNECTED if member == "ConnectionStatus" else bool(value)
               for (_, member), value in latest.items())


def _check_round_ends(counts, writes, snapshots, state, money, connection):
    money = sorted(money)
    times = [s[0] for s in snapshots]
    for t, before_rewards in writes:
        counts["round ends"] += 1
        if state is None:
            counts["round ends: info not joined to a PlayerState"] += 1
            continue
        at = bisect.bisect_right(times, t)
        elements = snapshots[at - 1][1] if at else []
        if any(e["rewards"][CREDITS_INDEX] is None for e in elements):
            counts["round ends: an element without Rewards"] += 1
            continue
        i = bisect.bisect_left(money, (t,))
        if i == len(money):
            counts["round ends: no Money at or after E"] += 1
            continue
        after, previous = money[i][3], money[i - 1][3] if i else None
        paid = sum(e["rewards"][CREDITS_INDEX] for e in elements if e["RewardGrantStrategy"] == PAID_AT_ROUND_END)
        now = [e for e in elements if e["RewardGrantStrategy"] != PAID_AT_ROUND_END
               and e["first_ms"] == t and e["rewards"][CREDITS_INDEX]]
        resets = [e["rewards"][CREDITS_INDEX] for e in now if e["RewardName"] in RESETS]
        granted = sum(e["rewards"][CREDITS_INDEX] for e in now if e["RewardName"] not in RESETS)
        expected = min(MAX_CREDITS, before_rewards + paid)
        with_grant = min(MAX_CREDITS, expected + granted)
        if resets:
            key = "equal, reset to the GameStart/OvertimeStart credits" if after == resets[-1] else None
        elif after == expected:
            key = "equal, the round-start grant already in E" if granted else "equal"
        elif granted and after == with_grant:
            key = "equal, a round-start grant added after E"
        else:
            key = None
        if key is not None:
            counts[f"round ends: {key}"] += 1
            counts["round ends: of the equal, capped at 9000"] += after == MAX_CREDITS and not resets
            continue
        if previous in (expected, with_grant) and money[i][0] > t:
            counts["round ends: balance already the expected value, no Money row"] += 1
        elif _disconnected_at(connection, t):
            counts["round ends: player not connected or AFK"] += 1
        else:
            counts["round ends: unexplained"] += 1


def main(argv=None) -> int:
    return parquet_cli(__doc__, build, SCHEMA, "reward element(s)", argv)


if __name__ == "__main__":
    raise SystemExit(main())
