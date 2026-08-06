"""Guards for the component-remap check.

The remap table in `sink/paths.rs` was read out of a shipped build, and a later
build can rename a component without anything in the repo noticing: the replay
never named it either, so there is no test that can fail. The export baseline
pins `overlay_no_field_name` and would catch it -- but only on the one replay
that has a baseline.

What the checker does instead works on any export: for each remap pair, ask
whether the native group it targets carries rows. If it does not, and the bare
leaf still does, the remap stopped matching.
"""
import collections
import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_component_remaps as guard  # noqa: E402


PAIRS = [("ZoomStateMachine", "/Script/ShooterGame.EquippableStateMachineComponent")]


class VerdictTests(unittest.TestCase):
    def test_rows_on_the_native_group_mean_the_remap_works(self):
        v = guard.verdicts(PAIRS, {"/Script/ShooterGame.EquippableStateMachineComponent": 8112})
        self.assertEqual([x.state for x in v], ["ok"])

    def test_a_dead_remap_with_the_leaf_still_present_fails(self):
        """The rename signature: nothing on the target, everything still bare."""
        v = guard.verdicts(PAIRS, {"ZoomStateMachine": 8112})
        self.assertEqual([x.state for x in v], ["broken"])
        self.assertIn("8112", v[0].detail)

    def test_neither_present_is_absent_not_broken(self):
        """A replay without that component is not evidence of anything."""
        v = guard.verdicts(PAIRS, {"/Script/Other.Thing": 5})
        self.assertEqual([x.state for x in v], ["absent"])

    def test_a_leaf_lingering_beside_a_working_target_is_still_ok(self):
        """Some blocks resolve by another route and stay bare; that is normal.

        On the reference replay `ZoomStateMachine` drops from 8,112 rows to 70
        rather than to zero, so requiring the leaf to disappear would fail on a
        healthy export. 70 against 60,101 is 0.12%.
        """
        v = guard.verdicts(PAIRS, {
            "/Script/ShooterGame.EquippableStateMachineComponent": 60101,
            "ZoomStateMachine": 70,
        })
        self.assertEqual([x.state for x in v], ["ok"])

    def test_a_dead_leaf_sharing_a_live_target_is_caught(self):
        """The case that a target-only check misses.

        Nine leaves map to `EquippableStateMachineComponent`. If one is renamed
        by a build the other eight keep the target busy, so "does the target
        have rows" says everything is fine while that component's blocks are all
        bare again. Measured by simulating the rename: the leaf goes to 15.58%
        of the target's rows, against 0.12% when it is healthy.
        """
        v = guard.verdicts(PAIRS, {
            "/Script/ShooterGame.EquippableStateMachineComponent": 52059,
            "ZoomStateMachine": 8112,
        })
        self.assertEqual([x.state for x in v], ["broken"])

    def test_class_net_cache_rows_do_not_count_as_bare(self):
        """The RepLayout-only remaps leave their RPC stream bare on purpose.

        `AbilitiesAndBuffsComponent` is mapped for RepLayout blocks and
        deliberately not for ClassNetCache ones -- the AbilitySystem `_cnc`
        group declares an incomplete function table, so remapping the RPC stream
        would mis-parse it. On the reference replay that leaves 9,366 bare rows
        against 652 on the native group, every one of them a `_cnc_h*` or
        unresolved-payload row. Counting those would report the healthy case as
        broken, which it did before this was split out.
        """
        counts = guard.bare_counts({
            "AbilitiesAndBuffsComponent": collections.Counter({
                "_cnc_h1": 4683,
                "__vrfkit_unresolved_class_net_cache_payload__": 4683,
            }),
            "ZoomStateMachine": collections.Counter({"CurrentState": 70}),
        })
        self.assertEqual(counts["AbilitiesAndBuffsComponent"], 0)
        self.assertEqual(counts["ZoomStateMachine"], 70)

    def test_exit_code_is_nonzero_only_when_something_is_broken(self):
        ok = guard.verdicts(PAIRS, {"/Script/ShooterGame.EquippableStateMachineComponent": 1})
        broken = guard.verdicts(PAIRS, {"ZoomStateMachine": 1})
        absent = guard.verdicts(PAIRS, {})
        self.assertEqual(guard.exit_code(ok), 0)
        self.assertEqual(guard.exit_code(absent), 0)
        self.assertEqual(guard.exit_code(broken), 1)


class PairParsingTests(unittest.TestCase):
    def test_the_real_table_parses(self):
        """Reads sink/paths.rs, so a reformat that breaks it shows up here."""
        pairs = guard.remap_pairs()
        self.assertGreater(len(pairs), 15)
        self.assertIn(
            ("ZoomStateMachine", "/Script/ShooterGame.EquippableStateMachineComponent"),
            pairs,
        )
        self.assertIn(
            ("InventoryComponent", "/Script/ShooterGame.AresInventory"), pairs
        )

    def test_every_target_is_a_script_path(self):
        for leaf, native in guard.remap_pairs():
            self.assertTrue(native.startswith("/Script/"), f"{leaf} -> {native}")


if __name__ == "__main__":
    unittest.main()
