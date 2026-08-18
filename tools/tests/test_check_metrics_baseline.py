"""Guards for the metrics guard.

check_metrics_baseline.py exists because framing counters cannot see a decoder
that stops producing values. Its invariants are the part that carries that
weight -- they need no baseline and survive legitimate changes -- so they are
the part that must not rot.

The headline case uses the REAL shape of the section-26 break, measured on the
13.02 fixture before commit bcc7d70: ClientRoundStart RPCs said 21 rounds while
BombGameState RoundResults produced none, so team_score was empty.
"""
import contextlib
import copy
import io
import json
import os
import sys
import tempfile
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


class UpdateScopeTests(unittest.TestCase):
    """`--update --only X` used to rewrite the whole-build baseline with X alone.

    The module docstring makes the one-file-covering-every-build design a
    guarantee: "a build disappearing from the set is itself a failure. Per-file
    baselines cannot see that". Re-pinning after looking at a single build
    deleted the other four from the file, so the very next full run reported
    them as new rather than as missing, and the guarantee was gone.
    """

    STORED = {"12.10": {"kills": 1}, "13.02": {"kills": 2}}

    def test_a_scoped_update_keeps_the_builds_it_did_not_look_at(self):
        merged = guard.merged_metrics(self.STORED, {"13.02": {"kills": 9}},
                                      only=["13.02"])
        self.assertEqual(sorted(merged), ["12.10", "13.02"])
        self.assertEqual(merged["12.10"], {"kills": 1})

    def test_a_scoped_update_replaces_the_build_it_did_look_at(self):
        merged = guard.merged_metrics(self.STORED, {"13.02": {"kills": 9}},
                                      only=["13.02"])
        self.assertEqual(merged["13.02"], {"kills": 9})

    def test_an_unscoped_update_replaces_the_whole_set(self):
        """A full run is the only thing allowed to retire a build.

        Merging there would keep a build pinned forever after it left REPLAYS,
        and every later run would then fail with "MISSING from this run".
        """
        merged = guard.merged_metrics(self.STORED, {"13.02": {"kills": 9}},
                                      only=None)
        self.assertEqual(sorted(merged), ["13.02"])


class WiringTests(unittest.TestCase):
    def test_pipeline_uses_separate_export_and_bundle_trees(self):
        self.assertTrue(hasattr(guard, "pipeline_paths"))
        export_dir, bundle_dir, metrics_path = guard.pipeline_paths(Path("scratch"))
        self.assertEqual(export_dir, Path("scratch/export"))
        self.assertEqual(bundle_dir, Path("scratch/bundle"))
        self.assertEqual(metrics_path, Path("scratch/metrics.json"))

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


#: A raw `compute_metrics.py` output, shaped exactly like the real valplay
#: JSON `extract()` reads -- nested dicts, `per_player` maps, not the already
#: flattened `HEALTHY` fixture above. `HEALTHY` pins what the invariants see;
#: this pins the seam one layer earlier, between valplay's schema and this
#: tool's parsing of it. That seam had no coverage: a renamed or reshaped key
#: on the valplay side raises a loud `KeyError` today (by construction --
#: `extract()` indexes with `[...]`, never `.get(..., default)`), but nothing
#: proved the *mapping itself* -- which flattened key reads which nested path,
#: and which fields get summed versus counted -- was still right.
RAW_METRICS = {
    "combat": {
        "per_player": {
            "p1": {"kills": 5, "deaths": 3, "assists": 1, "headshots": 2,
                    "damage_dealt": 501.5},
            "p2": {"kills": 2, "deaths": 4, "assists": 0, "headshots": 0,
                    "damage_dealt": 88.25},
        },
    },
    "tactical": {
        "per_player": {
            "p1": {"first_bloods": 1, "trade_kills": 0},
            "p2": {"first_bloods": 0, "trade_kills": 2},
        },
    },
    "rounds": {"round_count": 6, "client_round_start_events": 6},
    "objective": {"round_count": 6, "team_score": {"Blue": 4, "Red": 2}},
    "objective_detail": {"plant_count": 3, "defuse_count": 1},
    "players": ["p1", "p2"],
    "kast": {"per_player": {"p1": {"kast_rounds": 5}, "p2": {"kast_rounds": 4}}},
    "ultimate": {"total_casts": 2},
    "weapons": {"distinct_weapons": 3,
                "shots_by_weapon": {"Vandal": 30, "Classic": 12}},
    "shot_rays": {"ray_count": 41},
    "ability_usage": {"ability_spawn_count": 9},
    "movement_summary": {"movement_samples": 12345},
    "economy_detail": {"rounds": 6},
}


