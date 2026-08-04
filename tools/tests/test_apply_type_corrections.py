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
import sys
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
    ("/Script/ShooterGame.ZebraComponent", "Stripes"),
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
        "// Raw/Custom: 0, Skip: 0, Typed: 0.\n"
        f"pub static OVERLAY_TABLE: [OverlayEntry; {len(pairs)}] = [\n"
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
            self.assertTrue(group.startswith("/"), group)
            self.assertTrue(field and not field.startswith("_"), field)
            self.assertTrue(ftype.startswith("FieldType::"), ftype)
            self.assertNotIn((group, field), seen, f"duplicate {group}/{field}")
            seen.add((group, field))

    def test_additions_stay_the_narrow_exception(self):
        """A guardrail on scope, not on the exact contents.

        These entries assert a type NO descriptor declares. If the list ever
        grows past a handful, the mechanism has quietly become a second,
        unreviewed descriptor source.
        """
        self.assertLessEqual(len(atc.ADDITIONS), 8, atc.ADDITIONS)

    def test_every_addition_is_also_verified(self):
        """An addition absent from EXPECTED would apply once and never be checked."""
        for group, field, ftype in atc.ADDITIONS:
            self.assertIn((group, field, ftype.split("::")[1]), atc.EXPECTED)

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

    def test_refuses_to_append_past_the_end_of_the_slice(self):
        """Everything sorting before the addition means there is no marker to
        insert in front of. Appending there would write into the closing `];`."""
        earlier = [("/Script/AAA.Group", "Field")]
        with self.assertRaises(SystemExit):
            atc.apply_additions(formatted(earlier))


if __name__ == "__main__":
    unittest.main()
