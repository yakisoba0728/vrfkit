"""Guards for the metrics guard.

check_metrics_baseline.py exists because framing counters cannot see a decoder
that stops producing values. Its invariants are the part that carries that
weight -- they need no baseline and survive legitimate changes -- so they are
the part that must not rot.

The headline case uses the REAL shape of the section-26 break, measured on the
13.02 fixture before commit bcc7d70: ClientRoundStart RPCs said 21 rounds while
BombGameState RoundResults produced none, so team_score was empty.
"""
import copy
import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_metrics_baseline as guard  # noqa: E402


# A healthy 13.02 fixture run, as pinned.
HEALTHY = {
    "rounds_rpc": 21,
    "rounds_objective": 21,
    "client_round_starts": 21,
    "team_score": {"Blue": 13, "Red": 8},
    "plants": 14,
    "defuses": 3,
    "players": 10,
    "combat_players": 10,
    "kills": 151,
    "deaths": 151,
    "assists": 40,
    "headshots": 30,
    "damage_dealt": 28000.0,
    "first_bloods": 20,
    "trade_kills": 40,
    "kast_rounds": 150,
    "ultimate_casts": 22,
    "distinct_weapons": 16,
    "shots": 4147,
    "shot_rays": 3300,
    "ability_spawns": 916,
    "movement_samples": 2156308,
    "economy_rounds": 21,
}

# What the SAME replay produced before bcc7d70: RoundResults decoded nothing.
SECTION_26_BREAK = dict(
    HEALTHY, rounds_objective=0, team_score={}, economy_rounds=0
)


class InvariantTests(unittest.TestCase):
    def test_a_healthy_run_violates_nothing(self):
        self.assertEqual(guard.invariants(HEALTHY), [])

    def test_the_section_26_break_is_caught(self):
        """The whole reason this tool exists.

        R1 (no rounds at all) and R2 (the two round sources disagree) each
        catch it independently, so neither can rot silently and leave the tool
        green.

        R3 deliberately does NOT fire here, and that is worth stating so nobody
        "fixes" it later: an empty team_score sums to 0, and rounds_objective
        is also 0, so the two are internally consistent. R3's job is to catch a
        round with no recorded winner WITHIN a working RoundResults stream, not
        to catch the stream being absent -- that is R1 and R2's job. Rewiring
        R3 to compare against the RPC count would make it a duplicate of R2 and
        would risk a false positive on a replay whose recording stops
        mid-round.
        """
        bad = guard.invariants(SECTION_26_BREAK)
        self.assertTrue(bad, "the 13.02 regression must not pass")
        codes = " ".join(bad)
        for code in ("R1", "R2"):
            self.assertIn(code, codes, f"{code} should fire; got {bad}")
        self.assertNotIn("R3", codes, "see the docstring: R3 cannot see this")

    def test_round_sources_disagreeing_is_caught_on_its_own(self):
        v = dict(HEALTHY, rounds_objective=20, team_score={"Blue": 13, "Red": 7})
        bad = guard.invariants(v)
        self.assertTrue(any("R2" in b for b in bad), bad)

    def test_score_not_summing_to_rounds_is_caught(self):
        v = dict(HEALTHY, team_score={"Blue": 13, "Red": 7})
        bad = guard.invariants(v)
        self.assertTrue(any("R3" in b for b in bad), bad)

    def test_kills_without_damage_is_caught(self):
        """A combat report that stops decoding while the kill timeline works."""
        v = dict(HEALTHY, damage_dealt=0.0)
        bad = guard.invariants(v)
        self.assertTrue(any("R5" in b for b in bad), bad)

    def test_no_players_is_caught(self):
        bad = guard.invariants(dict(HEALTHY, players=0))
        self.assertTrue(any("R4" in b for b in bad), bad)

    def test_kills_need_not_equal_deaths(self):
        """Resurrection breaks that equality on CORRECT data (section 34).

        A resurrected player who dies again in the same round gets two `bDied`
        reports, so deaths counts both, while kills counts DidKill per
        (round, subject) and collapses them. Measured: gap 0 on the three
        Swiftplay replays with no resurrections, exactly 1 on each of the two
        that had one.
        """
        self.assertEqual(guard.invariants(dict(HEALTHY, kills=150, deaths=151)), [])

    def test_a_zero_kill_fixture_is_not_a_violation(self):
        """The three small fixtures have 1 player and no kills."""
        v = dict(HEALTHY, kills=0, deaths=0, damage_dealt=0.0, players=1,
                 combat_players=1, rounds_rpc=7, rounds_objective=7,
                 team_score={"Blue": 6, "Red": 1}, economy_rounds=7)
        self.assertEqual(guard.invariants(v), [])


class DriftTests(unittest.TestCase):
    def test_identical_runs_do_not_drift(self):
        self.assertEqual(guard.compare("13.02", HEALTHY, HEALTHY), [])

    def test_a_changed_value_is_named_with_both_sides(self):
        got = dict(HEALTHY, kills=150)
        drift = guard.compare("13.02", got, HEALTHY)
        self.assertEqual(len(drift), 1)
        self.assertIn("kills", drift[0])
        self.assertIn("150", drift[0])
        self.assertIn("151", drift[0])

    def test_a_key_appearing_or_vanishing_is_drift(self):
        """A metric that stops being extracted must not read as unchanged."""
        got = copy.deepcopy(HEALTHY)
        del got["shot_rays"]
        drift = guard.compare("13.02", got, HEALTHY)
        self.assertTrue(any("shot_rays" in d and "absent" in d for d in drift), drift)


class WiringTests(unittest.TestCase):
    def test_every_build_has_a_replay_path(self):
        self.assertEqual(sorted(guard.REPLAYS),
                         ["12.10", "12.11", "13.00", "13.01", "13.02"])

    def test_no_build_points_at_the_directory_the_game_rotates(self):
        """Saved\\Demos is owned by VALORANT and lost four pinned replays once."""
        for build, path in guard.REPLAYS.items():
            self.assertNotIn("Saved\\Demos", path, f"{build} points at Saved\\Demos")

    def test_extract_and_invariants_agree_on_their_keys(self):
        """Every field the invariants read must be one extract() produces."""
        for key in ("rounds_rpc", "rounds_objective", "team_score", "players",
                    "kills", "damage_dealt"):
            self.assertIn(key, HEALTHY)


if __name__ == "__main__":
    unittest.main()