class ExtractShapeTests(unittest.TestCase):
    """`extract()` is the only code that reads valplay's real JSON shape.
    Nothing else in this suite exercises it against a shape that looks like
    what `compute_metrics.py` actually emits -- every other test starts from
    the already-flattened `HEALTHY` dict, which proves the invariants but
    never proves the mapping into them.
    """

    def test_scalar_fields_are_read_from_their_nested_path(self):
        got = guard.extract(RAW_METRICS)
        self.assertEqual(got["rounds_rpc"], 6)
        self.assertEqual(got["rounds_objective"], 6)
        self.assertEqual(got["client_round_starts"], 6)
        self.assertEqual(got["team_score"], {"Blue": 4, "Red": 2})
        self.assertEqual(got["plants"], 3)
        self.assertEqual(got["defuses"], 1)
        self.assertEqual(got["players"], 2)
        self.assertEqual(got["combat_players"], 2)
        self.assertEqual(got["ultimate_casts"], 2)
        self.assertEqual(got["distinct_weapons"], 3)
        self.assertEqual(got["shot_rays"], 41)
        self.assertEqual(got["ability_spawns"], 9)
        self.assertEqual(got["movement_samples"], 12345)
        self.assertEqual(got["economy_rounds"], 6)

    def test_per_player_combat_fields_are_summed_across_players(self):
        got = guard.extract(RAW_METRICS)
        self.assertEqual(got["kills"], 7)       # 5 + 2
        self.assertEqual(got["deaths"], 7)      # 3 + 4
        self.assertEqual(got["assists"], 1)     # 1 + 0
        self.assertEqual(got["headshots"], 2)   # 2 + 0
        self.assertEqual(got["damage_dealt"], 589.75)  # 501.5 + 88.25

    def test_per_player_tactical_and_kast_fields_are_summed(self):
        got = guard.extract(RAW_METRICS)
        self.assertEqual(got["first_bloods"], 1)   # 1 + 0
        self.assertEqual(got["trade_kills"], 2)    # 0 + 2
        self.assertEqual(got["kast_rounds"], 9)    # 5 + 4

    def test_shots_are_summed_across_weapons_not_taken_from_distinct_weapons(self):
        """`shots` and `distinct_weapons` read different things off the same
        `weapons` block -- a copy/paste of one into the other would pass every
        other test here since both are small integers."""
        got = guard.extract(RAW_METRICS)
        self.assertEqual(got["shots"], 42)  # 30 + 12
        self.assertNotEqual(got["shots"], got["distinct_weapons"])

    def test_a_player_with_no_combat_entry_does_not_crash_the_sum(self):
        """`_sum` reads `.get(field) or 0` per player -- a player present in
        `players` but absent from `combat.per_player` (never fired a shot,
        never took damage) must not raise, and must not count."""
        raw = copy.deepcopy(RAW_METRICS)
        raw["players"].append("p3")
        got = guard.extract(raw)
        self.assertEqual(got["players"], 3)
        self.assertEqual(got["combat_players"], 2)  # p3 never joined combat
        self.assertEqual(got["kills"], 7)            # unchanged


#: Stand-ins for the three pipeline stages `run_one` shells out to --
#: `vrfkit export`, `to_valplay_bundle.py`, and valplay's `compute_metrics.py`.
#: The first is invoked positionally (`[str(exe), "export", ...]`), so under
#: `sys.executable` a file literally named `export` in the process's cwd
#: stands in for it, exactly as the other two corpus scripts' fake executables
#: do. The other two are invoked by explicit path, so ordinary `.py` files
#: patched onto `guard.BUNDLE_TOOL` / `guard.COMPUTE_METRICS` stand in for
#: them. The fake `compute_metrics.py` does not compute anything -- it copies
#: whatever this test staged as the desired metrics.json, so one pair of fake
#: scripts can play every scenario below by changing what gets staged.
FAKE_EXPORT_SCRIPT = '''\
import sys
from pathlib import Path
out = Path(sys.argv[sys.argv.index("--out") + 1])
out.mkdir(parents=True, exist_ok=True)
print("export ok")
'''

FAKE_BUNDLE_SCRIPT = '''\
import sys
from pathlib import Path
out = Path(sys.argv[sys.argv.index("-o") + 1])
out.mkdir(parents=True, exist_ok=True)
print("bundle ok")
'''

#: Reads the desired metrics.json from the path the test staged in an
#: environment variable (`subprocess.run` inherits the parent's environment
#: by default, so this reaches the child) and writes it to wherever `-o`
#: says -- so it stands in for compute_metrics.py without knowing anything
#: about the bundle format `-o`'s sibling argument actually names.
FAKE_METRICS_SCRIPT = '''\
import os
import shutil
import sys
from pathlib import Path
staged = Path(os.environ["VRFKIT_TEST_DESIRED_METRICS"])
out = Path(sys.argv[sys.argv.index("-o") + 1])
shutil.copyfile(staged, out)
print("metrics ok")
'''


