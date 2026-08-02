import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import to_valplay_bundle as bundle  # noqa: E402


class ShotEventTests(unittest.TestCase):
    def build_shot(self, scalar_params: dict) -> dict:
        return bundle._build_shot_event(
            bundle._ShotContext(tag_table={}),
            12788,
            4368,
            2,
            22,
            1,
            scalar_params,
            bundle._EffectBlobs(),
        )["shot"]

    def test_typed_and_raw_runtime_names_produce_identical_shot_geometry(self):
        raw = self.build_shot(
            {
                "248": {
                    "BitCount": 192,
                    "Data": "mpmZmZmVc8CkcD0KV5W9wM3MzMzMZHtA",
                },
                "249": {"BitCount": 35, "Data": "l/pPkgE="},
            }
        )
        typed = self.build_shot(
            {
                "248": "(-313.35,-7573.34,438.3)",
                "249": "rot(356.19324,141.4325,0)",
            }
        )

        self.assertEqual(typed["location"], raw["location"])
        self.assertEqual(typed["rotation"], raw["rotation"])
        self.assertEqual(
            typed["location"],
            {"x": -313.35, "y": -7573.34, "z": 438.3},
        )
        self.assertEqual(
            typed["rotation"],
            {
                "pitch": 356.1932373046875,
                "yaw": 141.4324951171875,
                "roll": 0.0,
            },
        )


class EffectBlobBitLengthTests(unittest.TestCase):
    """The bit length must come from the parser, not from the byte length.

    Parquet stores whole bytes, so a payload of N bits arrives as ceil(N/8)
    bytes with up to 7 padding bits in the last one. Deriving the length as
    `len(data) * 8` hands those padding bits to the decoder as data.

    Every effect blob measured -- 692,840 across the 11 cross-validated
    replays -- has `bit_count == len(raw_bits) * 8`, so the two readings agree
    on all real data and no corpus check can tell them apart. That is exactly
    why this needs a test: the wrong reading is currently invisible.
    """

    SPEC = bundle._EFFECT_FLOATS

    # A real FloatValues payload lifted from 02d4d478's fields.parquet: 50
    # bytes, declared 400 bits, four complete tag/value pairs. It is also the
    # first of the eight vectors pinned in crates/vrf-decode/src/effect.rs.
    BLOB = bytes.fromhex(
        "08021020390412400000803f000410200f0412400000a040"
        "000610203d0412400000803f000810203b04124015f9b3ce0000"
    )
    FOUR_PAIRS = {"284": 1.0, "263": 5.0, "286": 1.0, "285": -1509722752.0}
    THREE_PAIRS = {"284": 1.0, "263": 5.0, "286": 1.0}

    def decode(self, data: bytes, bit_count: int) -> dict:
        return bundle._decode_effect_blob(
            bundle._EffectBlob(data, bit_count), self.SPEC, {}
        )

    def test_declared_length_is_used_rather_than_the_byte_length(self):
        # Same 50 bytes of storage both times. Only the declared length
        # differs, and it is the declared length that must win: at 350 bits the
        # fourth pair is cut off and must not be decoded.
        #
        # This shape does not occur in the corpus -- every measured blob has
        # bit_count == len(data) * 8 -- so no corpus check can catch the wrong
        # reading. That is what makes it worth a test rather than a comment.
        self.assertEqual(self.decode(self.BLOB, 400), self.FOUR_PAIRS)
        self.assertEqual(self.decode(self.BLOB, 350), self.THREE_PAIRS)

    def test_byte_length_would_have_given_the_wrong_answer(self):
        # Spelled out as its own case so the regression is unmistakable: the
        # old code derived the length as len(data) * 8, which is 400 here, and
        # would have returned four pairs for a payload declaring 350 bits.
        self.assertEqual(len(self.BLOB) * 8, 400)
        self.assertNotEqual(self.decode(self.BLOB, 350), self.decode(self.BLOB, 400))

    def test_absent_blob_decodes_to_an_empty_mapping(self):
        self.assertEqual(bundle._decode_effect_blob(None, self.SPEC, {}), {})

    def test_blob_carries_its_own_bit_count(self):
        # The container must not let the two drift apart silently: a caller
        # that builds one has to supply both.
        blob = bundle._EffectBlob(b"\x00\x01", 9)
        self.assertEqual(blob.data, b"\x00\x01")
        self.assertEqual(blob.bit_count, 9)
        with self.assertRaises(TypeError):
            bundle._EffectBlob(b"\x00\x01")  # bit_count is not optional


if __name__ == "__main__":
    unittest.main()
