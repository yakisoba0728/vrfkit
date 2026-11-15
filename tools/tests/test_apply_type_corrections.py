"""Guards for apply_type_corrections.py: its pins, and `main()`'s verdict on a
one-line table.rs, judged by whether applying would change the file."""
import unittest
from unittest import mock


from support import TempDirTestCase, run_cli
import apply_type_corrections as atc

#: The handle table after OVERLAY_TABLE, naming a pinned key: applying must
#: leave it byte for byte.
HANDLES = ("#[rustfmt::skip]\n"
           "pub static OVERLAY_HANDLE_TABLE: [OverlayHandleEntry; 1] = [\n"
           '    OverlayHandleEntry { group_path: "/Script/ShooterGame.BaseTeamState", '
           'handle: 7, field_name: "LoadoutValue" },\n'
           "];\n")
SHORT_WHOLE = "FieldType::" + atc.rep_movement("ShortComponents", "RoundWholeNumber")
#: What each glob pin stands for in a fixture: one group it matches.
WEAPON = ("/Game/Equippables/X", "215")
SMOKE_SCREEN = ("XSmokeScreenX", "ReplicatedMovement")
STATS = ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator"
         ".Comp_AbilityStatisticsReplicator_C")


def render(rows) -> str:
    """A table.rs holding `rows` `(group, field, FieldType)` in the given order."""
    body = "".join(f'    OverlayEntry {{ group_path: "{g}", field_name: "{f}", '
                   f"field_type: {t} }},\n" for g, f, t in rows)
    return ("// Overlay table.\n\nuse crate::decode::FieldType;\n\n#[rustfmt::skip]\n"
            f"pub static OVERLAY_TABLE: [OverlayEntry; {len(rows)}] = [\n{body}];\n\n{HANDLES}")


def whole_table(overrides=None, drop=(), extra=()) -> str:
    """A sorted table every pin holds on, with `overrides` {key: type} applied,
    `drop` keys left out and `extra` rows added. Derived from the pins, so it
    cannot go stale."""
    rows = {(g.replace("*", "X"), f): t for g, f, t in atc.EXPECTED + atc.WEAPON_PINS}
    rows.update(overrides or {})
    rows.update({(g, f): t for g, f, t in extra})
    return render(sorted((g, f, t) for (g, f), t in rows.items() if (g, f) not in drop))


class PinTests(unittest.TestCase):
    def test_every_addition_names_a_real_field_type(self):
        """Each addition is well formed (`FieldType::X`) and appears once."""
        seen = set()
        for group, field, ftype in atc.ADDITIONS:
            self.assertTrue(group.startswith("/") or group[0].isalpha(), group)
            self.assertTrue(field and not field.startswith("_"), field)
            self.assertTrue(ftype.startswith("FieldType::"), ftype)
            self.assertNotIn((group, field), seen, f"duplicate {group}/{field}")
            seen.add((group, field))

    def test_additions_stay_the_narrow_exception(self):
        """Pinned exactly, so adding or removing an ADDITION updates this
        number in the same commit; check_docs measures the count too."""
        self.assertEqual(len(atc.ADDITIONS), 148, atc.ADDITIONS)


