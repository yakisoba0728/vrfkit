import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import to_valplay_bundle as bundle  # noqa: E402


def write_fields_parquet(path: Path, rows: list[dict]) -> None:
    """Write a fields.parquet with the column set the bundle reads.

    Module level so every test class can build an export; `rows` carries only
    the columns a case cares about and the rest default to null.
    """
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


#: One innocuous replicated property, so `convert` has a fields.parquet to read
#: when the case under test is about some other table.
MINIMAL_FIELD_ROWS = [
    {
        "time_ms": 10,
        "packet_id": 1,
        "actor": 101,
        "group_path": "PlayerState",
        "field_name": "Health",
        "bit_count": 32,
        "value_i64": 100,
    },
]


def write_movement_parquet(path: Path, rows: list[dict]) -> None:
    """Write a movement.parquet with the columns `_write_movement` reads."""
    def values(name, default=0.0):
        return [row.get(name, default) for row in rows]

    def u32(name):
        return pa.array([row.get(name, 0) for row in rows], type=pa.uint32())

    table = pa.table(
        {
            "time_ms": u32("time_ms"),
            "packet_id": u32("packet_id"),
            "character_net_guid": u32("char"),
            "pos_x": pa.array(values("pos_x"), type=pa.float32()),
            "pos_y": pa.array(values("pos_y"), type=pa.float32()),
            "pos_z": pa.array(values("pos_z"), type=pa.float32()),
            "yaw": pa.array(values("yaw"), type=pa.float32()),
            "pitch": pa.array(values("pitch"), type=pa.float32()),
            "vel_x": pa.array(values("vel_x"), type=pa.float32()),
            "vel_y": pa.array(values("vel_y"), type=pa.float32()),
            "vel_z": pa.array(values("vel_z"), type=pa.float32()),
        }
    )
    pq.write_table(table, path)


class MovementCollapseTests(unittest.TestCase):
    """The bundle keeps the final move PER PACKET, which is what the reference has.

    Every sub-move decoded out of one movement RPC is stamped with the
    time_ms and packet_id the sink hoisted before the loop
    (vrfkit/src/sink/stream.rs, decode_movement_rpc), so all sub-moves of one
    packet share both. Collapsing on the millisecond therefore also merges
    two DIFFERENT packets that land in the same millisecond, and the earlier
    packet's final move -- a real, distinct sample -- disappears.
    """

    @staticmethod
    def convert_movement(root: Path, rows: list[dict]) -> list[str]:
        export = root / "export"
        out = root / "bundle"
        export.mkdir()
        write_fields_parquet(export / "fields.parquet", MINIMAL_FIELD_ROWS)
        write_movement_parquet(export / "movement.parquet", rows)
        bundle.convert(export, out)
        text = (out / "movement.ndjson").read_text(encoding="utf-8")
        return text.splitlines()

    def test_two_packets_in_one_millisecond_each_keep_their_final_move(self):
        rows = [
            # Packet 1: two sub-moves, only the second survives.
            {"time_ms": 100, "packet_id": 1, "char": 42, "pos_x": 1.0},
            {"time_ms": 100, "packet_id": 1, "char": 42, "pos_x": 2.0},
            # Packet 2, same millisecond, same character: a separate final
            # move that must NOT be treated as packet 1's sub-move.
            {"time_ms": 100, "packet_id": 2, "char": 42, "pos_x": 3.0},
        ]
        with tempfile.TemporaryDirectory() as tmp:
            lines = self.convert_movement(Path(tmp), rows)

        self.assertEqual(len(lines), 2, lines)
        self.assertIn('"x":2', lines[0])
        self.assertIn('"x":3', lines[1])

    def test_sub_moves_within_one_packet_are_still_collapsed(self):
        rows = [
            {"time_ms": 100, "packet_id": 1, "char": 42, "pos_x": 1.0},
            {"time_ms": 100, "packet_id": 1, "char": 42, "pos_x": 2.0},
            {"time_ms": 100, "packet_id": 1, "char": 43, "pos_x": 9.0},
        ]
        with tempfile.TemporaryDirectory() as tmp:
            lines = self.convert_movement(Path(tmp), rows)

        self.assertEqual(len(lines), 2, lines)
        self.assertIn('"x":2', lines[0])
        self.assertIn('"x":9', lines[1])


class MovementTruncationTests(unittest.TestCase):
    """A replay with no movement table must not inherit the previous one's.

    `convert` reuses an existing output directory, so converting replay B over
    replay A's bundle left A's movement.ndjson sitting beside B's events while
    the run reported success.
    """

    def test_missing_movement_table_truncates_a_reused_bundle_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with_mv = root / "with_movement"
            without_mv = root / "without_movement"
            out = root / "bundle"
            with_mv.mkdir()
            without_mv.mkdir()
            write_fields_parquet(with_mv / "fields.parquet", MINIMAL_FIELD_ROWS)
            write_fields_parquet(without_mv / "fields.parquet", MINIMAL_FIELD_ROWS)
            write_movement_parquet(
                with_mv / "movement.parquet",
                [{"time_ms": 100, "packet_id": 1, "char": 42, "pos_x": 1.0}],
            )

            bundle.convert(with_mv, out)
            self.assertNotEqual(
                (out / "movement.ndjson").read_text(encoding="utf-8"), ""
            )

            summary = bundle.convert(without_mv, out)

            self.assertEqual(summary["movement_written"], 0)
            self.assertTrue(
                (out / "movement.ndjson").exists(),
                "movement.ndjson was neither written nor truncated",
            )
            self.assertEqual(
                (out / "movement.ndjson").read_text(encoding="utf-8"), ""
            )


