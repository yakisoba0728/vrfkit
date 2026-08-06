"""Guards for the ADDITIONS pass in apply_type_corrections.py.

The corrections in that script rewrite entries that already exist; the
additions pass INSERTS entries the C# descriptors cannot declare. Insertion has
two failure modes the replacements do not:

* the table exists in two layouts -- one entry per line as
  extract_descriptors.py emits it, and the rustfmt'd multi-line form that gets
  committed. The script's own docstring records that a newline-anchored helper
  silently matched nothing on a freshly generated table, which is exactly when
  it is supposed to run. An insertion pass that only works on one layout fails
  the same way.
* the slice is sorted by (group_path, field_name) and declared with an explicit
  length. Inserting at the wrong position breaks `tests::overlay::table_is_sorted`
  and forgetting the length breaks the build.
"""
import contextlib
import io
import re
import sys
import tempfile
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import apply_type_corrections as atc  # noqa: E402


#: A synthetic table containing every real ADDITION plus bookends that sort
#: before and after all of them. DERIVED from ADDITIONS rather than hardcoded:
#: the first version of this file pinned "the two entries" as a literal and
#: broke the moment a third was added, which is noise rather than signal.
BOOKENDS = [
    ("/Game/AAA.AAA_C", "Alpha"),
    ("/Script/ShooterGame.AmmoComponent", "MagazineAmmo"),
    ("/Script/ShooterGame.OwnerExclusivePlayerInfo", "RoundNumber"),
    ("/Script/ShooterGame.ZzzTailComponent", "Stripes"),
]
GROUPS = sorted(BOOKENDS + [(g, f) for g, f, _t in atc.ADDITIONS])


def formatted(pairs):
    """The rustfmt'd layout, which is what is committed."""
    body = "".join(
        "    OverlayEntry {\n"
        f'        group_path: "{g}",\n'
        f'        field_name: "{f}",\n'
        "        field_type: FieldType::Int32,\n"
        "    },\n"
        for g, f in pairs
    )
    return (
        "// 0 entries from 0 groups.\n"
        "// Raw/Custom: 0, Skip: 0, Typed: 0.\n"
        f"pub static OVERLAY_TABLE: [OverlayEntry; {len(pairs)}] = [\n"
        f"{body}"
        "];\n"
    )


def formatted_typed(rows):
    """`formatted`, but each row carries its own field_type.

    `formatted` pins every entry to Int32, which is exactly the type that
    cannot show a wrong-type bug -- the substring check it has to expose is
    `"Int32" in "FieldType::UInt32"`.
    """
    body = "".join(
        "    OverlayEntry {\n"
        f'        group_path: "{g}",\n'
        f'        field_name: "{f}",\n'
        f"        field_type: {t},\n"
        "    },\n"
        for g, f, t in rows
    )
    return (
        "// 0 entries from 0 groups.\n"
        "// Raw/Custom: 0, Skip: 0, Typed: 0.\n"
        f"pub static OVERLAY_TABLE: [OverlayEntry; {len(rows)}] = [\n"
        f"{body}"
        "];\n"
    )


def one_line(pairs):
    """The layout extract_descriptors.py emits, before cargo fmt."""
    body = "".join(
        f'    OverlayEntry {{ group_path: "{g}", field_name: "{f}", '
        "field_type: FieldType::Int32 },\n"
        for g, f in pairs
    )
    return (
        "// 0 entries from 0 groups.\n"
        "// Raw/Custom: 0, Skip: 0, Typed: 0.\n"
        f"pub static OVERLAY_TABLE: [OverlayEntry; {len(pairs)}] = [\n"
        f"{body}"
        "];\n"
    )


ADDED_KEYS = {(g, f) for g, f, _t in atc.ADDITIONS}
WITHOUT = [p for p in GROUPS if p not in ADDED_KEYS]
N_ADDED = len(atc.ADDITIONS)


