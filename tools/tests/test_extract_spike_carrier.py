"""Guards for the spike-custody derived view.

The script reads a NetGUID out of `raw_bits` by hand, because
`BombEquippable_C.Owner` has no overlay type entry and so no `value_i64`. That
decoder is the one place here that reimplements wire semantics rather than
joining columns, so it is the part that has to be pinned.

The vectors below are not this decoder's own output. Each is a real
`(raw_bits, value_i64)` pair from `Owner`/`Instigator`/`Controller`/
`AttachParent` rows on groups that vrfkit *does* type, so the expected value
comes from the Rust decoder, independently of the Python one.
"""
import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import extract_spike_carrier as spike  # noqa: E402


#: (packed bytes, NetGUID) taken from fields.parquet, expected value supplied
#: by vrfkit's own value_i64 column. One- and two-byte groups both appear.
VECTORS = [
    ("00", 0),
    ("04", 2),
    ("d4", 106),
    ("0102", 128),
    ("1902", 140),
    ("3102", 152),
    ("4902", 164),
]


class UnpackNetGuidTests(unittest.TestCase):
    def test_the_real_wire_vectors_decode_to_the_rust_values(self):
        for packed, expected in VECTORS:
            with self.subTest(packed=packed):
                self.assertEqual(
                    spike.unpack_netguid(bytes.fromhex(packed)), expected)

    def test_an_empty_payload_is_none_not_zero(self):
        """A missing value must not be confused with NetGUID 0."""
        self.assertIsNone(spike.unpack_netguid(b""))

    def test_the_continuation_bit_stops_the_walk(self):
        """Bit 0 clear ends the value; trailing bytes belong to another field."""
        self.assertEqual(spike.unpack_netguid(bytes.fromhex("04ff")), 2)

    def test_groups_are_little_endian(self):
        """0x1902 is 12 + (1 << 7); big-endian would give 1 + (12 << 7)."""
        self.assertEqual(spike.unpack_netguid(bytes.fromhex("1902")), 140)
        self.assertNotEqual(spike.unpack_netguid(bytes.fromhex("1902")), 1537)


class LeafTests(unittest.TestCase):
    def test_a_class_path_reduces_to_its_last_segment(self):
        self.assertEqual(
            spike.leaf("/Game/GameModes/Bomb/BombEquippable.BombEquippable_C"),
            "BombEquippable.BombEquippable_C")

    def test_an_absent_class_is_the_empty_string(self):
        self.assertEqual(spike.leaf(None), "")


class NetGuidAtTests(unittest.TestCase):
    def test_the_typed_column_wins_when_it_is_populated(self):
        self.assertEqual(
            spike.netguid_at([4242], [bytes.fromhex("04")], 0), 4242)

    def test_the_packed_bits_are_read_when_the_column_is_null(self):
        self.assertEqual(
            spike.netguid_at([None], [bytes.fromhex("1902")], 0), 140)

    def test_a_null_column_with_no_bits_is_none(self):
        self.assertIsNone(spike.netguid_at([None], [None], 0))


if __name__ == "__main__":
    unittest.main()
