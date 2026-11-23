import json
from unittest import mock

import pyarrow as pa
import pyarrow.parquet as pq

from support import TempDirTestCase, run_cli
from wire_fixtures import FIELD_SCHEMA, BitWriter, field_row
import check_rep_movement_levels as levels

#: table.rs: PICKUP byte rotator, whole units; SEEKER short, two decimals.
#: scoped_types.rs: TURRET short, two decimals.
PICKUP = "/Game/Weapons/WeaponPickups/EquippablePickupProjectile.EquippablePickupProjectile_C"
SEEKER = ("/Game/Characters/AggroBot/S0/Ability_Q/Pawn_Aggrobot_SeekerNade."
          "Pawn_Aggrobot_SeekerNade_C")
TURRET = ("/Game/Characters/Killjoy/S0/Ability_E/Pawn_Killjoy_E_Turret."
          "Pawn_Killjoy_E_Turret_C")
UNTYPED = "/Test/Projectile_Untyped.Projectile_Untyped_C"
PAWN = "/Test/Pawn_Untyped.Pawn_Untyped_C"
ACTORS = pa.schema([("time_ms", pa.uint32()), ("channel_index", pa.uint32()),
                    ("actor_net_guid", pa.uint32()), ("class_path", pa.string()),
                    ("event", pa.string()), ("spawn_x", pa.float32()),
                    ("spawn_y", pa.float32()), ("spawn_z", pa.float32())])


def movement(location, velocity=(0, 0, 0), *, scaled=True, yaw=None):
    """An FRepMovement payload: no flags, 20-bit packed location, a rotator
    with only a 16-bit yaw when given, packed velocity."""
    out = BitWriter().write(0, 4).write(20 | scaled << 6, 7)
    for value in location:
        out.write(value, 20)
    out.write(0, 1).write(yaw is not None, 1).write(yaw or 0, 16 if yaw is not None else 0)
    out.write(0, 1).write(20 | 64, 7)
    for value in velocity:
        out.write(value, 20)
    return out.to_bytes()


def write_export(root, opens, rows):
    """`opens` (time, channel, guid, group, spawn); `rows` (time, channel,
    guid, group, payload[, value_str])."""
    root.mkdir()
    (root / "manifest.json").write_text(json.dumps({"replay_build": "13.02"}), encoding="utf-8")
    pq.write_table(pa.Table.from_pylist([
        dict(zip(ACTORS.names, (t, c, g, group, "open", *spawn)))
        for t, c, g, group, spawn in opens], schema=ACTORS), root / "actors.parquet")
    pq.write_table(pa.Table.from_pylist([
        field_row(time_ms=t, channel_index=c, actor_net_guid=g, group_path=group,
                  field_name="ReplicatedMovement", compatible_checksum=2749104612,
                  raw_bits=raw, bit_count=bits, value_str=(*text, None)[0])
        for t, c, g, group, (raw, bits), *text in rows], schema=FIELD_SCHEMA),
        root / "fields.parquet")
    return root


