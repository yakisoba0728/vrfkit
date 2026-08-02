import sys
import tempfile
import unittest
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq


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

class BlockPayloadExclusionTests(unittest.TestCase):
    marker = "__vrfkit_unresolved_class_net_cache_payload__"

    @staticmethod
    def write_fields(path: Path, rows: list[dict]) -> None:
        def values(name, default=None):
            return [row.get(name, default) for row in rows]

        table = pa.table(
            {
                "time_ms": pa.array(values("time_ms"), type=pa.uint32()),
                "packet_id": pa.array(values("packet_id"), type=pa.uint32()),
                "channel_index": pa.array(values("channel_index", 7), type=pa.uint32()),
                "actor_net_guid": pa.array(values("actor"), type=pa.uint32()),
                "object_net_guid": pa.array(values("object"), type=pa.uint32()),
                "group_path": pa.array(values("group_path"), type=pa.string()),
                "handle": pa.array(values("handle", 0), type=pa.uint32()),
                "field_name": pa.array(values("field_name"), type=pa.string()),
                "bit_count": pa.array(values("bit_count"), type=pa.uint32()),
                "raw_bits": pa.array(values("raw_bits"), type=pa.binary()),
                "value_i64": pa.array(values("value_i64"), type=pa.int64()),
                "value_f64": pa.array(values("value_f64"), type=pa.float64()),
                "value_bool": pa.array(values("value_bool"), type=pa.bool_()),
                "value_str": pa.array(values("value_str"), type=pa.string()),
            }
        )
        pq.write_table(table, path)

    @staticmethod
    def bundle_files(path: Path) -> dict[str, bytes]:
        return {item.name: item.read_bytes() for item in sorted(path.iterdir())}

    def test_block_payload_row_is_excluded_before_grouping_and_lifetimes(self):
        ordinary = [
            {
                "time_ms": 10,
                "packet_id": 1,
                "actor": 101,
                "group_path": "PlayerState",
                "field_name": "Health",
                "bit_count": 32,
                "value_i64": 100,
            },
            {
                "time_ms": 20,
                "packet_id": 2,
                "actor": 202,
                "group_path": "UnknownComponent",
                "field_name": None,
                "bit_count": 3,
                "raw_bits": b"\x05",
            },
        ]
        marker_row = {
            "time_ms": 30,
            "packet_id": 3,
            "actor": 303,
            "group_path": "AbilitiesAndBuffsComponent",
            "handle": (1 << 32) - 1,
            "field_name": self.marker,
            "bit_count": 5,
            "raw_bits": b"\x15",
        }

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            base_export = root / "base_export"
            marked_export = root / "marked_export"
            base_bundle = root / "base_bundle"
            marked_bundle = root / "marked_bundle"
            base_export.mkdir()
            marked_export.mkdir()
            self.write_fields(base_export / "fields.parquet", ordinary)
            self.write_fields(marked_export / "fields.parquet", ordinary + [marker_row])

            base_summary = bundle.convert(base_export, base_bundle)
            marked_summary = bundle.convert(marked_export, marked_bundle)

            self.assertEqual(marked_summary, base_summary)
            self.assertEqual(
                self.bundle_files(marked_bundle),
                self.bundle_files(base_bundle),
            )
            events = (marked_bundle / "events.ndjson").read_bytes()
            self.assertIn(b'"actor_net_guid":202', events)
            self.assertNotIn(b'"actor_net_guid":303', events)
            self.assertNotIn(self.marker.encode(), events)


class CombatReportLeafNameTests(unittest.TestCase):
    """The bundle keys combat-report leaves on the handle, not on the wire name.

    The parser labels each leaf with the name the replay declares. Two of those
    declarations would break this bundle if they reached it: Riot's own typos
    and 'b'-prefixed booleans are not what compute_metrics.py reads, and the
    quartet HUDConfig/StateRemainingTime/GameTime/GamePhase is declared at
    several handles in the SAME flattened element, so keying on the name would
    merge distinct values into one JSON key.
    """

    GROUP = ("/Game/GameModes/Bomb/Bomb_CombatReportComponent"
             ".Bomb_CombatReportComponent_C")

    def relabel(self, field_name, handle):
        return bundle._combat_report_leaf_name(self.GROUP, field_name, handle)

    def test_wire_spelling_is_mapped_back_to_the_reference_member_name(self):
        # Riot's typo, the 'b' prefix, and the Participant* prefix.
        self.assertEqual(
            self.relabel("Rounds[0].Reports[0].Interactions[0].DamageRecieved", 20),
            "Rounds[0].Reports[0].Interactions[0].DamageReceived",
        )
        self.assertEqual(
            self.relabel("Rounds[0].Reports[0].Interactions[0].bDidKill", 22),
            "Rounds[0].Reports[0].Interactions[0].DidKill",
        )
        self.assertEqual(
            self.relabel("Rounds[0].Reports[0].Interactions[0].ParticipantSubject", 11),
            "Rounds[0].Reports[0].Interactions[0].Subject",
        )
        self.assertEqual(
            self.relabel("Rounds[0].RoundNum", 3), "Rounds[0].RoundNumber",
        )

    def test_repeated_declared_names_stay_distinct_keys(self):
        # Handles 6, 99 and 105 ALL declare 'HUDConfig' at the Reports level.
        # Keying on the name would collapse three values into one.
        labels = {
            self.relabel("Rounds[0].Reports[0].HUDConfig", h) for h in (6, 99, 105)
        }
        self.assertEqual(
            labels,
            {
                "Rounds[0].Reports[0]._h6",
                "Rounds[0].Reports[0]._h99",
                "Rounds[0].Reports[0]._h105",
            },
        )

    def test_container_segments_and_foreign_groups_are_untouched(self):
        # Only the last segment moves; the container segments already match.
        self.assertEqual(
            self.relabel(
                "Rounds[0].Reports[0].Interactions[0].DealtInteractions[0]"
                ".Regions[0].bIsWallPen",
                48,
            ),
            "Rounds[0].Reports[0].Interactions[0].DealtInteractions[0]"
            ".Regions[0].IsWallPen",
        )
        # A different group with a same-shaped name is not rewritten.
        self.assertEqual(
            bundle._combat_report_leaf_name(
                "/Game/Something/Else_C", "Rounds[0].Reports[0].bDidKill", 22
            ),
            "Rounds[0].Reports[0].bDidKill",
        )
        # The bare array container row has no leaf segment to replace.
        self.assertEqual(self.relabel("Rounds", 2), "Rounds")

    def test_synthesised_rows_keep_the_parser_label(self):
        # emit_remaining_raw's row carries handle u32::MAX; rewriting it would
        # produce '_h4294967295'.
        self.assertEqual(
            self.relabel("Rounds[0]._raw", (1 << 32) - 1), "Rounds[0]._raw",
        )
        # The depth-limit row carries a CONTAINER handle and already has the
        # schema's name; rewriting it would produce '_h4'.
        self.assertEqual(
            self.relabel("Rounds[0].Reports", 4), "Rounds[0].Reports",
        )


if __name__ == "__main__":
    unittest.main()