class AdditionsTests(unittest.TestCase):
    def test_every_addition_names_a_real_field_type(self):
        """`FieldType::X` with a plausible X, and no duplicates.

        The COUNT is deliberately not pinned -- see 26-I and 32 for the bar an
        addition has to clear. What is pinned is that each one is well formed
        and appears once.
        """
        seen = set()
        for group, field, ftype in atc.ADDITIONS:
            # Most groups are full UE paths ("/Script/..."); a few replay-declared
            # groups are bare names ("MagazineAmmo"). Either is valid; empty or
            # non-alphanumeric-leading garbage is not.
            self.assertTrue(group.startswith("/") or group[0].isalpha(), group)
            self.assertTrue(field and not field.startswith("_"), field)
            self.assertTrue(ftype.startswith("FieldType::"), ftype)
            self.assertNotIn((group, field), seen, f"duplicate {group}/{field}")
            seen.add((group, field))

    def test_additions_stay_the_narrow_exception(self):
        """A guardrail on scope: trips on every change so growth is deliberate.

        Each entry asserts a type NO descriptor declares, so the bar is
        individual wire evidence -- the ADDITIONS rationale block plus a
        per-field Rust pin in tests/overlay.rs. The count is pinned exactly
        so adding or removing one forces this number to update in the same
        commit. The prior `<= 8` ceiling silently went stale at 13; an exact
        count cannot.

        25 -> 47 is one piece of evidence, not 22: `FTransform` reaches this
        wire as three separate double vectors, and the 22 entries apply that
        one finding to every group carrying them. `Scale3D` reading exactly
        (1,1,1) is what rules out any other split, and the replay's own
        `compatible_checksum` agrees with the grouping without having been used
        to derive it.

        47 -> 49 is two ordinary additions: `StopMovementTime`, the other half
        of a pair whose `StartMovementTime` is already Float, and
        `HandleNumber`, whose 3,741 rows hold a dense 1..765. One entry each is
        enough -- checksum propagation carries both to their sibling RPCs.

        64 -> 70 is two findings, not six. Five entries type `249` as the
        rotation that pairs with the already-typed `248` on every RPC that
        sends the pair numbered -- 441,814 rows, whose 3/19/35/51-bit widths
        are `3 + 16 x (flags set)` and nothing else, and whose named spelling
        the table already carries on the same UFunction. The sixth is
        `AuthCurrentRandomSeed`, 120,853 rows of near-total distinctness across
        the full i32 range.

        63 -> 64 types `LocalizedStat` as `FText`. It was removed at 62 -> 61
        for being a wrong `FString`; it is back because a decoder now exists
        and the reason given for waiting was itself wrong -- `Statistic` was
        said to carry the same fact, but it decodes to a bare integer and no
        table in this repo maps those integers to names. 225 of 225 rows now
        decode to `EnemiesBlocked`, `HealingDone` and 17 more.

        61 -> 63 types Phoenix's `MulticastAddSmokeScreenPoint`. Viper's class
        declares the same RPC and was typed; Phoenix's was not, so 2,791 rows
        over 31 replays read null while decode errors stayed at 0, and Viper's
        working side made the ability look handled. The checksum fallback could
        not carry it -- different properties, different checksums -- and
        refusing rather than guessing is what made this a missing name.

        62 -> 61 drops `LocalizedStat`: typed `FString`, it decoded to null on
        3,011 of 3,011 rows because the wire is an `FText`. Removing a type that
        produces nothing is not a loss -- the bits stay in `raw_bits`.

        56 -> 62 added the six members of `AbilityCastsThisRound[].Effects[]`,
        the authoritative debuff log. `AffectedPlayer` resolves to a manifest
        actor 224/224 over exactly 10 players.

        49 -> 56 added the seven members of `AbilityCastsThisRound`, the
        per-cast ability log. One finding, seven fields, each checked against
        something outside itself -- `Player` matches a manifest subject 352/352,
        `Round` covers exactly 0..17.

        48 -> 49 added `CalloutRegionTrackingComponent.CurrentRegion`, the named
        map area a player is standing in: every one of its 1,957 non-zero rows
        resolves through `net_guids` to a `CalloutRegion_*` path.

        49 -> 48 removed one: `MagazineAmmo.AmmoCount`. The cooked game says
        that group is an `AmmoComponent`, which the replay declares with handle
        2 as `AuthResourceAmount`, so the leaf remap in `sink/paths.rs` now
        reaches a real declaration and the guessed name is gone.
        """
        self.assertEqual(len(atc.ADDITIONS), 70, atc.ADDITIONS)

    def test_handle_additions_stay_the_narrow_exception(self):
        """Same guardrail for the handle -> name additions.

        Each names a handle the replay leaves unnamed so the overlay can type
        it; the (group, field_name) must also appear in ADDITIONS, or the name
        resolves to nothing. Pinned exactly like ADDITIONS.

        Currently empty. Its one entry named `MagazineAmmo` handle 2 as
        `AmmoCount`, which the cooked game corrected: that group is an
        `AmmoComponent`, and the replay declares handle 2 on it as
        `AuthResourceAmount`. The leaf remap in `sink/paths.rs` reaches the real
        declaration, so the hand-written name is not needed. The mechanism stays
        because the next unnamed handle will not necessarily have a native group
        to borrow from.
        """
        self.assertEqual(len(atc.HANDLE_ADDITIONS), 0, atc.HANDLE_ADDITIONS)
        addition_keys = {(g, f) for g, f, _t in atc.ADDITIONS}
        for group, _handle, field in atc.HANDLE_ADDITIONS:
            self.assertIn(
                (group, field), addition_keys,
                f"handle addition {group}/{field} has no matching ADDITION type",
            )

    def test_every_addition_is_also_verified(self):
        """An addition absent from EXPECTED would apply once and never be checked.

        The whole `FieldType::...` is asserted, not its variant name: EXPECTED
        now carries full types so `verify` can compare them exactly, and the
        variant-only form could not tell `FieldType::Int32` from
        `FieldType::UInt32`.
        """
        for group, field, ftype in atc.ADDITIONS:
            self.assertIn((group, field, ftype), atc.EXPECTED)

    def _assert_inserted_in_order(self, content):
        keys = [(g, f) for g, f, _ in atc.parse_entries(content)]
        self.assertEqual(keys, sorted(keys), "table must stay sorted")
        for group, field, _ in atc.ADDITIONS:
            self.assertIn((group, field), keys)
        self.assertEqual(len(keys), len(WITHOUT) + N_ADDED)
        return keys

    def test_inserts_into_the_formatted_layout(self):
        out, n = atc.apply_additions(formatted(WITHOUT))
        self.assertEqual(n, N_ADDED)
        self._assert_inserted_in_order(out)

    def test_inserts_into_the_freshly_generated_one_line_layout(self):
        """The layout the script actually meets when run in documented order."""
        out, n = atc.apply_additions(one_line(WITHOUT))
        self.assertEqual(n, N_ADDED)
        self._assert_inserted_in_order(out)

    def test_is_idempotent(self):
        once, n1 = atc.apply_additions(formatted(WITHOUT))
        twice, n2 = atc.apply_additions(once)
        self.assertEqual((n1, n2), (N_ADDED, 0))
        self.assertEqual(once, twice)

    def test_verify_reports_a_missing_addition(self):
        problems = atc.verify(formatted(WITHOUT))
        self.assertTrue(
            any("LoadoutValue" in p and "BaseTeamState" in p for p in problems),
            problems,
        )

    def test_verify_passes_once_added(self):
        out, _ = atc.apply_additions(formatted(WITHOUT))
        remaining = [p for p in atc.verify(out) if "BaseTeamState" in p]
        self.assertEqual(remaining, [])

    def test_declared_length_is_resynced(self):
        out, _ = atc.apply_additions(formatted(WITHOUT))
        out = atc.resync_table_len(out)
        self.assertIn(
            f"[OverlayEntry; {len(WITHOUT) + len(atc.ADDITIONS)}]", out
        )

    def test_both_generated_header_lines_are_recounted(self):
        """The shape line went stale for exactly the reason the bucket line did.

        `// N entries from M groups.` is written by extract_descriptors.py from
        the descriptors it read, and ADDITIONS then inserts entries -- and, when
        an addition names a group no descriptor declared, a whole new group. The
        committed table said "1185 entries from 171 groups" above a bucket line
        that summed to 1188, over a slice declared 1188 long.
        """
        out, _ = atc.apply_additions(formatted(WITHOUT))
        out, lines = atc.rewrite_header(out)
        n_groups = len({g for g, _f in GROUPS})
        self.assertEqual(lines[0],
                         f"// {len(GROUPS)} entries from {n_groups} groups.")
        self.assertIn(lines[0], out)
        # The two lines must agree with each other and with the slice length.
        buckets = [int(n) for n in re.findall(r": (\d+)", lines[1])]
        self.assertEqual(sum(buckets), len(GROUPS))
        self.assertIn(f"[OverlayEntry; {len(GROUPS)}]",
                      atc.resync_table_len(out))

    def test_rewriting_the_header_is_idempotent(self):
        out, _ = atc.apply_additions(formatted(WITHOUT))
        once, lines1 = atc.rewrite_header(out)
        twice, lines2 = atc.rewrite_header(once)
        self.assertEqual((once, lines1), (twice, lines2))

    def test_a_missing_header_line_is_a_hard_failure(self):
        """Not a silent skip: a table without the line was not generated by
        extract_descriptors.py, and recounting it would say nothing."""
        out, _ = atc.apply_additions(formatted(WITHOUT))
        with self.assertRaises(SystemExit):
            atc.rewrite_header(out.replace("// 0 entries from 0 groups.\n", ""))

    def test_appends_past_the_end_of_the_slice(self):
        """An entry sorting after every existing entry is the new tail, not a
        fatal error.

        The first `];` in the final split block is OVERLAY_TABLE's close; the
        splice lands before it and the table stays sorted. The prior behavior
        was to refuse -- that blocked any addition whose group sorts last
        (e.g. ZoomMultiplierComponent, which is how the append path was
        forced into existence), so the append is handled now rather than
        rejected. The end-to-end insertion tests above also cross this path
        because the ZoomMultiplier additions sort past the synthetic tail
        bookend.
        """
        earlier = [("/AAAAA.First", "Field")]
        out, n = atc.apply_additions(formatted(earlier))
        self.assertEqual(n, len(atc.ADDITIONS))
        keys = [(g, f) for g, f, _ in atc.parse_entries(out)]
        self.assertEqual(keys, sorted(keys), "table must stay sorted after append")
        self.assertEqual(len(keys), 1 + len(atc.ADDITIONS))
        # OVERLAY_TABLE's closing bracket is still present exactly once.
        self.assertEqual(out.count("];\n"), 1)


