"""`extract_active_effects.py` must not treat a dormancy close as a despawn.

`actors.parquet` gained a third `event` value when the sink stopped recording
every channel close as `"close"`. A dormant actor has NOT been destroyed -- it
merely stopped replicating, which for a settled smoke or wall is the normal
steady state -- so ending its lifetime there would make persistent effects
vanish early in any reproduction built on this table.

The failure this file guards is the quieter one: `elif ev == "close"` simply
does not match `"dormant"`, so the instance stays pending and falls out of the
loop as open-ended. No row is lost and nothing errors; the output just silently
changes meaning. Counting is what makes that visible.
"""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import extract_active_effects as effects  # noqa: E402

CLASS = "/Game/Characters/Pandemic/S0/Ability_Q/GameObject_Pandemic_Q_Smoke.GameObject_X_C"


def _export(tmp: Path, rows: list[tuple[int, str, int]]) -> Path:
    """Write a minimal `actors.parquet` of `(guid, event, time_ms)` rows."""
    n = len(rows)
    table = pa.table({
        "actor_net_guid": pa.array([r[0] for r in rows], pa.int64()),
        "event": pa.array([r[1] for r in rows], pa.string()),
        "time_ms": pa.array([r[2] for r in rows], pa.int64()),
        "class_path": pa.array([CLASS] * n, pa.string()),
        "spawn_x": pa.array([1.0] * n, pa.float64()),
        "spawn_y": pa.array([2.0] * n, pa.float64()),
        "spawn_z": pa.array([3.0] * n, pa.float64()),
    })
    out = tmp / "export"
    out.mkdir(parents=True, exist_ok=True)
    pq.write_table(table, out / "actors.parquet")
    return out


class DormantCloseTests(unittest.TestCase):
    def setUp(self):
        import tempfile
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def test_a_dormant_actor_does_not_end_its_effect_instance(self):
        """Dormancy is not destruction: the instance stays open-ended."""
        rows = effects.build(_export(self.tmp, [(7, "open", 100), (7, "dormant", 500)]))
        self.assertEqual(len(rows), 1)
        self.assertIsNone(rows[0]["close_ms"])
        self.assertIsNone(rows[0]["duration_ms"])

    def test_a_dormant_instance_is_counted_rather_than_quietly_open_ended(self):
        """The tally is the whole point -- an unexplained open-ended row and a
        dormancy-ended one are indistinguishable in the table itself."""
        out = _export(self.tmp, [(7, "open", 100), (7, "dormant", 500)])
        self.assertEqual(effects.build_with_tally(out)[1]["went_dormant"], 1)

    def test_a_real_close_still_ends_the_instance(self):
        rows = effects.build(_export(self.tmp, [(7, "open", 100), (7, "close", 500)]))
        self.assertEqual((rows[0]["close_ms"], rows[0]["duration_ms"]), (500, 400))
        self.assertEqual(
            effects.build_with_tally(_export(self.tmp, [(7, "open", 100), (7, "close", 500)]))[1]
            ["went_dormant"], 0)

    def test_an_actor_that_wakes_after_dormancy_keeps_one_instance(self):
        """A dormant actor that replicates again was never gone. Two instances
        here would be a false despawn/respawn pair in any reproduction."""
        out = _export(self.tmp, [(7, "open", 100), (7, "dormant", 300), (7, "close", 900)])
        rows, tally = effects.build_with_tally(out)
        self.assertEqual(len(rows), 1)
        self.assertEqual((rows[0]["open_ms"], rows[0]["close_ms"]), (100, 900))
        self.assertEqual(tally["went_dormant"], 1)


if __name__ == "__main__":
    unittest.main()
