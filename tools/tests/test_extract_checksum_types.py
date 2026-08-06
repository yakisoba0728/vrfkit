"""Guards for the checksum-table generator.

`--check` demanded byte equality with a fresh render, which a content-addressed
table cannot deliver: a checksum is a property of the property, so a different
set of replays teaches a different *subset*, not a different answer. One export
learned 415 entries, seventy-one learned 442, and the committed file holds 417.
The check therefore failed for everyone, always, and a guard that cannot pass
is not a guard -- it reads as a broken generator and gets ignored.

What is worth catching is disagreement: a checksum both the file and the
manifests know, mapped to two different types. That is portable, because it
only ever compares the overlap.
"""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import extract_checksum_types as gen  # noqa: E402


class ReconcileTests(unittest.TestCase):
    def test_a_wider_basis_is_not_staleness(self):
        """Learning more than the file holds is coverage, not drift."""
        verdict = gen.reconcile(
            committed={1: "FieldType::Int32"},
            learned={1: "FieldType::Int32", 2: "FieldType::Float"},
        )
        self.assertEqual(verdict.disagreed, {})
        self.assertEqual(verdict.new, {2: "FieldType::Float"})
        self.assertTrue(verdict.ok)

    def test_a_narrower_basis_is_not_staleness_either(self):
        """One replay cannot teach what seventy-one did. That is not an error."""
        verdict = gen.reconcile(
            committed={1: "FieldType::Int32", 2: "FieldType::Float"},
            learned={1: "FieldType::Int32"},
        )
        self.assertEqual(verdict.disagreed, {})
        self.assertEqual(verdict.unseen, {2: "FieldType::Float"})
        self.assertTrue(verdict.ok)

    def test_a_type_that_changed_is_caught(self):
        """The one thing that is real drift."""
        verdict = gen.reconcile(
            committed={1: "FieldType::Int32"},
            learned={1: "FieldType::Float"},
        )
        self.assertEqual(
            verdict.disagreed, {1: ("FieldType::Int32", "FieldType::Float")})
        self.assertFalse(verdict.ok)

    def test_an_empty_basis_settles_nothing(self):
        verdict = gen.reconcile(committed={1: "FieldType::Int32"}, learned={})
        self.assertTrue(verdict.ok)
        self.assertEqual(verdict.unseen, {1: "FieldType::Int32"})


class MergeTests(unittest.TestCase):
    """Widening must not drop what a narrower basis cannot re-derive."""

    def test_merge_keeps_what_the_basis_did_not_see(self):
        merged = gen.merge({1: "FieldType::Int32"}, {2: "FieldType::Float"})
        self.assertEqual(merged, {1: "FieldType::Int32", 2: "FieldType::Float"})

    def test_merge_refuses_to_overwrite_a_disagreement(self):
        with self.assertRaises(ValueError):
            gen.merge({1: "FieldType::Int32"}, {1: "FieldType::Float"})


class ConflictTests(unittest.TestCase):
    """A dropped conflict never reached the verdict.

    `learn()` splits its findings into `resolved` and `conflicts`, and only
    `resolved` was reconciled against the committed file. `conflicts` was
    printed and counted and nothing else -- so when the manifests found a
    checksum whose donors RULE OUT the type the file commits, `--check` passed
    and `merge()` quietly kept the committed answer. Dropping a conflict is the
    right thing to do with NEW evidence; it is not a reason to stop looking at
    what is already written down.
    """

    def test_a_committed_type_the_evidence_rules_out_is_caught(self):
        verdict = gen.reconcile(
            committed={1: "FieldType::Float"},
            learned={},
            conflicts={1: (["FieldType::Int32", "FieldType::UInt32"], ["Foo"])},
        )
        self.assertIn(1, verdict.contradicted)
        self.assertFalse(verdict.ok)

    def test_a_committed_type_still_among_the_candidates_is_not_fatal(self):
        """Ambiguity is not contradiction.

        A committed entry learned from a narrower basis, where only one donor
        was visible, is exactly the "a narrower basis teaches less" case this
        module is built to tolerate. It is reported, not failed.
        """
        verdict = gen.reconcile(
            committed={1: "FieldType::Int32"},
            learned={},
            conflicts={1: (["FieldType::Int32", "FieldType::UInt32"], ["Foo"])},
        )
        self.assertEqual(verdict.contradicted, {})
        self.assertIn(1, verdict.ambiguous)
        self.assertTrue(verdict.ok)

    def test_a_conflict_the_file_never_committed_is_not_a_problem(self):
        """The safety property stays: an unwritten checksum stays unwritten."""
        verdict = gen.reconcile(
            committed={},
            learned={},
            conflicts={1: (["FieldType::Int32", "FieldType::UInt32"], ["Foo"])},
        )
        self.assertTrue(verdict.ok)
        self.assertEqual(verdict.ambiguous, {})

    def test_merge_refuses_to_carry_a_contradicted_entry_forward(self):
        """The write path never consults the verdict, so `merge` has to."""
        with self.assertRaises(ValueError):
            gen.merge(
                {1: "FieldType::Float"},
                {},
                conflicts={1: (["FieldType::Int32", "FieldType::UInt32"], ["Foo"])},
            )


class ParseTests(unittest.TestCase):
    def test_the_committed_table_parses(self):
        committed = gen.load_committed()
        self.assertGreater(len(committed), 300, "expected a populated table")
        for checksum, ftype in committed.items():
            self.assertIsInstance(checksum, int)
            self.assertTrue(ftype.startswith("FieldType::"), ftype)


if __name__ == "__main__":
    unittest.main()