#: A weapon group of the shape the "215"/"216" pass discovers for itself.
WEAPON_GROUP = "/Game/Equippables/Guns/Rifles/Vandal.Vandal_C"
WEAPON_ROWS = [
    (WEAPON_GROUP, "215", "FieldType::EnumRemainingBits"),
    (WEAPON_GROUP, "216", "FieldType::EnumRemainingBits"),
]


def whole_table(overrides=None, drop=(), one_line=False):
    """A complete table.rs that satisfies every correction, minus `overrides`.

    Derived from EXPECTED rather than hand-written, so it cannot go stale as
    corrections are added. `overrides` maps an EXPECTED key to the type the
    file should carry INSTEAD, which is how "regenerated but never corrected"
    is expressed. The two generated header lines and both slice lengths are
    written truthfully, so `main()` fails for the reason under test and not
    because the fixture is malformed.
    """
    overrides = overrides or {}
    rows = [
        (g, f, overrides.get((g, f), t))
        for g, f, t in list(atc.EXPECTED) + WEAPON_ROWS
        if (g, f) not in drop
    ]
    rows.sort()
    raw = sum(1 for _g, _f, t in rows if t == "FieldType::Raw")
    skip = sum(1 for _g, _f, t in rows if t == "FieldType::Skip")
    if one_line:
        body = "".join(
            f'    OverlayEntry {{ group_path: "{g}", field_name: "{f}", '
            f"field_type: {t} }},\n"
            for g, f, t in rows
        )
    else:
        body = "".join(
            "    OverlayEntry {\n"
            f'        group_path: "{g}",\n'
            f'        field_name: "{f}",\n'
            f"        field_type: {t},\n"
            "    },\n"
            for g, f, t in rows
        )
    return (
        "// GENERATED by tools/extract_descriptors.py -- do not edit by hand.\n"
        f"// {len(rows)} entries from {len({g for g, _f, _t in rows})} groups.\n"
        f"// Raw/Custom: {raw}, Skip: {skip}, Typed: {len(rows) - raw - skip}.\n"
        "\n"
        f"pub static OVERLAY_TABLE: [OverlayEntry; {len(rows)}] = [\n"
        f"{body}"
        "];\n"
        "\n"
        "pub static OVERLAY_HANDLE_TABLE: [OverlayHandleEntry; 0] = [\n"
        "];\n"
    )