class TransactionalConversionTests(unittest.TestCase):
    @staticmethod
    def snapshot(path: Path) -> dict[str, bytes]:
        return {
            item.relative_to(path).as_posix(): item.read_bytes()
            for item in sorted(path.rglob("*"))
            if item.is_file()
        }

    @staticmethod
    def make_export(path: Path) -> bytes:
        path.mkdir(parents=True)
        write_fields_parquet(path / "fields.parquet", MINIMAL_FIELD_ROWS)
        manifest = b'{"replay_version":"source","source_file":"match.vrf"}\n'
        (path / "manifest.json").write_bytes(manifest)
        return manifest

    def test_input_and_output_may_not_be_the_same_directory(self):
        with tempfile.TemporaryDirectory() as temp:
            export = Path(temp) / "export"
            source_manifest = self.make_export(export)
            before = self.snapshot(export)

            with self.assertRaises(ValueError):
                bundle.convert(export, export)

            self.assertEqual(self.snapshot(export), before)
            self.assertEqual((export / "manifest.json").read_bytes(), source_manifest)

    def test_output_nested_inside_input_is_rejected_before_writing(self):
        with tempfile.TemporaryDirectory() as temp:
            export = Path(temp) / "export"
            self.make_export(export)
            output = export / "bundle"

            with self.assertRaises(ValueError):
                bundle.convert(export, output)

            self.assertFalse(output.exists())

    def test_input_nested_inside_output_is_rejected_before_writing(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "output"
            export = output / "export"
            self.make_export(export)
            before = self.snapshot(output)

            with self.assertRaises(ValueError):
                bundle.convert(export, output)

            self.assertEqual(self.snapshot(output), before)

    def test_conversion_failure_preserves_an_existing_complete_bundle(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            export = root / "export"
            export.mkdir()
            (export / "fields.parquet").write_bytes(b"not parquet")
            (export / "manifest.json").write_text(
                '{"replay_version":"new"}', encoding="utf-8"
            )
            output = root / "bundle"
            output.mkdir()
            for name, content in {
                "manifest.json": "{\"replay_version\":\"old\"}",
                "events.ndjson": "{\"type\":\"old\"}\n",
                "movement.ndjson": "",
                ".complete": "old marker",
            }.items():
                (output / name).write_text(content, encoding="utf-8")
            before = self.snapshot(output)

            with self.assertRaises(Exception):
                bundle.convert(export, output)

            self.assertEqual(self.snapshot(output), before)
            self.assertEqual(
                [p for p in root.iterdir() if p.name.startswith(".bundle.")], []
            )

    def test_success_never_modifies_the_source_manifest(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            export = root / "export"
            original = self.make_export(export)

            bundle.convert(export, root / "bundle")

            self.assertEqual((export / "manifest.json").read_bytes(), original)

    def test_backup_cleanup_failure_does_not_turn_a_committed_publish_into_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            staging = root / ".bundle.staging"
            output = root / "bundle"
            staging.mkdir()
            output.mkdir()
            (staging / "manifest.json").write_text("new", encoding="utf-8")
            (output / "manifest.json").write_text("old", encoding="utf-8")

            real_remove_tree = bundle.remove_tree

            def fail_cleanup(path, parent):
                raise OSError(f"cannot remove {path} under {parent}")

            bundle.remove_tree = fail_cleanup
            stderr = io.StringIO()
            try:
                with contextlib.redirect_stderr(stderr):
                    bundle._publish_bundle(staging, output)
            finally:
                bundle.remove_tree = real_remove_tree

            self.assertEqual(
                (output / "manifest.json").read_text(encoding="utf-8"), "new"
            )
            backups = list(root.glob(".bundle.backup.*"))
            self.assertEqual(len(backups), 1, backups)
            self.assertEqual(
                (backups[0] / "manifest.json").read_text(encoding="utf-8"), "old"
            )
            self.assertIn("backup", stderr.getvalue().lower())


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
            # The DATA files must be byte-identical: the marker row changes
            # nothing a consumer reads as an event or a position.
            for name in ("events.ndjson", "movement.ndjson"):
                self.assertEqual(
                    (marked_bundle / name).read_bytes(),
                    (base_bundle / name).read_bytes(),
                    name,
                )
            # The manifest must NOT be identical. The row was on disk and was
            # deliberately skipped; a manifest that read the same either way
            # would be a bundle that cannot say how much preservation data its
            # export carried.
            base_manifest = json.loads(
                (base_bundle / "manifest.json").read_text(encoding="utf-8")
            )["adapter"]
            marked_manifest = json.loads(
                (marked_bundle / "manifest.json").read_text(encoding="utf-8")
            )["adapter"]
            self.assertEqual(base_manifest["field_rows_read"], 2)
            self.assertEqual(base_manifest["field_rows_unresolved_class_net_cache"], 0)
            self.assertEqual(marked_manifest["field_rows_read"], 3)
            self.assertEqual(marked_manifest["field_rows_unresolved_class_net_cache"], 1)
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


class TallyTestCase(unittest.TestCase):
    """Shared plumbing for the cases that read the conversion's loss counters."""

    RPC_GROUP = "/Script/ShooterGame.DamageableComponent_ClassNetCache"

    def convert_rows(self, tmp: str, rows: list[dict], **files) -> dict:
        root = Path(tmp)
        export = root / "export"
        export.mkdir()
        write_fields_parquet(export / "fields.parquet", rows)
        for name, text in files.items():
            (export / name.replace("_", ".")).write_text(text, encoding="utf-8")
        return bundle.convert(export, root / "bundle")

    def tally_of(self, tmp: str, rows: list[dict], **files) -> dict:
        return self.convert_rows(tmp, rows, **files)["tally"]

    def events_of(self, tmp: str, event_type: str) -> list[dict]:
        """Every event of one type from the bundle `convert_rows` just wrote.

        The bundle also carries actor_spawned/actor_closed for each actor it
        saw, so a bare line count says nothing about the events under test.
        """
        text = (Path(tmp) / "bundle" / "events.ndjson").read_text(encoding="utf-8")
        events = [json.loads(line) for line in text.splitlines()]
        return [e for e in events if e["type"] == event_type]


class UnnamedRowTallyTests(TallyTestCase):
    """A row the parser could not name is a dropped row, and must be counted.

    Both drop sites are silent today: a property group containing one becomes
    an empty but valid-looking event, and an RPC whose rows are ALL unnamed is
    dropped whole because no field_name ever supplies the function name. The
    documented reference export carries 1,996 such rows and the bundle never
    said so.
    """

    def test_an_unnamed_property_row_is_counted(self):
        rows = [
            {
                "time_ms": 10, "packet_id": 1, "actor": 101,
                "group_path": "PlayerState", "field_name": "Health",
                "bit_count": 32, "value_i64": 100,
            },
            {
                "time_ms": 10, "packet_id": 1, "actor": 101,
                "group_path": "PlayerState", "field_name": None,
                "bit_count": 3, "raw_bits": b"\x05",
            },
        ]
        with tempfile.TemporaryDirectory() as tmp:
            tally = self.tally_of(tmp, rows)
        self.assertEqual(tally["unnamed_property_rows"], 1)

    def test_an_rpc_of_only_unnamed_rows_counts_the_rows_and_the_invocation(self):
        rows = [
            {
                "time_ms": 20, "packet_id": 2, "actor": 202,
                "group_path": self.RPC_GROUP, "handle": 5,
                "field_name": None, "bit_count": 3, "raw_bits": b"\x05",
            },
            {
                "time_ms": 20, "packet_id": 2, "actor": 202,
                "group_path": self.RPC_GROUP, "handle": 5,
                "field_name": None, "bit_count": 4, "raw_bits": b"\x06",
            },
        ]
        with tempfile.TemporaryDirectory() as tmp:
            summary = self.convert_rows(tmp, rows)
            # The invocation really is gone -- the count is its only trace.
            self.assertEqual(self.events_of(tmp, "rpc_received"), [])
        self.assertEqual(summary["tally"]["unnamed_rpc_rows"], 2)
        self.assertEqual(summary["tally"]["unnamed_rpc_invocations"], 1)


class ManifestTallyTests(TallyTestCase):
    """A substituted manifest must not read as a real one.

    With no manifest the gameplay-tag table is empty, so every effect blob is
    keyed by its numeric tag index instead of a name like
    'FiringState.AmmoRemaining' -- and every shot then reports null ammo,
    firing state, player and attack vectors while the run says SUCCESS.
    """

    SHOT_RPC = "/Script/ShooterGame.ShooterCharacter_ClassNetCache"

    def shot_rows(self) -> list[dict]:
        return [
            {
                "time_ms": 30, "packet_id": 3, "actor": 2, "object": 22,
                "channel_index": 1, "group_path": self.SHOT_RPC, "handle": 9,
                "field_name": "ReplayPlayContinuousEffectAtLocation.FloatValues",
                "bit_count": len(EffectBlobBitLengthTests.BLOB) * 8,
                "raw_bits": EffectBlobBitLengthTests.BLOB,
            },
        ]

    def test_a_missing_manifest_is_counted(self):
        with tempfile.TemporaryDirectory() as tmp:
            tally = self.tally_of(tmp, MINIMAL_FIELD_ROWS)
        self.assertEqual(tally["missing_manifest"], 1)

    def test_a_present_manifest_is_not_counted(self):
        manifest = '{"replay_version": "x", "duration_ms": 5}'
        with tempfile.TemporaryDirectory() as tmp:
            tally = self.tally_of(tmp, MINIMAL_FIELD_ROWS, manifest_json=manifest)
        self.assertEqual(tally["missing_manifest"], 0)

    def test_shots_decoded_with_no_tag_table_are_counted(self):
        with tempfile.TemporaryDirectory() as tmp:
            tally = self.tally_of(tmp, self.shot_rows())
        self.assertEqual(tally["empty_gameplay_tag_table"], 1)

    def test_a_tag_table_from_the_manifest_clears_the_count(self):
        manifest = json.dumps({
            "net_field_export_groups": [
                {
                    "path": "NetworkGameplayTagNodeIndex",
                    "fields": [{"handle": 263, "name": "FiringState.AmmoRemaining"}],
                }
            ]
        })
        with tempfile.TemporaryDirectory() as tmp:
            tally = self.tally_of(tmp, self.shot_rows(), manifest_json=manifest)
        self.assertEqual(tally["empty_gameplay_tag_table"], 0)


class RpcCollisionTallyTests(TallyTestCase):
    """Two invocations of one function by one actor in one packet collide.

    The RPC group key is (packet_id, actor, group_path, handle), so both calls
    land in one group, the second call's parameters overwrite the first's, and
    a single rpc_received comes out. Nothing can un-interleave them from the
    export, so the fix is to say it happened, not to guess a boundary.
    """

    def collided_rows(self) -> list[dict]:
        common = {
            "time_ms": 40, "packet_id": 4, "actor": 404,
            "group_path": self.RPC_GROUP, "handle": 12,
            "field_name": "MulticastNotifyKilledEnemy.MultikillLevel",
        }
        return [
            {**common, "bit_count": 8, "value_i64": 1},
            {**common, "bit_count": 8, "value_i64": 2},
        ]

    def test_a_repeated_parameter_in_one_group_is_counted(self):
        with tempfile.TemporaryDirectory() as tmp:
            summary = self.convert_rows(tmp, self.collided_rows())
            # One rpc_received for what were two calls, carrying only the
            # second call's value -- the loss the count names.
            rpcs = self.events_of(tmp, "rpc_received")
        self.assertEqual(summary["tally"]["rpc_param_collisions"], 1)
        self.assertEqual(len(rpcs), 1, rpcs)
        self.assertEqual(rpcs[0]["payload"], {"MultikillLevel": 2})

    def test_distinct_parameters_in_one_group_are_not_counted(self):
        common = {
            "time_ms": 40, "packet_id": 4, "actor": 404,
            "group_path": self.RPC_GROUP, "handle": 12, "bit_count": 8,
        }
        rows = [
            {**common, "field_name": "MulticastNotifyKilledEnemy.KillerCharacter",
             "value_i64": 1},
            {**common, "field_name": "MulticastNotifyKilledEnemy.KilledCharacter",
             "value_i64": 2},
        ]
        with tempfile.TemporaryDirectory() as tmp:
            tally = self.tally_of(tmp, rows)
        self.assertEqual(tally["rpc_param_collisions"], 0)


class FlatPathTallyTests(unittest.TestCase):
    """A path segment the parser cannot read becomes a literal key, silently.

    'Rounds[0][1].Damage' yields the literal object key 'Rounds[0][1]', and
    two rows whose shapes disagree ('Foo' and 'Foo.Bar') destroy each other
    depending on arrival order. Both produce valid JSON and a successful
    count, so a counter is the only thing that can report them.
    """

    def test_an_unparsable_segment_is_counted(self):
        tally = bundle._Tally()
        parts = bundle._parse_field_path("Rounds[0][1].Damage", tally)
        self.assertEqual(parts[0], ("Rounds[0][1]", None))
        self.assertEqual(tally["unparsable_path_segments"], 1)

    def test_a_bare_numeric_segment_is_not_counted(self):
        # '248' is the documented spelling of an unnamed handle, not a parse
        # failure; counting it would drown the real ones.
        tally = bundle._Tally()
        self.assertEqual(bundle._parse_field_path("248", tally), [("248", None)])
        self.assertEqual(tally["unparsable_path_segments"], 0)

    def test_a_blueprint_name_with_spaces_is_not_counted(self):
        # Measured on out/baseline: 449 rows carry a segment that is neither
        # an identifier nor a number, and all 41 distinct spellings are
        # Blueprint display names like these -- none has a bracket in it. A
        # name with a space is a leaf, and a literal key is the RIGHT
        # representation of a leaf, so counting these would put a
        # three-figure number in every clean conversion's summary and teach
        # the reader to skip the block.
        tally = bundle._Tally()
        for name in ("Victim FXC", "Set skeletal Collision", "Socket Name"):
            self.assertEqual(bundle._parse_field_path(name, tally),
                             [(name, None)])
        self.assertEqual(tally["unparsable_path_segments"], 0)

    def test_a_segment_whose_subscripts_did_not_parse_is_counted(self):
        # The real failure: bracket structure the parser could not read. The
        # nesting it describes is silently flattened into one literal key.
        tally = bundle._Tally()
        parts = bundle._parse_field_path("Rounds[0][1]", tally)
        self.assertEqual(parts, [("Rounds[0][1]", None)])
        self.assertEqual(tally["unparsable_path_segments"], 1)

    def test_a_scalar_overwritten_by_a_nested_value_is_counted(self):
        tally = bundle._Tally()
        payload = {}
        bundle._set_nested(payload, bundle._parse_field_path("Foo"), 1, tally)
        bundle._set_nested(payload, bundle._parse_field_path("Foo.Bar"), 2, tally)
        self.assertEqual(payload, {"Foo": {"Bar": 2}})
        self.assertEqual(tally["payload_shape_conflicts"], 1)

    def test_a_nested_value_overwritten_by_a_scalar_is_counted(self):
        tally = bundle._Tally()
        payload = {}
        bundle._set_nested(payload, bundle._parse_field_path("Foo.Bar"), 2, tally)
        bundle._set_nested(payload, bundle._parse_field_path("Foo"), 1, tally)
        self.assertEqual(payload, {"Foo": 1})
        self.assertEqual(tally["payload_shape_conflicts"], 1)

    def test_ordinary_nesting_is_not_counted(self):
        tally = bundle._Tally()
        payload = {}
        for path, value in (("Rounds[0].Damage", 5), ("Rounds[1].Damage", 7),
                            ("Rounds[0].Kills", 2), ("Plain", 1)):
            bundle._set_nested(payload, bundle._parse_field_path(path), value, tally)
        self.assertEqual(tally["payload_shape_conflicts"], 0)
        self.assertEqual(tally["unparsable_path_segments"], 0)

    def test_life_change_child_rows_are_still_dropped(self):
        # Guard, not a new claim: these children arrive spelled with '[0]'
        # and must never reach the payload as flat keys.
        for param in ("LifeChangeEvents[0].LifeResult",
                      "LifeChangeBySection[1].Amount"):
            self.assertIsNone(
                bundle._normalize_rpc_param(
                    "MulticastNotifyDamage_Point", param, 3, False
                )
            )


class TypedColumnTallyTests(unittest.TestCase):
    """More than one typed column set is a writer regression, not a value.

    _get_value picks i64, then f64, then bool, then string. If a regression
    filled value_i64=1 and value_bool=False, the bundle emits 1, discards the
    boolean and completes normally.
    """

    def test_two_populated_typed_columns_are_counted(self):
        tally = bundle._Tally()
        value, is_raw = bundle._get_value(1, None, False, None, None, None, tally)
        self.assertEqual((value, is_raw), (1, False))
        self.assertEqual(tally["multi_typed_rows"], 1)

    def test_one_populated_typed_column_is_not_counted(self):
        tally = bundle._Tally()
        for args in ((7, None, None, None, None, None),
                     (None, 1.5, None, None, None, None),
                     (None, None, True, None, None, None),
                     (None, None, None, "s", None, None),
                     (None, None, None, None, b"\x01", 8)):
            bundle._get_value(*args, tally)
        self.assertEqual(tally["multi_typed_rows"], 0)

    def test_a_typed_value_beside_raw_bits_is_not_counted(self):
        # raw_bits travels alongside decoded values by design; only the four
        # TYPED columns are meant to be mutually exclusive.
        tally = bundle._Tally()
        bundle._get_value(1, None, None, None, b"\x01", 8, tally)
        self.assertEqual(tally["multi_typed_rows"], 0)


class FabricatedLocationTests(TallyTestCase):
    """A shot with no readable location gets the world origin -- say so.

    _parse_vector_or_zero's docstring claimed an upstream filter guarantees a
    location is present. There is no such filter: _build_rpc_events emits
    every ReplayPlayContinuousEffectAtLocation invocation and says so in its
    own comment ('No blob guard'), so the fabricated origin is reachable.
    """

    def build_shot(self, scalar_params: dict, tally):
        return bundle._build_shot_event(
            bundle._ShotContext(tag_table={}), 1, 2, 3, 4, 5,
            scalar_params, bundle._EffectBlobs(), tally=tally,
        )["shot"]

    def test_a_shot_with_no_location_counts_a_fabricated_origin(self):
        tally = bundle._Tally()
        shot = self.build_shot({}, tally)
        self.assertEqual(shot["location"], {"x": 0, "y": 0, "z": 0})
        self.assertEqual(tally["fabricated_shot_locations"], 1)

    def test_an_unparsable_location_counts_a_fabricated_origin(self):
        tally = bundle._Tally()
        self.assertEqual(
            self.build_shot({"Location": "(1,2)"}, tally)["location"],
            {"x": 0, "y": 0, "z": 0},
        )
        self.assertEqual(tally["fabricated_shot_locations"], 1)

    def test_a_real_location_counts_nothing(self):
        tally = bundle._Tally()
        self.assertEqual(
            self.build_shot({"Location": "(1,2,3)"}, tally)["location"],
            {"x": 1, "y": 2, "z": 3},
        )
        self.assertEqual(tally["fabricated_shot_locations"], 0)

    def test_a_genuine_origin_shot_counts_nothing(self):
        # (0,0,0) parsed from the wire is a real position, not a fabrication.
        tally = bundle._Tally()
        self.assertEqual(
            self.build_shot({"Location": "(0,0,0)"}, tally)["location"],
            {"x": 0, "y": 0, "z": 0},
        )
        self.assertEqual(tally["fabricated_shot_locations"], 0)


class SummaryReportingTests(TallyTestCase):
    """The summary must not say 'complete' about a conversion that lost rows."""

    def test_a_clean_conversion_reports_no_losses(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest = '{"replay_version": "x"}'
            summary = self.convert_rows(
                tmp, MINIMAL_FIELD_ROWS, manifest_json=manifest
            )
        self.assertEqual(summary["tally"].total, 0)

    def test_a_lossy_conversion_names_every_loss_in_its_summary(self):
        rows = MINIMAL_FIELD_ROWS + [
            {
                "time_ms": 10, "packet_id": 1, "actor": 101,
                "group_path": "PlayerState", "field_name": None,
                "bit_count": 3, "raw_bits": b"\x05",
            },
        ]
        with tempfile.TemporaryDirectory() as tmp:
            summary = self.convert_rows(tmp, rows)
        lines = summary["tally"].lines()
        self.assertTrue(any("unnamed_property_rows" in ln for ln in lines), lines)
        self.assertTrue(any("missing_manifest" in ln for ln in lines), lines)
        # Only the non-zero counters are printed.
        self.assertEqual(len(lines), 2, lines)


# ---------------------------------------------------------------------------
# The seam
#
# Everything below tests the contract between this repository and valplay:
# what the bundle manifest carries, what it deliberately does not, and the
# order events.ndjson is written in. valplay reads all of it and has no way to
# check any of it -- these are the assertions that fail HERE when a change
# would have broken it silently over there.
# ---------------------------------------------------------------------------

#: A quality object shaped like the one crates/vrfkit/src/manifest.rs emits.
#: Trimmed to the members the seam actually reads plus enough of the rest to
#: prove the forwarding is verbatim and not a hand-picked subset.
UPSTREAM_QUALITY = {
    "content_blocks_lost": 0,
    "chunks_processed": 19,
    "export_groups": 475,
    "movement_rows": 3,
    "net_guid_rows": 2,
    "event_rows": 195,
    "event_trailing_bytes": 0,
    "replay_data_trailing_bytes": 0,
    "event_layout_mismatches": 0,
    "event_first_layout_mismatch": None,
    "overlay_error_buckets": 0,
    "overlay_errors_reported": 0,
    "checkpoints_enabled": False,
    "net": {"packets": 530401, "malformed_packets": 0, "skipped_bits": 19135006},
    "sink": {"overlay_decoded_ok": 742738, "struct_blob_first_error": None},
    "checkpoints": None,
}

UPSTREAM_GROUPS = [
    {
        "path": "/Script/ShooterGame.OwnerExclusivePlayerInfo",
        "path_name_index": 12,
        "fields": [
            {"handle": 40, "name": "RoundNumber", "compatible_checksum": 1},
            {"handle": 41, "name": "StartOfRoundMoney", "compatible_checksum": 2},
        ],
    },
]

#: Two account UUIDs, shaped like the ones vrfkit's manifest `players` array
#: carries. Present in the EXPORT manifest so the omission test has something
#: real to prove is absent from the bundle.
UPSTREAM_PLAYERS = [
    {
        "actor_net_guid": 101,
        "subject": "11111111-2222-3333-4444-555555555555",
        "character_net_guid": 501,
    },
]


def write_net_guids_parquet(path: Path, rows: list[dict]) -> None:
    """Write a net_guids.parquet with the columns `_load_net_guids` reads."""
    table = pa.table(
        {
            "net_guid": pa.array([r["net_guid"] for r in rows], type=pa.uint32()),
            "outer_net_guid": pa.array(
                [r.get("outer") for r in rows], type=pa.uint32()
            ),
            "path": pa.array([r.get("path") for r in rows], type=pa.string()),
        }
    )
    pq.write_table(table, path)


class SeamTestCase(unittest.TestCase):
    """Build an export with every table the adapter reads, then convert it."""

    def build(self, tmp, *, field_rows=None, movement_rows=(), guid_rows=(),
              manifest=None):
        root = Path(tmp)
        export = root / "export"
        export.mkdir()
        write_fields_parquet(
            export / "fields.parquet",
            list(MINIMAL_FIELD_ROWS if field_rows is None else field_rows),
        )
        if movement_rows:
            write_movement_parquet(export / "movement.parquet", list(movement_rows))
        if guid_rows:
            write_net_guids_parquet(export / "net_guids.parquet", list(guid_rows))
        if manifest is not None:
            (export / "manifest.json").write_text(
                json.dumps(manifest), encoding="utf-8"
            )
        out = root / "bundle"
        summary = bundle.convert(export, out)
        published = json.loads((out / "manifest.json").read_text(encoding="utf-8"))
        return out, published, summary

    def full_manifest(self, **overrides):
        manifest = {
            "source_file": "02d4d478.vrf",
            "source_size_bytes": 55297993,
            "replay_version": "5.3.2",
            "replay_build": "++Ares-Core+release-13.02",
            "replay_changelist": 2152699011,
            "duration_ms": 2296000,
            "quality": json.loads(json.dumps(UPSTREAM_QUALITY)),
            "players": json.loads(json.dumps(UPSTREAM_PLAYERS)),
            "net_field_export_groups": json.loads(json.dumps(UPSTREAM_GROUPS)),
        }
        manifest.update(overrides)
        return manifest


class UpstreamAccountingForwardingTests(SeamTestCase):
    """vrfkit counts the losses; the bundle has to carry the count.

    Before this, `_write_manifest` emitted six header scalars and dropped the
    entire `quality` object, so valplay's only way to judge completeness was
    to recount the NDJSON it had just been handed -- which cannot detect
    anything that never reached the NDJSON in the first place.
    """

    def test_quality_is_forwarded_verbatim(self):
        with tempfile.TemporaryDirectory() as tmp:
            _, published, _ = self.build(
                tmp,
                movement_rows=[{"time_ms": 1, "packet_id": 1, "char": 5}] * 3,
                guid_rows=[{"net_guid": 1, "outer": 2, "path": "a"},
                           {"net_guid": 2, "outer": 3, "path": "b"}],
                manifest=self.full_manifest(),
            )
        self.assertEqual(published["quality"], UPSTREAM_QUALITY)

    def test_net_field_export_groups_are_forwarded_verbatim(self):
        with tempfile.TemporaryDirectory() as tmp:
            _, published, _ = self.build(tmp, manifest=self.full_manifest())
        self.assertEqual(published["net_field_export_groups"], UPSTREAM_GROUPS)

    def test_an_export_without_quality_publishes_null_not_zeroes(self):
        """A missing value renders as a visible absence, never as a number.

        `{}` or a zero-filled object here would tell a consumer the export was
        complete on the strength of nobody having counted.
        """
        with tempfile.TemporaryDirectory() as tmp:
            _, published, summary = self.build(
                tmp, manifest={"replay_version": "5.3.2"}
            )
        self.assertIsNone(published["quality"])
        self.assertIsNone(published["net_field_export_groups"])
        self.assertIn("quality", published)
        # Nothing was dropped, so this must NOT read as a lossy conversion.
        self.assertEqual(summary["tally"].total, 0)

    def test_account_subjects_are_not_forwarded(self):
        """`players` is deliberately left behind; prove it stays behind.

        vrfkit's manifest bridges actor guid -> account subject -> character
        guid. valplay derives the same table from the same BombPlayerState
        rows, and its version is strictly richer (a SET of characters, which is
        what keeps a resurrected player's kills attributed). Forwarding a
        poorer copy would add account UUIDs to a second file while offering a
        tempting alternative that silently loses those kills.
        """
        with tempfile.TemporaryDirectory() as tmp:
            out, published, _ = self.build(tmp, manifest=self.full_manifest())
            raw = (out / "manifest.json").read_text(encoding="utf-8")
        self.assertNotIn("players", published)
        self.assertNotIn("11111111-2222-3333-4444-555555555555", raw)

    def test_the_manifest_key_set_is_pinned(self):
        """The bundle's shape is a contract, so it is spelled out once."""
        with tempfile.TemporaryDirectory() as tmp:
            _, published, _ = self.build(tmp, manifest=self.full_manifest())
        self.assertEqual(
            sorted(published),
            sorted([
                "adapter",
                "bundle_schema_version",
                "converter",
                "duration_ms",
                "net_field_export_groups",
                "quality",
                "replay_build",
                "replay_changelist",
                "replay_version",
                "source_file",
                "source_size_bytes",
            ]),
        )


class AdapterAccountingTests(SeamTestCase):
    """What this adapter measured, kept apart from what vrfkit declared."""

    def test_events_written_equals_the_lines_on_disk(self):
        """The one identity a consumer can re-verify exactly.

        valplay recounts events.ndjson; this is the number it recounts
        against. If they ever differ, the bundle was truncated after it was
        written, and no metric computed from it is worth publishing.
        """
        with tempfile.TemporaryDirectory() as tmp:
            out, published, _ = self.build(tmp, manifest=self.full_manifest())
            lines = (out / "events.ndjson").read_text(encoding="utf-8").splitlines()
        self.assertEqual(published["adapter"]["events_written"], len(lines))
        self.assertGreater(len(lines), 0)

    def test_movement_rows_read_is_the_table_height_not_the_written_count(self):
        """They differ by the intra-packet collapse, and both are published.

        Comparing the WRITTEN count with `quality.movement_rows` would report
        every healthy replay as lossy, because the collapse is intentional.
        """
        movement = [
            {"time_ms": 1, "packet_id": 1, "char": 5, "pos_x": 1.0},
            {"time_ms": 1, "packet_id": 1, "char": 5, "pos_x": 2.0},
            {"time_ms": 2, "packet_id": 2, "char": 5, "pos_x": 3.0},
        ]
        with tempfile.TemporaryDirectory() as tmp:
            _, published, _ = self.build(
                tmp, movement_rows=movement, manifest=self.full_manifest()
            )
        adapter = published["adapter"]
        self.assertEqual(adapter["movement_rows_read"], 3)
        self.assertEqual(adapter["movement_rows_written"], 2)
        self.assertTrue(adapter["upstream_row_counts"]["movement_rows"]["agrees"])

    def test_a_declared_count_its_own_table_contradicts_is_reported(self):
        """The producer disagreeing with itself is a signal, not a crash.

        The adapter counts it and publishes both numbers. It does not repair
        either -- which of the two is wrong is not knowable here -- and it does
        not refuse: refusing is the consumer's call, at the point of
        publication.
        """
        quality = json.loads(json.dumps(UPSTREAM_QUALITY))
        quality["movement_rows"] = 999
        with tempfile.TemporaryDirectory() as tmp:
            _, published, summary = self.build(
                tmp,
                movement_rows=[{"time_ms": 1, "packet_id": 1, "char": 5}],
                guid_rows=[{"net_guid": 1, "outer": 2, "path": "a"},
                           {"net_guid": 2, "outer": 3, "path": "b"}],
                manifest=self.full_manifest(quality=quality),
            )
        counts = published["adapter"]["upstream_row_counts"]
        self.assertEqual(
            counts["movement_rows"],
            {"declared": 999, "observed": 1, "agrees": False},
        )
        # The table that DOES match must still read as agreement, so the
        # signal names the one table that is wrong.
        self.assertEqual(
            counts["net_guid_rows"],
            {"declared": 2, "observed": 2, "agrees": True},
        )
        self.assertEqual(summary["tally"]["upstream_row_count_disagreement"], 1)
        self.assertGreater(summary["tally"].total, 0)

    def test_a_declared_table_that_is_absent_is_a_disagreement(self):
        """"vrfkit wrote 2 rows" and "the file is not there" cannot both hold.

        An absent table used to convert silently -- weapon identity simply
        went unresolved -- which is right when nothing claimed the table
        existed. Once the export declares a row count, its absence is a
        contradiction and has to be said out loud.
        """
        with tempfile.TemporaryDirectory() as tmp:
            _, published, summary = self.build(tmp, manifest=self.full_manifest())
        check = published["adapter"]["upstream_row_counts"]["net_guid_rows"]
        self.assertEqual(check, {"declared": 2, "observed": None, "agrees": False})
        self.assertGreaterEqual(
            summary["tally"]["upstream_row_count_disagreement"], 1
        )

    def test_nothing_declared_reads_as_unknown_not_as_agreement(self):
        with tempfile.TemporaryDirectory() as tmp:
            _, published, summary = self.build(
                tmp, manifest={"replay_version": "5.3.2"}
            )
        for name in ("movement_rows", "net_guid_rows"):
            check = published["adapter"]["upstream_row_counts"][name]
            self.assertIsNone(check["declared"], name)
            self.assertIsNone(
                check["agrees"],
                "an export that declared nothing must not certify itself",
            )
        self.assertEqual(summary["tally"]["upstream_row_count_disagreement"], 0)

    def test_the_loss_tally_reaches_the_manifest(self):
        rows = list(MINIMAL_FIELD_ROWS) + [
            {
                "time_ms": 10, "packet_id": 1, "actor": 101,
                "group_path": "PlayerState", "field_name": None,
                "bit_count": 3, "raw_bits": b"\x05",
            },
        ]
        with tempfile.TemporaryDirectory() as tmp:
            _, published, _ = self.build(
                tmp, field_rows=rows, manifest=self.full_manifest()
            )
        losses = published["adapter"]["losses"]
        self.assertEqual(losses["unnamed_property_rows"], 1)
        # Present and zero, not absent: a key that appears only when non-zero
        # cannot distinguish "clean" from "this counter stopped running".
        self.assertEqual(losses["unnamed_rpc_rows"], 0)


class EventOrderingContractTests(SeamTestCase):
    """The order events.ndjson is written in, pinned at the layer that sets it.

    valplay orders same-millisecond events by (time_ms, line index), which is
    only a total order if this file's output is in wire order and its time_ms
    column is non-decreasing. Neither was asserted anywhere, at any layer.
    """

    #: Two actors, three packets, deliberately supplied out of packet order in
    #: the parquet so a test that merely echoed input order would pass by luck.
    SHUFFLED = [
        {"time_ms": 30, "packet_id": 3, "actor": 303,
         "group_path": "PlayerState", "field_name": "Health",
         "bit_count": 32, "value_i64": 3},
        {"time_ms": 10, "packet_id": 1, "actor": 101,
         "group_path": "PlayerState", "field_name": "Health",
         "bit_count": 32, "value_i64": 1},
        {"time_ms": 20, "packet_id": 2, "actor": 202,
         "group_path": "PlayerState", "field_name": "Health",
         "bit_count": 32, "value_i64": 2},
    ]

    def read_events(self, out):
        text = (out / "events.ndjson").read_text(encoding="utf-8")
        return [json.loads(line) for line in text.splitlines()]

    def test_events_are_written_in_non_decreasing_time_order(self):
        with tempfile.TemporaryDirectory() as tmp:
            out, published, _ = self.build(
                tmp, field_rows=self.SHUFFLED, manifest=self.full_manifest()
            )
            events = self.read_events(out)
        times = [e["time_ms"] for e in events]
        self.assertEqual(times, sorted(times), events)
        self.assertEqual(published["adapter"]["events_time_ms_regressions"], 0)

    def test_a_spawn_precedes_the_property_event_at_the_same_millisecond(self):
        """The phase order (actors, properties, RPCs) is the tie-break.

        A property event for an actor that has not spawned yet is a document
        the consumer cannot read in one pass, so the stable sort's tie
        behaviour is a contract, not an implementation detail.
        """
        with tempfile.TemporaryDirectory() as tmp:
            out, _, _ = self.build(
                tmp, field_rows=self.SHUFFLED, manifest=self.full_manifest()
            )
            events = self.read_events(out)
        for actor in (101, 202, 303):
            same = [e for e in events if e.get("actor_net_guid") == actor]
            types = [e["type"] for e in same]
            self.assertEqual(
                types[0], "actor_spawned",
                f"actor {actor} is described before it exists: {types}",
            )

    def test_a_time_ms_regression_is_counted_and_published(self):
        """A frame whose time is not finite exports time_ms = 0.

        vrf-frame reads it with a bare read_f32 and substitutes 0 for anything
        non-finite, so one bad frame mid-replay makes time_ms non-monotonic
        while packet order stays correct. The adapter keeps packet order --
        that is the wire -- and reports the regression instead of hiding it by
        sorting on a value the replay does not guarantee.
        """
        rows = [
            {"time_ms": 10, "packet_id": 1, "actor": 101,
             "group_path": "PlayerState", "field_name": "Health",
             "bit_count": 32, "value_i64": 1},
            {"time_ms": 0, "packet_id": 2, "actor": 101,
             "group_path": "PlayerState", "field_name": "Health",
             "bit_count": 32, "value_i64": 2},
        ]
        with tempfile.TemporaryDirectory() as tmp:
            out, published, summary = self.build(
                tmp, field_rows=rows, manifest=self.full_manifest()
            )
            events = self.read_events(out)
        self.assertGreater(published["adapter"]["events_time_ms_regressions"], 0)
        self.assertGreater(summary["tally"]["events_time_ms_regressions"], 0)
        # Packet order is preserved: the regression is reported, not repaired.
        self.assertEqual(
            [e["time_ms"] for e in events if e["type"] == "export_group_received"],
            [10, 0],
        )


if __name__ == "__main__":
    unittest.main()