class LevelTests(TempDirTestCase):
    def run_tool(self, opens, rows, *flags):
        code, out, err = run_cli(levels.main, write_export(self.tmp() / "e", opens, rows),
                                 *flags)
        table = {line.split(" | ")[-1]: line.split(" | ") for line in out.splitlines()
                 if line.startswith(("Projectile_", "Pawn_", "Equippable"))}
        return code, table, out + err

    def test_the_join_measures_each_class_level_and_rotator(self):
        code, table, out = self.run_tool(
            opens=[(100, 1, 10, UNTYPED, (1000, -2000, 300)),
                   (100, 2, 20, PAWN, (1234.5, 50, 60)),
                   (100, 3, 30, UNTYPED, (20, 10, 0)),  # |spawn| < 50
                   (100, 4, 40, UNTYPED, (900, 900, 900))],  # no row at its time
            rows=[(100, 2, 21, PAWN, movement((1, 1, 1))),  # reused channel, other actor
                  (100, 1, 10, UNTYPED, movement((1000, -2000, 300))),
                  (100, 1, 10, UNTYPED, movement((5, 5, 5))),  # not the first row
                  (1000, 5, 50, UNTYPED, movement((3000, 0, 0), (1000, 0, 0))),
                  (1100, 5, 50, UNTYPED, movement((3100, 0, 0))),  # 100 units in 0.1 s
                  (100, 2, 20, PAWN, movement((123450, 5000, 6000), yaw=512)),
                  (100, 3, 30, UNTYPED, movement((20, 10, 0))),
                  (300, 4, 40, UNTYPED, movement((900, 900, 900)))])
        self.assertEqual(code, 0, out)
        untyped, pawn = table[UNTYPED], table[PAWN]
        self.assertEqual(untyped[1:9], ["6", "0", "1 (1)", "1", "0", "1.0000/1.0000/1.0000",
                                        "0.0000", "1.00 (1)"], out)
        self.assertEqual(untyped[11], "untyped, RoundWholeNumber, rotator either")
        self.assertEqual(pawn[3], "1 (1)", out)
        self.assertEqual(pawn[9], "0/1/1/0", out)
        self.assertEqual(pawn[11], "untyped, RoundTwoDecimals, rotator RepMovementShort")

    def test_a_declared_class_must_measure_its_own_level(self):
        spawn = (1500, -700, 200)
        for group, payload, verdict, want in (
            (PICKUP, movement(spawn), "typed, agrees", 0),
            (PICKUP, movement((150000, -70000, 20000)),
             "FAILED: declared RoundWholeNumber, measured RoundTwoDecimals", 1),
            (PICKUP, movement((1503, -700, 200)),
             "FAILED: declared RoundWholeNumber, measured not clean", 1),
            # short-only: a rotator read as Byte would fail on consumption instead
            (TURRET, movement(spawn, yaw=512),
             "FAILED: declared RoundTwoDecimals, measured RoundWholeNumber", 1),
        ):
            with self.subTest(group=group, verdict=verdict):
                code, table, out = self.run_tool(
                    [(100, 1, 10, group, spawn)], [(100, 1, 10, group, payload, "{}")])
                self.assertEqual((code, table[group][2], table[group][11]),
                                 (want, "1", verdict), out)

    def test_a_declared_rotator_must_consume_every_row(self):
        for payload, counts in ((movement((1500, -700, 200), yaw=512), "0/1/1/0"),
                                ((b"", 3), "0/0/1/1")):
            with self.subTest(counts=counts):
                code, table, out = self.run_tool(
                    [(100, 1, 10, PICKUP, (1500, -700, 200))],
                    [(100, 1, 10, PICKUP, movement((1500, -700, 200))),
                     (200, 1, 10, PICKUP, payload)])
                self.assertEqual(table[PICKUP][9], counts, out)
                self.assertEqual((code, table[PICKUP][11]),
                                 (1, "FAILED: ByteComponents does not consume every row"), out)

    def test_an_unscaled_row_does_not_vote(self):
        """Whole units at any level: joined, it would read SEEKER as whole."""
        code, table, out = self.run_tool(
            [(100, 1, 10, SEEKER, (1500, -700, 200))],
            [(100, 1, 10, SEEKER, movement((1500, -700, 200), scaled=False, yaw=512), "{}")])
        self.assertEqual((code, table[SEEKER][3], table[SEEKER][5], table[SEEKER][11]),
                         (0, "0 (0)", "1", "typed, no joins"), out)

    def test_a_declared_class_must_be_typed_on_every_row(self):
        """A null typed column on a declared class is the overlay not running,
        whatever level the raw bits measure."""
        spawn = (1500, -700, 200)
        for texts, verdict, want in (
            (("{}", "{}"), "typed, agrees", 0),
            (("{}", None), "FAILED: 1 of 2 rows have no typed value", 1),
            ((None, None), "FAILED: 2 of 2 rows have no typed value", 1),
        ):
            with self.subTest(texts=texts):
                code, table, out = self.run_tool(
                    [(100, 1, 10, PICKUP, spawn)],
                    [(t, 1, 10, PICKUP, movement(spawn), text)
                     for t, text in zip((100, 200), texts)])
                self.assertEqual((code, table[PICKUP][11]), (want, verdict), out)

    def test_an_untyped_class_needs_no_typed_value(self):
        code, table, out = self.run_tool([], [(100, 1, 10, UNTYPED, movement((5, 5, 5)))])
        self.assertEqual((code, table[UNTYPED][2]), (0, "0"), out)

    def test_require_all_fails_on_a_typed_class_with_no_row(self):
        rows = [(100, 1, 10, PICKUP, movement((1500, -700, 200)), "{}")]
        declared = {PICKUP: ("ByteComponents", "RoundWholeNumber"),
                    SEEKER: ("ShortComponents", "RoundTwoDecimals")}
        with mock.patch.object(levels, "declared_types", return_value=declared):
            code, _table, out = self.run_tool([], rows)
            self.assertEqual(code, 0, out)
            self.assertIn("typed classes with no row: 1; failed: 0", out)
            code, _table, out = self.run_tool([], rows, "--require-all")
            self.assertEqual(code, 1, out)
            self.assertIn("1 typed classes with no row: Pawn_Aggrobot_SeekerNade_C", out)
            del declared[SEEKER]
            code, _table, out = self.run_tool([], rows, "--require-all")
            self.assertEqual(code, 0, out)

    def test_no_export_or_no_row_exits_2(self):
        self.assertEqual(run_cli(levels.main, self.tmp())[0], 2)
        code, _table, out = self.run_tool([(100, 1, 10, PICKUP, (1500, -700, 200))], [])
        self.assertEqual(code, 2, out)
        self.assertIn("ReplicatedMovement rows 0", out)