#: The two mutations below are deliberately BUCKET-NEUTRAL -- they swap one
#: typed variant for another, so the generated header lines are identical
#: either way and `--check` cannot pass or fail for header reasons.
#:
#: `SmokeScreen.ReplicatedMovement` is rewritten by a block-based pass, so it
#: applies in BOTH layouts: a file carrying ShortComponents is correctable.
UNCORRECTED_SMOKESCREEN = {
    ("SmokeScreen", "ReplicatedMovement"):
        "FieldType::RepMovement { rotation: RotatorQuantization::ShortComponents }",
}
#: `TimedBomb.TimeRemainingToExplode` is rewritten by a pass that matches a
#: one-line literal, so in the rustfmt'd layout it is DEAD -- the file is not
#: correctable at all and has to fail loudly, which is the existing behaviour.
DEAD_FLOAT_TO_DOUBLE = {
    ("TimedBomb.TimedBomb_C", "TimeRemainingToExplode"): "FieldType::Float",
}


class MainOnDiskTests(unittest.TestCase):
    """`--check` verified the CORRECTED COPY, not the file on disk.

    It applied every correction to the in-memory content and then verified
    that, suppressing only the write. So it could not tell "table.rs is already
    corrected" from "table.rs is correctable and nobody corrected it" -- and CI
    runs exactly this command, so a regenerated table.rs committed without
    running the script goes green while the Rust build uses the uncorrected one.
    """

    def setUp(self):
        self._real_table = atc.TABLE_RS
        self._temp = tempfile.TemporaryDirectory()
        self.path = Path(self._temp.name) / "table.rs"
        atc.TABLE_RS = self.path
        self._real_argv = sys.argv

    def tearDown(self):
        atc.TABLE_RS = self._real_table
        sys.argv = self._real_argv
        self._temp.cleanup()

    def run_main(self, source, *args):
        """`(exit_code, stdout, stderr)` for one main() run over `source`."""
        self.path.write_text(source, encoding="utf-8")
        sys.argv = ["apply_type_corrections.py", *args]
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = atc.main()
        return code, out.getvalue(), err.getvalue()

    def test_a_correct_table_still_passes(self):
        code, out, err = self.run_main(whole_table(), "--check")
        self.assertEqual(code, 0, err)
        self.assertIn("verified", out)

    def test_a_correctable_but_uncorrected_file_fails(self):
        """The whole point. The passes CAN fix it, and nobody ran them."""
        code, _out, err = self.run_main(
            whole_table(UNCORRECTED_SMOKESCREEN), "--check"
        )
        self.assertEqual(code, 1, "--check must not pass an uncorrected file")
        self.assertIn("ShortComponents", err)

    def test_a_dead_pattern_still_fails_loudly(self):
        """Unchanged behaviour: a correction that cannot be applied at all."""
        code, _out, err = self.run_main(
            whole_table(DEAD_FLOAT_TO_DOUBLE), "--check"
        )
        self.assertEqual(code, 1)
        self.assertIn("TimeRemainingToExplode", err)

    def test_the_two_failure_modes_are_distinguishable(self):
        """A regenerated table trips BOTH at once -- some passes are dead in
        the rustfmt'd layout, the block-based ones apply fine in memory. One
        report that hides the other is how the second one gets missed."""
        both = {**UNCORRECTED_SMOKESCREEN, **DEAD_FLOAT_TO_DOUBLE}
        code, _out, err = self.run_main(whole_table(both), "--check")
        self.assertEqual(code, 1)
        self.assertIn("TimeRemainingToExplode", err)
        self.assertIn("ShortComponents", err)

    def test_check_writes_nothing(self):
        source = whole_table(UNCORRECTED_SMOKESCREEN)
        self.run_main(source, "--check")
        self.assertEqual(self.path.read_text(encoding="utf-8"), source)

    def test_applying_then_checking_passes(self):
        """The documented cure: run without --check, and --check goes green."""
        code, _out, err = self.run_main(whole_table(UNCORRECTED_SMOKESCREEN))
        self.assertEqual(code, 0, err)
        sys.argv = ["apply_type_corrections.py", "--check"]
        with contextlib.redirect_stdout(io.StringIO()), \
                contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(atc.main(), 0)

    def test_an_uncorrected_weapon_group_is_reported(self):
        """The 18 weapon groups had NO expectation of any kind.

        EXPECTED's "215"/"216" rows name five hardcoded non-weapon groups; the
        weapon pass discovers its own targets and nothing checked its result,
        so it could no-op in silence.
        """
        raw_weapons = {(WEAPON_GROUP, "215"): "FieldType::Raw"}
        code, _out, err = self.run_main(whole_table(raw_weapons), "--check")
        self.assertEqual(code, 1, "an uncorrected weapon group must be caught")
        self.assertIn(WEAPON_GROUP, err)

    def test_a_table_with_no_weapon_groups_at_all_is_reported(self):
        """The other shape of the same silence: the discovery finds nothing.

        `content.splitlines()` is how the pass finds its groups. If that ever
        stops matching, or the groups stop being emitted, "Applied 0" is the
        only trace -- so the absence itself has to be the failure.
        """
        code, _out, err = self.run_main(
            whole_table(drop=[(WEAPON_GROUP, "215"), (WEAPON_GROUP, "216")]),
            "--check",
        )
        self.assertEqual(code, 1)
        self.assertIn("/Game/Equippables/", err)

    def test_the_summary_counts_the_weapon_expectations_too(self):
        """The summary line is a claim about how much was checked."""
        _code, out, _err = self.run_main(whole_table(), "--check")
        self.assertIn(f"all {len(atc.EXPECTED) + len(WEAPON_ROWS)} ", out)