class MainTests(TempDirTestCase):
    """`main()` on a temporary table.rs."""

    def setUp(self):
        self.path = self.tmp() / "table.rs"
        self.enterContext(mock.patch.object(atc, "TABLE_RS", self.path))

    def run_main(self, source, *args):
        """`(exit_code, stdout, stderr)` for one main() run over `source`."""
        self.path.write_text(source, encoding="utf-8")
        return run_cli(atc.main, *args)

    def assert_unchanged(self, source):
        self.assertEqual(self.path.read_text(encoding="utf-8"), source)

    def test_a_correct_table_passes_and_is_left_alone(self):
        """The summary counts each weapon entry, as check_docs quotes it."""
        for args in (("--check",), ()):
            code, out, err = self.run_main(whole_table(), *args)
            self.assertEqual(code, 0, err)
            self.assertIn(f"all {len(atc.EXPECTED) + 2} pins hold", out)
            self.assert_unchanged(whole_table())

    def test_a_mistyped_entry_fails_check_and_apply_fixes_it(self):
        """Near misses included: a substring test passes UInt32 for Int32 and
        EnumByte for Byte."""
        cases = {
            "signedness": {("/Script/ShooterGame.BaseTeamState", "LoadoutValue"):
                           "FieldType::UInt32"},
            "byte variant": {(STATS, "Slot_12_22D571914FAFD5F0EBD400B7E2F28B36"):
                             "FieldType::EnumByte"},
            "glob": {SMOKE_SCREEN: SHORT_WHOLE},
            "weapon": {WEAPON: "FieldType::Raw"},
            "location level": {(atc.SEEKER_NADE_GROUP, "ReplicatedMovement"): SHORT_WHOLE},
            "rotators": {(group, "ReplicatedMovement"): SHORT_WHOLE
                         for group in atc.GAME_OBJECT_BYTE_ROTATOR_GROUPS},
        }
        for name, overrides in cases.items():
            with self.subTest(name):
                source = whole_table(overrides)
                code, _out, err = self.run_main(source, "--check")
                self.assertEqual(code, 1, "--check must not pass an uncorrected file")
                for (group, field), ftype in overrides.items():
                    self.assertIn(f'"{group}", field_name: "{field}", field_type: {ftype}', err)
                self.assert_unchanged(source)
                code, _out, err = self.run_main(source)
                self.assertEqual(code, 0, err)
                self.assert_unchanged(whole_table())

    def test_absent_exact_pins_are_inserted_in_order(self):
        """Every ADDITION and a dropped correction go back in sorted, the
        length is resynced, and the handle table comes through untouched."""
        code, _out, err = self.run_main(
            whole_table(drop=[(atc.TIMED_BOMB, "DefuseProgress")]), "--check")
        self.assertEqual(code, 1)
        self.assertIn('+    OverlayEntry { group_path: "/Game/GameModes/Bomb/TimedBomb.TimedBomb_C", '
                      'field_name: "DefuseProgress"', err)
        extra = [("/AAAAA.First", "Field", "FieldType::Int32"),
                 ("zzz/Tail.Tail_C", "Field", "FieldType::Int32")]
        dropped = [(g, f) for g, f, _t in atc.ADDITIONS] + [(atc.TIMED_BOMB, "DefuseProgress")]
        source = whole_table(drop=dropped, extra=extra)
        self.assertEqual(self.run_main(source, "--check")[0], 1)
        code, out, err = self.run_main(source)
        self.assertEqual(code, 0, err)
        self.assertIn(f"{len(dropped)} entries changed", out)
        self.assert_unchanged(whole_table(extra=extra))

    def test_an_exact_pin_changes_only_its_own_entry(self):
        """A group that merely contains, or starts like, a pinned one keeps its
        type."""
        source = whole_table(extra=[
            (atc.TIMED_BOMB + ":Rpc", "DefuseProgress", "FieldType::Float"),
            (atc.TIMED_BOMB[:-2], "DefuseProgress", "FieldType::Float"),
            ("/Game/GameModes/Bomb/TimedBomb.TimedBomb_C2", "215", "FieldType::Int32"),
        ])
        code, _out, err = self.run_main(source)
        self.assertEqual(code, 0, err)
        self.assert_unchanged(source)

    def test_a_glob_that_matches_nothing_fails_both_modes(self):
        """A glob with no entry is what its discovery going dead looks like.
        Beside a mistyped entry, --check reports both."""
        cases = {
            "smoke screen": ([SMOKE_SCREEN], {}, "*SmokeScreen*"),
            "weapons": ([WEAPON, (WEAPON[0], "216")], {}, "/Game/Equippables/*"),
            "both": ([SMOKE_SCREEN], {(atc.SEEKER_NADE_GROUP, "ReplicatedMovement"): SHORT_WHOLE},
                     "RoundWholeNumber"),
        }
        for name, (drop, overrides, needle) in cases.items():
            with self.subTest(name):
                source = whole_table(overrides, drop=drop)
                for args in (("--check",), ()):
                    code, _out, err = self.run_main(source, *args)
                    self.assertEqual(code, 1)
                    self.assertIn("matches no entry", err)
                    self.assert_unchanged(source)
                self.assertIn(needle, self.run_main(source, "--check")[2])

    def test_one_entry_pinned_two_ways_fails_both_modes(self):
        """The pawn ADDITIONS also sit under the `/Game/Characters/*` Float
        glob: one retyped to Double must fail, not let the last pin win,
        whichever of the two types the file holds."""
        key = ("/Game/Characters/Killjoy/S0/Ability_E/Pawn_Killjoy_E_Turret."
               "Pawn_Killjoy_E_Turret_C", "ReplayLastTransformUpdateTimeStamp")
        retyped = [pin for pin in atc.EXPECTED if pin[:2] != key] + [(*key, "FieldType::Double")]
        with mock.patch.object(atc, "EXPECTED", retyped):
            for ftype in ("FieldType::Float", "FieldType::Double"):
                source = whole_table({key: ftype})
                for args in (("--check",), ()):
                    with self.subTest(ftype=ftype, args=args):
                        code, _out, err = self.run_main(source, *args)
                        self.assertEqual(code, 1)
                        self.assertIn("pinned as FieldType::Float and as FieldType::Double", err)
                        self.assert_unchanged(source)

    def test_the_layout_is_enforced(self):
        """Only one entry per line parses, and each key once; an unsorted table
        or a stale length fails --check though every pin holds."""
        table = whole_table()
        first = table.index("    OverlayEntry {")
        end = table.index("\n", first)
        entry = table[first:end]
        broken = (
            (entry.replace(", field_name", ",\n        field_name"), "not a one-line OverlayEntry"),
            (entry + "\n" + entry, "declared twice"),
        )
        for replacement, message in broken:
            with self.subTest(message), self.assertRaises(SystemExit) as raised:
                self.run_main(table[:first] + replacement + table[end:], "--check")
            self.assertIn(message, str(raised.exception))
        second_end = table.index("\n", end + 1)
        unsorted = table[:first] + table[end + 1:second_end] + "\n" + entry + table[second_end:]
        for name, source in (("unsorted", unsorted),
                             ("stale length", table.replace("OverlayEntry; ", "OverlayEntry; 1"))):
            with self.subTest(name):
                self.assertEqual(atc.apply(atc.parse_table(source)[1])[1], [])
                self.assertEqual(self.run_main(source, "--check")[0], 1)

    def test_unknown_flag_is_rejected_and_cannot_fall_into_write_mode(self):
        source = whole_table({SMOKE_SCREEN: SHORT_WHOLE})
        self.path.write_text(source, encoding="utf-8")
        with self.assertRaises(SystemExit) as raised:
            atc.main(["--chekc"])
        self.assertEqual(raised.exception.code, 2)
        self.assert_unchanged(source)