class MainWiringTests(unittest.TestCase):
    """`InvariantTests` and `DriftTests` above prove the pure functions; they
    say nothing about whether `main()` calls them and acts on the result
    before deciding an exit code -- the same wiring gap the other two corpus
    scripts' `MainWiringTests` closes, for the one check this project built
    specifically because the framing layer cannot see a semantic break.
    """

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        (self.root / "export").write_text(FAKE_EXPORT_SCRIPT, encoding="utf-8")
        self.bundle_tool = self.root / "fake_bundle.py"
        self.bundle_tool.write_text(FAKE_BUNDLE_SCRIPT, encoding="utf-8")
        self.compute_metrics = self.root / "fake_compute_metrics.py"
        self.compute_metrics.write_text(FAKE_METRICS_SCRIPT, encoding="utf-8")

        self.replay = self.root / "match.vrf"
        self.replay.write_bytes(b"not a real replay")

        self._orig_bundle_tool = guard.BUNDLE_TOOL
        self._orig_compute_metrics = guard.COMPUTE_METRICS
        self._orig_replays = guard.REPLAYS
        guard.BUNDLE_TOOL = self.bundle_tool
        guard.COMPUTE_METRICS = self.compute_metrics
        guard.REPLAYS = {"test": str(self.replay)}
        self.addCleanup(self._restore_module_state)

        self._previous_cwd = Path.cwd()
        os.chdir(self.root)
        self.addCleanup(os.chdir, self._previous_cwd)

        self._argv = sys.argv
        self.addCleanup(self._restore_argv)

    def _restore_module_state(self):
        guard.BUNDLE_TOOL = self._orig_bundle_tool
        guard.COMPUTE_METRICS = self._orig_compute_metrics
        guard.REPLAYS = self._orig_replays

    def _restore_argv(self):
        sys.argv = self._argv

    def stage_metrics(self, metrics: dict) -> None:
        staged = self.root / "desired_metrics.json"
        staged.write_text(json.dumps(metrics), encoding="utf-8")
        os.environ["VRFKIT_TEST_DESIRED_METRICS"] = str(staged)
        self.addCleanup(os.environ.pop, "VRFKIT_TEST_DESIRED_METRICS", None)

    def run_main(self, extra_args=()):
        argv = ["check_metrics_baseline.py", "--exe", sys.executable,
                "--only", "test", "--jobs", "1", *extra_args]
        sys.argv = argv
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            code = guard.main()
        return code, out.getvalue()

    def test_a_healthy_run_matching_the_baseline_exits_zero(self):
        self.stage_metrics(RAW_METRICS)
        baseline = self.root / "baseline.json"
        baseline.write_text(json.dumps(
            {"metrics": {"test": guard.extract(RAW_METRICS)}}), encoding="utf-8")

        code, output = self.run_main(["--baseline", str(baseline)])

        self.assertEqual(code, 0, output)

    def test_an_invariant_violation_fails_the_run_and_refuses_to_update(self):
        """The headline case: R1/R2 fire (see `InvariantTests`), and `main()`
        must both exit non-zero AND leave the baseline untouched under
        `--update` -- pinning a broken run would make the NEXT clean run look
        like drift instead of a fix."""
        broken = dict(RAW_METRICS)
        broken["objective"] = {"round_count": 0, "team_score": {}}
        broken["economy_detail"] = {"rounds": 0}
        self.stage_metrics(broken)
        baseline = self.root / "baseline.json"
        original = json.dumps({"metrics": {}})
        baseline.write_text(original, encoding="utf-8")

        code, output = self.run_main(["--baseline", str(baseline), "--update"])

        self.assertEqual(code, 1, output)
        self.assertIn("R1", output)
        self.assertIn("baseline NOT updated", output)
        self.assertEqual(baseline.read_text(encoding="utf-8"), original,
                         "a broken run must not be pinned")

    def test_drift_from_the_baseline_fails_the_run(self):
        self.stage_metrics(RAW_METRICS)
        stored = guard.extract(RAW_METRICS)
        stored = dict(stored, kills=stored["kills"] + 1000)
        baseline = self.root / "baseline.json"
        baseline.write_text(json.dumps({"metrics": {"test": stored}}),
                            encoding="utf-8")

        code, output = self.run_main(["--baseline", str(baseline)])

        self.assertEqual(code, 1, output)
        self.assertIn("drifted", output)
        self.assertIn("kills", output)

    def test_a_missing_baseline_is_a_controlled_failure(self):
        self.stage_metrics(RAW_METRICS)
        baseline = self.root / "does-not-exist.json"

        code, output = self.run_main(["--baseline", str(baseline)])

        self.assertEqual(code, 2, output)
        self.assertIn("baseline not found", output)

    def test_a_replay_that_does_not_exist_fails_the_pipeline(self):
        guard.REPLAYS = {"test": str(self.root / "missing.vrf")}
        baseline = self.root / "baseline.json"
        baseline.write_text(json.dumps({"metrics": {}}), encoding="utf-8")

        code, output = self.run_main(["--baseline", str(baseline)])

        self.assertEqual(code, 1, output)
        self.assertIn("did not complete the pipeline", output)
        self.assertIn("replay not found", output)


if __name__ == "__main__":
    unittest.main()