class VerifyMatchesTheWholeTypeTests(unittest.TestCase):
    """`verify` compared the required type as a SUBSTRING of the found one.

    `"Int32" in "FieldType::UInt32"` is True and `"Byte" in "FieldType::EnumByte"`
    is True, so an entry with the wrong signedness or the wrong byte variant
    verified clean -- the two mistakes a hand-written type table is most likely
    to make were the two it could not catch.
    """

    def _wrong_type(self, group, field, wrong):
        """One-entry table declaring a real EXPECTED key at the wrong type."""
        return formatted_typed([(group, field, wrong)])

    @staticmethod
    def _about(problems, field):
        """Only the problems naming `field` -- the rest of EXPECTED is absent
        from a one-entry table and would drown the assertion message."""
        return [p for p in problems if field in p]

    def test_a_wrong_signedness_is_reported(self):
        table = self._wrong_type(
            "/Script/ShooterGame.BaseTeamState", "LoadoutValue", "FieldType::UInt32"
        )
        about = self._about(atc.verify(table), "LoadoutValue")
        self.assertTrue(
            any("UInt32" in p for p in about),
            f"UInt32 where Int32 is required must be reported; got {about}",
        )

    def test_a_wrong_byte_variant_is_reported(self):
        group = ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator"
                 ".Comp_AbilityStatisticsReplicator_C")
        field = "Slot_12_22D571914FAFD5F0EBD400B7E2F28B36"
        about = self._about(
            atc.verify(self._wrong_type(group, field, "FieldType::EnumByte")), field
        )
        self.assertTrue(
            any("EnumByte" in p for p in about),
            f"EnumByte where Byte is required must be reported; got {about}",
        )

    def test_a_pre_existing_wrong_type_is_skipped_by_additions_and_reported(self):
        """The other half: ADDITIONS skips a key that is already there.

        So a table that already carries the key at the WRONG type is neither
        corrected nor -- before this -- reported. Skipping is the right call
        (silently overwriting a declaration the descriptors do make would be
        worse), which is exactly why the report has to fire.
        """
        table = self._wrong_type(
            "/Script/ShooterGame.BaseTeamState", "LoadoutValue", "FieldType::UInt32"
        )
        out, added = atc.apply_additions(table)
        self.assertEqual(added, len(atc.ADDITIONS) - 1, "the present key is skipped")
        about = self._about(atc.verify(out), "LoadoutValue")
        self.assertTrue(
            about, "the skipped, wrong-typed key must be reported; nothing was"
        )

    def test_the_committed_types_still_verify(self):
        """The tightening must not turn the real table's types into failures."""
        problems = atc.verify(atc.TABLE_RS.read_text(encoding="utf-8"))
        self.assertEqual(problems, [])


if __name__ == "__main__":
    unittest.main()
