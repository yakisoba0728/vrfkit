"""`extract_round_rewards.py`: Rewards read from the parent array window and
checked against the player's Money at each round end."""
from __future__ import annotations

import json
import struct

import pyarrow as pa
import pyarrow.parquet as pq

from support import TempDirTestCase, read_parquet, run_cli
import extract_round_rewards as rr

OEPI, MONEY, STATE = rr.OEPI, rr.MONEY_GROUP, rr.PLAYER_STATE_GROUPS[0]
H = {"Owner": 11, "TrackedRewards": 18, "RewardName": 28, "InstancesOfReward": 30,
     "RewardGrantStrategy": 31, "Source": 32, rr.EORB: 34, rr.PLAYERS: 47,
     "SubjectUniqueId": 48, "bIsAfk": 49, "ConnectionStatus": 50}
REWARDS = list(range(19, 28))
INFO, STATE_GUID, CONTROLLER, SUBJECT = 900, 500, 206, "s" * 36


class Writer:
    """LSB-first bit writer."""

    def __init__(self):
        self.value, self.width = 0, 0

    def put(self, value, width):
        self.value |= (value & ((1 << width) - 1)) << self.width
        self.width += width
        return self

    def packed(self, value):
        while True:
            more = value >> 7
            self.put((value & 0x7F) << 1 | bool(more), 8)
            if not more:
                return self
            value = more

    def fstring(self, text):
        data = text.encode() + b"\0"
        self.put(len(data), 32)
        for byte in data:
            self.put(byte, 8)
        return self

    def window(self):
        return self.value.to_bytes((self.width + 7) // 8, "little"), self.width


def leaf(width, value):
    return Writer().put(value, width).window()


def fname_leaf(name):
    return Writer().put(0, 1).fstring(name).put(0, 32).window()


def array(length, elements, *, trailer=False):
    """An array window: length, then (index + 1, (handle + 1, width, payload)..., 0)..., 0."""
    w = Writer().packed(length)
    for index, leaves in elements:
        w.packed(index + 1)
        for handle, (payload, width) in leaves:
            w.packed(handle + 1).packed(width)
            w.put(int.from_bytes(payload, "little"), width)
        w.packed(0)
    w.packed(0)
    if trailer:
        w.packed(0)
    return w.window()


def element(name, strategy, credits, *, values=None, instances=1, source=0):
    rewards = values or {1: credits}
    leaves = [(h, leaf(32, rewards.get(i, 0) & 0xFFFFFFFF)) for i, h in enumerate(REWARDS)]
    return leaves + [(H["RewardName"], fname_leaf(name)), (H["InstancesOfReward"], leaf(32, instances)),
                     (H["RewardGrantStrategy"], leaf(2, strategy)), (H["Source"], leaf(3, source))]


def row(t, group, name, *, actor=INFO, handle=0, window=None, i64=None, boolean=None, packet=None):
    raw, bits = window if window else (None, 0)
    return {"time_ms": t, "packet_id": t if packet is None else packet, "actor_net_guid": actor,
            "group_path": group, "field_name": name, "handle": handle, "bit_count": bits,
            "raw_bits": raw, "value_i64": i64, "value_bool": boolean}


def owners():
    return [row(1, OEPI, "Owner", handle=H["Owner"], i64=CONTROLLER),
            row(1, STATE, "Owner", actor=STATE_GUID, i64=CONTROLLER)]


def rewards_window(t, length, elements, **kw):
    return row(t, OEPI, "TrackedRewards", handle=H["TrackedRewards"], window=array(length, elements, **kw))


def money(t, value):
    return row(t, MONEY, "Money", actor=STATE_GUID, i64=value)


def end_of_round(t, value):
    return row(t, OEPI, rr.EORB, handle=H[rr.EORB], i64=value)


def players_window(t, status, afk):
    subject = Writer().put(0xF8, 8).fstring("Riot").fstring(SUBJECT).window()
    leaves = [(H["SubjectUniqueId"], subject), (H["bIsAfk"], leaf(1, afk)), (H["ConnectionStatus"], leaf(3, status))]
    return row(t, OEPI, rr.PLAYERS, handle=H[rr.PLAYERS], window=array(1, [(0, leaves)]))


def won_round(t0=1000, *, money_after=3800, before=800, values=None):
    """A round: a WinRound element paid at round end, then E and Money."""
    return [rewards_window(t0, 1, [(0, element("WinRound", 1, 3000, values=values))]),
            end_of_round(t0 + 7000, before), money(t0 + 7000, money_after),
            rewards_window(t0 + 7100, 0, [])]


class ExtractRoundRewardsTests(TempDirTestCase):
    def setUp(self):
        self.export = self.tmp()

    def write(self, rows, *, rewards=REWARDS):
        fields = [{"handle": h, "name": n, "compatible_checksum": 0} for n, h in H.items()]
        fields += [{"handle": h, "name": "Rewards", "compatible_checksum": 0} for h in rewards]
        (self.export / "manifest.json").write_text(json.dumps({
            "players": [{"actor_net_guid": STATE_GUID, "subject": SUBJECT}],
            "net_field_export_groups": [{"path": OEPI, "fields": fields}]}), encoding="utf-8")
        schema = pa.schema([("time_ms", pa.uint32()), ("packet_id", pa.uint32()), ("actor_net_guid", pa.uint32()),
                            ("group_path", pa.string()), ("field_name", pa.string()), ("handle", pa.uint32()),
                            ("bit_count", pa.uint32()), ("raw_bits", pa.binary()), ("value_i64", pa.int64()),
                            ("value_bool", pa.bool_())])
        pq.write_table(pa.Table.from_pylist([*owners(), *rows], schema=schema), self.export / "fields.parquet")
        return self.export

    def build(self, rows, **kw):
        return rr.build(self.write(rows, **kw))

    def test_credits_are_the_little_endian_second_window_and_the_round_end_agrees(self):
        rows, counts, problems = self.build(won_round(values={1: 3000, 4: 2, 5: -1}))
        self.assertEqual(problems, [])
        self.assertEqual([(r["reward_name"], r["credits"], r["grant_strategy"], r["end"], r["subject"])
                          for r in rows], [("WinRound", 3000, 1, "ended by truncation", SUBJECT)])
        self.assertEqual(rows[0]["rewards"], [0, 3000, 0, 0, 2, -1, 0, 0, 0])
        self.assertEqual(counts["round ends: equal"], 1)
        self.assertEqual(counts["child Rewards rows"], 0)
        self.assertEqual(counts["parent leaves without a child row"], 9)

    def test_a_money_balance_off_the_rewards_is_unexplained_and_fails_the_run(self):
        out = self.tmp() / "round_rewards.parquet"
        self.write(won_round(money_after=3801))
        code, stdout, stderr = run_cli(rr.main, "--export", self.export, "--out", out)
        self.assertEqual(code, 1)
        self.assertIn("round ends: unexplained: 1", stdout)
        self.assertIn("1 of 1 checked round ends are unexplained", stderr)
        self.assertFalse(out.exists())

    def test_the_cap_round_start_grants_and_resets_are_their_own_categories(self):
        rounds = [
            *won_round(1000, before=8000, money_after=9000),
            # A round-start grant written with E is paid on top of it.
            # A plant bonus is paid when granted, so E already holds it.
            rewards_window(20000, 2, [(0, element("LoseRound", 1, 1900)),
                                      (1, element("SP1-K3 Inititated", 0, 300, source=3))]),
            rewards_window(27000, 3, [(2, element("RoundNumStart", 0, 2400))]),
            end_of_round(27000, 100), money(27000, 4400), rewards_window(27100, 0, []),
            # A side switch resets the balance to GameStart's credits.
            rewards_window(40000, 1, [(0, element("LoseRound", 1, 1900))]),
            rewards_window(47000, 2, [(1, element("GameStart", 0, 800))]),
            end_of_round(47000, 800), money(47000, 800), rewards_window(47100, 0, []),
        ]
        rows, counts, problems = self.build(rounds)
        self.assertEqual(problems, [])
        self.assertEqual({k: counts[k] for k in rr.ROUND_KEYS if counts[k]}, {
            "round ends": 3, "round ends: equal": 1, "round ends: of the equal, capped at 9000": 1,
            "round ends: equal, a round-start grant added after E": 1,
            "round ends: equal, reset to the GameStart/OvertimeStart credits": 1})
        self.assertEqual(sorted(r["reward_name"] for r in rows),
                         ["GameStart", "LoseRound", "LoseRound", "RoundNumStart", "SP1-K3 Inititated", "WinRound"])

    def test_an_unchanged_balance_and_a_disconnected_player_are_named_not_equal(self):
        rows, counts, problems = self.build([
            money(10, 3800), *won_round(1000, money_after=3500)[:2], rewards_window(8100, 0, []),
            money(9000, 3500),
            players_window(10000, rr.DISCONNECTED, 1), *won_round(20000, money_after=800)])
        self.assertEqual(problems, [])
        self.assertEqual(counts["round ends: balance already the expected value, no Money row"], 1)
        self.assertEqual(counts["round ends: player not connected or AFK"], 1)
        self.assertEqual(counts["round ends: unexplained"], 0)

    def test_a_connected_player_off_the_rewards_stays_unexplained_unless_afk(self):
        for afk, named in ((0, 0), (1, 1)):
            _, counts, _ = self.build([players_window(10, 4, afk), *won_round(money_after=800)])
            self.assertEqual(counts["round ends: player not connected or AFK"], named)
            self.assertEqual(counts["round ends: unexplained"], 1 - named)

    def test_a_money_row_written_at_the_round_end_is_not_an_unchanged_balance(self):
        _, counts, _ = self.build([money(10, 3800), *won_round(money_after=3500)])
        self.assertEqual(counts["round ends: balance already the expected value, no Money row"], 0)
        self.assertEqual(counts["round ends: unexplained"], 1)

    def test_a_child_row_must_carry_its_parent_leaf(self):
        rounds = won_round()
        for value, problem in ((3000, False), (2999, True)):
            child = row(1000, OEPI, "TrackedRewards[0].Rewards", handle=REWARDS[1], window=leaf(32, value))
            _, counts, problems = self.build([*rounds, child])
            self.assertEqual(bool(problems), problem, problems)
            self.assertEqual(counts["child Rewards rows equal to their leaf"], 0 if problem else 1)
        _, _, problems = self.build([*rounds, row(5, OEPI, "TrackedRewards[0].Rewards", handle=REWARDS[1],
                                                  window=leaf(32, 3000))])
        self.assertIn("1 child Rewards row(s) have no leaf in their parent window", problems)

    def test_a_rename_ends_a_life_and_keeps_the_leaves_it_did_not_resend(self):
        rows, counts, _ = self.build([
            rewards_window(1000, 1, [(0, element("Kill", 1, 200))]),
            rewards_window(2000, 1, [(0, [(H["RewardName"], fname_leaf("Death"))])]),
            rewards_window(3000, 1, [], trailer=True)])
        self.assertEqual([(r["reward_name"], r["credits"], r["first_ms"], r["end"]) for r in rows],
                         [("Kill", 200, 1000, "ended by rename"), ("Death", 200, 2000, "open")])
        self.assertEqual((counts["windows with a zero trailer"], counts["lives ended by rename"],
                          counts["lives open at the end"]), (1, 1, 1))

    def test_a_trailer_that_is_not_zero_is_refused(self):
        raw, bits = array(0, [])
        self.assertEqual(rr.array_window(raw, bits), ((0, 0, []), False))
        bad = Writer().packed(0).packed(0).packed(1).window()
        with self.assertRaises(rr.InputError):
            rr.array_window(*bad)

    def test_rewards_must_be_nine_consecutive_declarations(self):
        rows, _, problems = self.build(won_round(), rewards=[19, 20, 21, 22, 23, 24, 25, 26, 28])
        self.assertEqual(rows, [])
        self.assertIn("not 9 consecutive", problems[0])

    def test_the_cli_prints_every_count_and_writes_the_view(self):
        out = self.tmp() / "round_rewards.parquet"
        self.write(won_round())
        code, stdout, _ = run_cli(rr.main, "--export", self.export, "--out", out)
        self.assertEqual(code, 0)
        for key in rr.COUNT_KEYS:
            self.assertIn(f"  {key}: ", stdout)
        self.assertIn("round ends: unexplained: 0", stdout)
        table = read_parquet(out)
        self.assertEqual(table.schema, rr.SCHEMA)
        self.assertEqual(table.column("credits").to_pylist(), [3000])


if __name__ == "__main__":
    import unittest
    unittest.main()
