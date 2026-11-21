import base64
import contextlib
import io
import json
import struct
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq


from support import TOOLS, TempDirTestCase
from wire_fixtures import pack_bits
from validate_type_evidence import (
    check_specifications, decode_exact, exported_matches, exported_value,
    load_specifications, main, spec_rows, validate, values_match)


def fstring_fields(text: str):
    data = text.encode("utf-8") + b"\0"
    return [(len(data), 32)] + [(byte, 8) for byte in data]


VALUE_TYPES = {"value_str": pa.string(), "value_i64": pa.int64(),
               "value_f64": pa.float64(), "value_bool": pa.bool_()}


def row(group, field, raw, bits, **columns):
    return {"group_path": group, "field_name": field, "raw_bits": raw, "bit_count": bits, **columns}


def write_rows(directory: Path, rows: list[dict], *, manifest: bool = True) -> Path:
    """A finished export: group/field dictionary-encoded as vrfkit writes them,
    handle 1 and checksum 7 unless a row says otherwise, value columns null."""
    directory.mkdir(parents=True, exist_ok=True)
    if manifest:
        (directory / "manifest.json").write_text('{"replay_build": "13.02"}', encoding="utf-8")
    pq.write_table(pa.table({
        "group_path": pa.array([r["group_path"] for r in rows]).dictionary_encode(),
        "field_name": pa.array([r["field_name"] for r in rows]).dictionary_encode(),
        "handle": pa.array([r.get("handle", 1) for r in rows], pa.uint32()),
        "compatible_checksum": pa.array([r.get("compatible_checksum", 7) for r in rows], pa.uint32()),
        "bit_count": pa.array([r["bit_count"] for r in rows], pa.uint32()),
        "raw_bits": pa.array([r["raw_bits"] for r in rows], pa.binary()),
        **{name: pa.array([r.get(name) for r in rows], kind) for name, kind in VALUE_TYPES.items()},
    }), directory / "fields.parquet")
    return directory


#: A payload `Int32` cannot decode: three bytes declared as 32 bits.
MALFORMED = (b"\x01\x02\x03", 32)
VALID = (struct.pack("<i", 5), 32)


class SpecificationScopeTests(TempDirTestCase):
    """Only (group, field[, checksum]) pairs the specification names are
    evidence. `validate` narrows rows by group AND field name before handing
    them to Python, then resolves the exact pair; these pin both halves."""

    def test_an_unspecified_field_of_a_specified_group_is_ignored(self):
        directory = self.tmp()
        write_rows(directory, [row("g", "f", *VALID), row("g", "other", *MALFORMED)])
        report = validate(directory,
                          [{"group": "g", "field": "f", "type": "Int32"}])
        self.assertEqual(report["failure_count"], 0)
        self.assertEqual(list(report["fields"]), ["g::f"])
        self.assertEqual(report["fields"]["g::f"]["rows"], 1)

    def test_a_group_paired_with_another_entrys_field_is_ignored(self):
        """Rows can pass a group-set x field-set prefilter without being a
        specified pair; the exact lookup after it must still drop them."""
        directory = self.tmp()
        write_rows(directory, [
            row("a", "x", *VALID), row("b", "y", *VALID),
            row("a", "y", *MALFORMED), row("b", "x", *MALFORMED)])
        report = validate(directory, [
            {"group": "a", "field": "x", "type": "Int32"},
            {"group": "b", "field": "y", "type": "Int32"}])
        self.assertEqual(report["failure_count"], 0)
        self.assertEqual(sorted(report["fields"]), ["a::x", "b::y"])
        self.assertEqual(report["missing"], [])

    def test_the_prefilter_keeps_only_specified_groups_and_fields_in_order(self):
        table = pa.table({
            "group_path": pa.array(["g", "g", "h", "g", "h"]).dictionary_encode(),
            "field_name": pa.array(["f", "other", "f", "f", "other"]).dictionary_encode(),
            "handle": [1, 2, 3, 4, 5],
        })
        kept = spec_rows(table, pa.array(["g"]), pa.array(["f"]))
        self.assertEqual(kept["handle"].to_pylist(), [1, 4])


#: Two real `ReplicatedMovement` rows and the JSON the Rust exporter wrote for
#: them. The Short one is the case a string compare cannot survive: f32 yaw
#: 283.88671875 is printed as `283.88672`.
DIVEBOMB_BYTE = (
    "e0ec4e985d5421c37b4d07e4de1b04", 118,
    '{"linear_velocity":{"x":2062,"y":-530,"z":525},"angular_velocity":null,'
    '"location":{"x":25.25,"y":-44.04,"z":6.8},"rotation":{"pitch":16.875,'
    '"yaw":347.34375,"roll":0},"simulated_physics_sleep":false,"rep_physics":false,'
    '"server_frame":null,"server_physics_handle":null}',
)
SEEKER_NADE_SHORT = (
    "4095feb5fa74b0e00ac1930501", 100,
    '{"linear_velocity":{"x":0,"y":0,"z":0},"angular_velocity":null,'
    '"location":{"x":4423.22,"y":598.93,"z":891.1},"rotation":{"pitch":0,'
    '"yaw":283.88672,"roll":0},"simulated_physics_sleep":false,"rep_physics":false,'
    '"server_frame":null,"server_physics_handle":null}',
)


def rep_movement_matches(text, decoded, type_name="RepMovementByte"):
    return values_match(type_name, decoded, exported_value({"value_str": text}, type_name))


class BitLevelTypeTests(TempDirTestCase):
    def test_enum_byte_takes_its_width_from_the_payload(self):
        self.assertEqual(decode_exact(b"\x05", 3, "EnumByte"), 5)
        self.assertEqual(decode_exact(b"\xff", 8, "EnumByte"), 255)
        with self.assertRaisesRegex(ValueError, "1..8"):
            decode_exact(b"\x00\x01", 9, "EnumByte")
        with self.assertRaisesRegex(ValueError, "1..8"):
            decode_exact(b"", 0, "EnumByte")

    def test_bit_level_padding_must_be_zero(self):
        with self.assertRaisesRegex(ValueError, "padding"):
            decode_exact(b"\x0d", 3, "EnumByte")
        with self.assertRaisesRegex(ValueError, "padding"):
            decode_exact(b"\x03", 1, "Bool")

    def test_fname_reads_the_two_corpus_payloads(self):
        """Real OriginalBuyerTeam payloads."""
        self.assertEqual(
            decode_exact(bytes.fromhex("08000000a4cac8000000000000"), 97, "FName"), "Red")
        self.assertEqual(
            decode_exact(bytes.fromhex("0a00000084d8eaca000000000000"), 105, "FName"), "Blue")

    def test_fname_instance_number_and_hardcoded_index(self):
        raw, bits = pack_bits((0, 1), *fstring_fields("Red"), (2, 32))
        self.assertEqual(decode_exact(raw, bits, "FName"), "Red_1")
        raw, bits = pack_bits((1, 1), (0x59, 8), (0x04, 8))
        self.assertEqual(decode_exact(raw, bits, "FName"), "300")

    def test_fname_rejects_residue_bad_numbers_and_missing_terminators(self):
        raw, bits = pack_bits((0, 1), *fstring_fields("Red"), (0, 32), (0, 1))
        with self.assertRaisesRegex(ValueError, "residual"):
            decode_exact(raw, bits, "FName")
        raw, bits = pack_bits((0, 1), *fstring_fields("Red"), (-1, 32))
        with self.assertRaisesRegex(ValueError, "negative"):
            decode_exact(raw, bits, "FName")
        raw, bits = pack_bits((0, 1), (3, 32), *[(b, 8) for b in b"Red"], (0, 32))
        with self.assertRaisesRegex(ValueError, "terminator"):
            decode_exact(raw, bits, "FName")

    def test_rep_movement_matches_the_exported_json(self):
        for (hex_bits, bits, text), type_name in (
            (DIVEBOMB_BYTE, "RepMovementByte"),
            (SEEKER_NADE_SHORT, "RepMovementShort"),
        ):
            decoded = decode_exact(bytes.fromhex(hex_bits), bits, type_name)
            self.assertEqual(rep_movement_matches(text, decoded, type_name), (True, "/100"), type_name)

    def test_the_location_scale_is_reported_not_assumed(self):
        """The same bits exported in world units still match, at "/1"; a
        scale nobody uses, or two scales in one vector, does not."""
        hex_bits, bits, text = DIVEBOMB_BYTE
        decoded = decode_exact(bytes.fromhex(hex_bits), bits, "RepMovementByte")
        self.assertEqual(decoded["location"], {"packed": (2525, -4404, 680), "scaled": True})
        for location, expected in (
            ('{"x":2525,"y":-4404,"z":680}', (True, "/1")),
            ('{"x":252.5,"y":-440.4,"z":68}', (False, None)),
            ('{"x":2525,"y":-44.04,"z":6.8}', (False, None)),
        ):
            world = text.replace('{"x":25.25,"y":-44.04,"z":6.8}', location)
            self.assertEqual(rep_movement_matches(world, decoded), expected, location)

    def test_the_wrong_rotator_width_is_caught(self):
        """A Byte row read as Short runs off the end. The reverse can consume
        exactly and still be wrong -- which only the value compare sees."""
        hex_bits, bits, _text = DIVEBOMB_BYTE
        with self.assertRaisesRegex(ValueError, "past the end"):
            decode_exact(bytes.fromhex(hex_bits), bits, "RepMovementShort")
        hex_bits, bits, text = SEEKER_NADE_SHORT
        as_byte = decode_exact(bytes.fromhex(hex_bits), bits, "RepMovementByte")
        self.assertEqual(rep_movement_matches(text, as_byte), (False, None))

    def test_rep_movement_optional_members_are_read_in_wire_order(self):
        # flags: physics, server frame and server handle set; every vector and
        # rotator component present and non-zero.
        raw, bits = pack_bits(
            (0, 1), (1, 1), (1, 1), (1, 1),
            (70, 7), (5, 6), (-3, 6), (1, 6),         # location, "scaled" set
            (1, 1), (64, 8), (0, 1), (1, 1), (128, 8),  # pitch, no yaw, roll
            (6, 7), (-7, 6), (0, 6), (31, 6),          # velocity, unscaled
            (6, 7), (1, 6), (2, 6), (3, 6),            # angular velocity
            (0x06, 8), (0x0a, 8),                      # frame 3, handle 5
        )
        self.assertEqual(decode_exact(raw, bits, "RepMovementByte"), {
            "linear_velocity": (-7.0, 0.0, 31.0),
            "angular_velocity": (1.0, 2.0, 3.0),
            "location": {"packed": (5, -3, 1), "scaled": True},
            "rotation": (90.0, 0.0, 180.0),
            "simulated_physics_sleep": False,
            "rep_physics": True,
            "server_frame": 3,
            "server_physics_handle": 5,
        })

    def test_an_unparseable_export_is_a_mismatch_not_a_crash(self):
        hex_bits, bits, _text = DIVEBOMB_BYTE
        decoded = decode_exact(bytes.fromhex(hex_bits), bits, "RepMovementByte")
        self.assertEqual(rep_movement_matches("{not json", decoded), (False, None))

    def test_compare_typed_checks_every_bit_level_type(self):
        rows = [
            row("g", "e", bytes.fromhex("05"), 3, value_i64=5),
            row("g", "n", bytes.fromhex("08000000a4cac8000000000000"), 97, value_str="Red"),
            row("g", "m", bytes.fromhex(DIVEBOMB_BYTE[0]), DIVEBOMB_BYTE[1], value_str=DIVEBOMB_BYTE[2]),
        ]
        specs = [{"group": "g", "field": "e", "type": "EnumByte"},
                 {"group": "g", "field": "n", "type": "FName"},
                 {"group": "g", "field": "m", "type": "RepMovementByte"}]

        def run(rows):
            directory = self.tmp()
            return validate(write_rows(directory, rows), specs, compare_typed=True)

        clean = run(rows)
        self.assertEqual((clean["failure_count"], clean["typed_mismatch_count"]), (0, 0))
        self.assertEqual(clean["missing"], [])
        self.assertEqual(clean["fields"]["g::m"]["location_scales"], {"/100": 1})
        wrong = [{**rows[0], "value_i64": 4}, {**rows[1], "value_str": "Blue"},
                 {**rows[2], "value_str": DIVEBOMB_BYTE[2].replace("347.34375", "348.75")}]
        self.assertEqual(run(wrong)["typed_mismatch_count"], 3)


def write_int_export(directory: Path, *, manifest: bool = True) -> None:
    write_rows(directory, [row("g", "f", struct.pack("<i", 17), 32, value_i64=17)], manifest=manifest)


class ExportDiscoveryTests(TempDirTestCase):
    SPEC = [{"group": "g", "field": "f", "type": "Int32"}]

    def test_a_stranded_prior_output_is_not_counted_a_second_time(self):
        """`vrfkit export` leaves the prior `--out` as a complete
        `.X.vrfkit-previous-*` export when it cannot delete it or dies between
        its two renames; read as a second export it doubles the rows."""
        root = self.tmp()
        write_int_export(root / "a")
        write_int_export(root / ".a.vrfkit-previous-4242-7")
        expected = [str((root / ".a.vrfkit-previous-4242-7").resolve())]
        report = validate(root, self.SPEC, compare_typed=True)
        self.assertEqual(report["fields"]["g::f"]["rows"], 1)
        self.assertEqual(report["typed_mismatch_count"], 0)
        self.assertEqual(report["skipped_generated_dirs"], expected)

    def test_a_killed_exports_staging_directory_is_not_read(self):
        root = self.tmp()
        write_int_export(root / "pub2")
        staging = root / ".pub2.vrfkit-staging-55396-0"
        staging.mkdir()
        (staging / "fields.parquet").write_bytes(b"PAR1 no footer")
        report = validate(root, self.SPEC)
        self.assertEqual(report["fields"]["g::f"]["rows"], 1)
        self.assertEqual(len(report["skipped_generated_dirs"]), 1)

    def test_a_leftover_nested_below_the_root_is_skipped_too(self):
        root = self.tmp()
        write_int_export(root / "build" / "a")
        write_int_export(root / "build" / ".a.vrfkit-previous-4242-7" / "nested")
        report = validate(root, self.SPEC)
        self.assertEqual(report["fields"]["g::f"]["rows"], 1)

    def test_a_discovered_table_without_a_manifest_is_refused(self):
        """manifest.json is written last; without it the tables may be partial."""
        root = self.tmp()
        write_int_export(root / "complete")
        write_int_export(root / "partial", manifest=False)
        with self.assertRaisesRegex(ValueError, "manifest.json"):
            validate(root, self.SPEC)

    def test_an_export_id_naming_no_tables_is_refused(self):
        root = self.tmp()
        write_int_export(root / "done")
        with self.assertRaisesRegex(ValueError, "typo"):
            validate(root, self.SPEC, export_ids=["done", "typo"])

    def test_only_leftovers_below_the_root_is_an_error_that_counts_them(self):
        root = self.tmp()
        write_int_export(root / ".a.vrfkit-previous-4242-7")
        with self.assertRaisesRegex(ValueError, r"no field parquet files.*1 "):
            validate(root, self.SPEC)

    def test_a_path_named_explicitly_is_never_filtered(self):
        """Only discovery filters; pointing the tool at a leftover is deliberate."""
        root = self.tmp()
        leftover = root / ".a.vrfkit-previous-4242-7"
        write_int_export(leftover)
        for report in (
            validate(leftover, self.SPEC),
            validate(leftover / "fields.parquet", self.SPEC),
            validate(root, self.SPEC, export_ids=[leftover.name]),
        ):
            self.assertEqual(report["fields"]["g::f"]["rows"], 1)
            self.assertEqual(report["skipped_generated_dirs"], [])


class SpecificationFileTests(TempDirTestCase):
    def test_every_committed_specification_is_well_formed(self):
        for name in ("type_evidence.json", "type_evidence_aliases.json", "type_evidence_rejected.json",
                     "public_fixture_type_evidence.json", "scoped_type_evidence.json"):
            with self.subTest(name):
                self.assertTrue(check_specifications(load_specifications(TOOLS / "fixtures" / name)))

    def test_the_scoped_fixture_is_read_in_the_exported_spelling(self):
        specs = load_specifications(TOOLS / "fixtures" / "scoped_type_evidence.json")
        self.assertIn({"group": "/Script/ShooterGame.DamageableComponent_ClassNetCache",
                       "field": "MulticastNotifyHeal.EventInstigator",
                       "checksum": 3087885251, "type": "ObjectNetGuid"}, specs)
        self.assertEqual([s for s in specs if ":" in s["group"]], [])

    def test_allow_missing_lists_an_absent_identity_but_fails_when_none_is_observed(self):
        root = self.tmp()
        write_int_export(root / "a")
        spec, absent = root / "spec.json", root / "absent.json"
        spec.write_text(json.dumps([{"group": "g", "field": "f", "type": "Int32"},
                                    {"group": "g", "field": "absent", "type": "Int32"}]), encoding="utf-8")
        absent.write_text(json.dumps([{"group": "other", "field": "x", "type": "Int32"}]), encoding="utf-8")
        output, errors = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
            codes = (main([str(root), str(spec)]), main([str(root), str(spec), "--allow-missing"]),
                     main([str(root), str(absent), "--allow-missing"]))
        self.assertEqual(codes, (1, 0, 1))
        self.assertEqual(output.getvalue().count('"g::absent"'), 2)
        self.assertEqual(errors.getvalue(), "no specified identity was observed\n")

    def test_a_failed_run_never_leaves_an_earlier_report_at_output(self):
        root = self.tmp()
        write_int_export(root / "a")
        spec, report = root / "spec.json", root / "report.json"
        spec.write_text(json.dumps([{"group": "g", "field": "f", "type": "Int32"}]), encoding="utf-8")
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(main([str(root), str(spec), "--output", str(report)]), 0)
        self.assertEqual(json.loads(report.read_text(encoding="utf-8"))["failure_count"], 0)
        with contextlib.redirect_stdout(io.StringIO()), self.assertRaisesRegex(ValueError, "does not exist"):
            main([str(root / "absent"), str(spec), "--output", str(report)])
        self.assertFalse(report.exists())

    def test_output_cannot_replace_the_specification_or_an_export_file(self):
        root = self.tmp()
        write_int_export(root / "a")
        spec = root / "spec.json"
        spec.write_text(json.dumps([{"group": "g", "field": "f", "type": "Int32"}]), encoding="utf-8")
        before = {path: path.read_bytes() for path in (spec, root / "a" / "fields.parquet")}
        for output in before:
            with self.subTest(output=output.name), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as exc:
                    main([str(root), str(spec), "--output", str(output)])
                self.assertEqual(exc.exception.code, 2)
        self.assertEqual({path: path.read_bytes() for path in before}, before)

    def test_empty_specification_and_missing_root_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "empty"):
            validate(Path("."), [])
        with self.assertRaisesRegex(ValueError, "does not exist"):
            validate(Path("definitely-not-a-real-evidence-root"), [
                {"group": "g", "field": "f", "type": "Bool"}
            ])


class DecodeExactTests(TempDirTestCase):
    def test_checksum_scope_separates_same_name_and_catches_wrong_values(self):
        directory = self.tmp()
        write_rows(directory, [
            row("g", "B", b"\xff", 8, handle=39, compatible_checksum=379198054, value_i64=254),
            row("g", "B", b"\0\0\0\0", 32, handle=208, compatible_checksum=943211507)])
        spec = {"group": "g", "field": "B", "type": "Byte", "checksum": 379198054}
        report = validate(directory, [spec], compare_typed=True)
        self.assertEqual(report["failure_count"], 0)
        self.assertEqual(report["typed_mismatch_count"], 1)
        self.assertEqual(report["fields"]["g::B::checksum=379198054"]["rows"], 1)
        with self.assertRaisesRegex(ValueError, "overlapping"):
            validate(directory, [spec, {"group": "g", "field": "B", "type": "Byte"}])
        unscoped = validate(directory, [{"group": "g", "field": "B", "type": "Byte"}])
        self.assertEqual(unscoped["failure_count"], 1)

    def test_primitive_widths_and_values(self):
        self.assertIs(decode_exact(b"\x01", 1, "Bool"), True)
        self.assertEqual(decode_exact(b"\xff", 8, "Byte"), 255)
        self.assertEqual(decode_exact(struct.pack("<i", -17), 32, "Int32"), -17)
        self.assertEqual(decode_exact(struct.pack("<f", 1.25), 32, "Float"), 1.25)
        self.assertEqual(decode_exact(struct.pack("<d", 2.5), 64, "Double"), 2.5)

    def test_uint32_keeps_the_high_bit_positive_and_catches_a_sign_flip(self):
        # 0xe28c69d7 is a real player-state GUID word: an Int32 reading of the
        # same four bytes is negative, and only an unsigned check can tell.
        raw = struct.pack("<I", 0xE28C69D7)
        self.assertEqual(decode_exact(raw, 32, "UInt32"), 0xE28C69D7)
        with self.assertRaisesRegex(ValueError, "32 bits"):
            decode_exact(raw[:3], 24, "UInt32")
        directory = self.tmp()
        write_rows(directory, [
            row("g", "D", raw, 32, handle=210, compatible_checksum=1032080829, value_i64=value)
            for value in (0xE28C69D7, 0xE28C69D7 - 2**32)])
        spec = {"group": "g", "field": "D", "type": "UInt32", "checksum": 1032080829}
        report = validate(directory, [spec], compare_typed=True)
        self.assertEqual(report["failure_count"], 0)
        self.assertEqual(report["typed_mismatch_count"], 1)
        self.assertEqual(report["typed_mismatch_examples"][0]["exported"], 0xE28C69D7 - 2**32)
        self.assertEqual(report["typed_mismatch_examples"][0]["decoded"], 0xE28C69D7)
        self.assertEqual(report["fields"]["g::D::checksum=1032080829"]["min"], 0xE28C69D7)

    def test_fstring_requires_full_consumption_and_terminator(self):
        raw = struct.pack("<i", 4) + b"abc\0"
        self.assertEqual(decode_exact(raw, 64, "FString"), "abc")
        with self.assertRaisesRegex(ValueError, "residual"):
            decode_exact(raw + b"x", 72, "FString")
        with self.assertRaisesRegex(ValueError, "terminator"):
            decode_exact(struct.pack("<i", 4) + b"abcd", 64, "FString")

    def test_object_guid_requires_terminated_full_intpacked(self):
        self.assertEqual(decode_exact(b"\x59\x04", 16, "ObjectNetGuid"), 300)
        with self.assertRaisesRegex(ValueError, "residual"):
            decode_exact(b"\x02\x00", 16, "ObjectNetGuid")

    def test_object_guid_rejects_leb128_mutant_truncation_and_overflow(self):
        with self.assertRaisesRegex(ValueError, "residual"):
            decode_exact(b"\xac\x02", 16, "ObjectNetGuid")
        with self.assertRaisesRegex(ValueError, "truncated"):
            decode_exact(b"\x01", 8, "ObjectNetGuid")
        with self.assertRaisesRegex(ValueError, "overflowing"):
            decode_exact(b"\x01\x01\x01\x01\x20", 40, "ObjectNetGuid")
        with self.assertRaisesRegex(ValueError, "runaway"):
            decode_exact(b"\xff" * 5, 40, "ObjectNetGuid")

    def test_object_guid_accepts_u32_max(self):
        self.assertEqual(
            decode_exact(b"\xff\xff\xff\xff\x1e", 40, "ObjectNetGuid"),
            0xffffffff,
        )

    #: A real `Projectile_Hunter_Q_RevealBolt_C.TrailPosition` payload (13.06),
    #: and the three doubles an independent `struct.unpack("<3d")` gives it.
    TRAIL = ("000000606cc8ba40000000e0079cbec0000000c081c27c40",
             (6856.42333984375, -7836.03076171875, 460.15667724609375))

    def test_vector_double_is_three_finite_little_endian_doubles(self):
        raw = bytes.fromhex(self.TRAIL[0])
        self.assertEqual(decode_exact(raw, 192, "VectorDouble"), self.TRAIL[1])
        with self.assertRaisesRegex(ValueError, "192 bits"):
            decode_exact(raw[:12], 96, "VectorDouble")
        with self.assertRaisesRegex(ValueError, "non-finite"):
            decode_exact(raw[:16] + struct.pack("<d", float("inf")), 192, "VectorDouble")

    def test_vector_double_compares_the_exported_spelling_numerically(self):
        decoded = self.TRAIL[1]
        exported = "(6856.42333984375,-7836.03076171875,460.15667724609375)"
        self.assertTrue(exported_matches("VectorDouble", exported, decoded))
        # A spelling that drops digits of the same double, and a different
        # number, both differ: the comparison is exact, not approximate.
        self.assertFalse(exported_matches(
            "VectorDouble", exported.replace("460.15667724609375", "460.1566772460938"), decoded))
        self.assertFalse(exported_matches(
            "VectorDouble", exported.replace("6856.", "6857."), decoded))
        self.assertFalse(exported_matches("VectorDouble", None, decoded))

    #: Real 13.06 `OverrideMatchTimerText` payloads: the 72-bit empty history
    #: 255, and a 376-bit history 4 (AsNumber) with a double source.
    TIMER_EMPTY = ("00000000ff00000000", 72)
    TIMER_NUMBER = ("010000000403000000805f3a2f40010000000000000001000000000200000002000000"
                    "020000000200000000000000", 376)
    TIMER_NUMBER_JSON = (
        '{"flags":1,"history":4,"kind":"as_number","source":{"tag":3,"double":15.614009857177734},'
        '"format":{"always_sign":false,"use_grouping":true,"rounding_mode":0,'
        '"minimum_integral_digits":2,"maximum_integral_digits":2,"minimum_fractional_digits":2,'
        '"maximum_fractional_digits":2},"culture":""}')

    def test_ftext_tree_reads_the_observed_histories(self):
        empty = decode_exact(bytes.fromhex(self.TIMER_EMPTY[0]), self.TIMER_EMPTY[1], "FTextTree")
        self.assertEqual(empty, {"flags": 0, "history": 255, "kind": "empty"})
        number = decode_exact(bytes.fromhex(self.TIMER_NUMBER[0]), self.TIMER_NUMBER[1], "FTextTree")
        self.assertEqual(number["source"], {"tag": 3, "double": 15.614009857177734})
        self.assertEqual(number["format"]["maximum_fractional_digits"], 2)
        # History 11, one bit off byte alignment after the history byte: a
        # string-table entry, here the table path and key "Kills".
        raw, bits = pack_bits((0, 32), (11, 8), (0, 1), *fstring_fields("/T/S.S"), (0, 32),
                              *fstring_fields("Kills"))
        self.assertEqual(decode_exact(raw, bits, "FTextTree"),
                         {"flags": 0, "history": 11, "kind": "string_table",
                          "table": {"name": "/T/S.S", "number": 0}, "key": "Kills"})

    def test_ftext_tree_refuses_unseen_forms_and_residue(self):
        raw = bytearray(bytes.fromhex(self.TIMER_NUMBER[0]))
        for index, value, message in ((4, 5, "history 5"), (5, 2, "tag 2"), (14, 2, "bool is 2")):
            mutant = bytearray(raw)
            mutant[index] = value
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                decode_exact(bytes(mutant), 376, "FTextTree")
        nan = bytearray(raw)
        nan[6:14] = struct.pack("<d", float("nan"))
        with self.assertRaisesRegex(ValueError, "non-finite"):
            decode_exact(bytes(nan), 376, "FTextTree")
        with self.assertRaisesRegex(ValueError, "residual"):
            decode_exact(bytes(raw) + b"\0", 384, "FTextTree")

    def test_ftext_tree_compares_the_exported_json_exactly(self):
        decoded = decode_exact(bytes.fromhex(self.TIMER_NUMBER[0]), 376, "FTextTree")
        self.assertTrue(exported_matches("FTextTree", self.TIMER_NUMBER_JSON, decoded))
        for wrong in (self.TIMER_NUMBER_JSON.replace("15.614009857177734", "15.61"),
                      self.TIMER_NUMBER_JSON.replace('"culture":""', '"culture":"","extra":1'),
                      self.TIMER_NUMBER_JSON[:-1], None):
            with self.subTest(wrong=wrong):
                self.assertFalse(exported_matches("FTextTree", wrong, decoded))

    def test_non_finite_float_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "non-finite"):
            decode_exact(struct.pack("<f", float("nan")), 32, "Float")

    def test_wrong_width_fails_instead_of_decoding_a_prefix(self):
        with self.assertRaisesRegex(ValueError, "32 bits"):
            decode_exact(struct.pack("<d", 1.0), 64, "Float")


class ShapedTypeTests(TempDirTestCase):
    """The non-primitive decoders scoped entries may use.

    The base64 vectors are payloads an independent parser's tests recorded
    from a replay outside the local corpus: an independent statement of what
    those exact bits decode to, and of their exact widths.
    """

    def test_recorded_raze_payloads_consume_their_exact_widths(self):
        self.assertEqual(decode_exact(base64.b64decode("4elLQA=="), 32, "Int32"), 1078716897)
        self.assertEqual(decode_exact(base64.b64decode("AQ=="), 3, "EnumRemainingBits"), 1)
        self.assertEqual(decode_exact(base64.b64decode("Aw=="), 3, "EnumRemainingBits"), 3)
        location = decode_exact(base64.b64decode("0yBnt6iXSAA="), 64, "VectorNetQuantize100")
        self.assertEqual(location, (-782.71, -1366.59, 5.8))
        rotation = decode_exact(base64.b64decode("AYDuJ/f/Bw=="), 51, "RotationShort")
        self.assertEqual(rotation, (90.0, 284.0350341796875, 359.989013671875))
        # The recorded truncation case: one byte cannot hold a rotator's flags
        # and the components they announce.
        with self.assertRaisesRegex(ValueError, "truncated"):
            decode_exact(base64.b64decode("AQ=="), 8, "RotationShort")

    def test_quantized_vector_scaled_components_and_double_fallback(self):
        raw, bits = pack_bits((8 | 64, 7), (100, 8), (100, 8), (-100, 8))
        self.assertEqual(bits, 31)
        self.assertEqual(decode_exact(raw, bits, "VectorNetQuantize100"), (1.0, 1.0, -1.0))
        with self.assertRaisesRegex(ValueError, "residual"):
            decode_exact(*pack_bits((8 | 64, 7), (100, 8), (100, 8), (100, 8), (0, 1)),
                         "VectorNetQuantize100")
        with self.assertRaisesRegex(ValueError, "truncated"):
            decode_exact(*pack_bits((8 | 64, 7), (100, 8), (100, 8)), "VectorNetQuantize100")
        doubles = [struct.unpack("<Q", struct.pack("<d", v))[0] for v in (1.5, -2.25, 3.0)]
        raw, bits = pack_bits((64, 7), *((d, 64) for d in doubles))
        self.assertEqual(decode_exact(raw, bits, "VectorNetQuantize100"), (1.5, -2.25, 3.0))
        nan = struct.unpack("<I", struct.pack("<f", float("nan")))[0]
        with self.assertRaisesRegex(ValueError, "non-finite"):
            decode_exact(*pack_bits((0, 7), (nan, 32), (0, 32), (0, 32)), "VectorNetQuantize100")

    def test_rotation_short_reads_only_the_announced_components(self):
        self.assertEqual(decode_exact(*pack_bits((0, 3)), "RotationShort"), (0.0, 0.0, 0.0))
        raw, bits = pack_bits((0, 1), (1, 1), (16384, 16), (0, 1))
        self.assertEqual(bits, 19)
        self.assertEqual(decode_exact(raw, bits, "RotationShort"), (0.0, 90.0, 0.0))

    def test_rep_movement_quantization_is_decided_by_exact_consumption(self):
        # Flags, a 31-bit location, byte rotation with only yaw, and a 7-bit
        # zero-width velocity header that falls back to three floats.
        fields = [(0, 4), (8 | 64, 7), (1, 8), (2, 8), (3, 8),
                  (0, 1), (1, 1), (64, 8), (0, 1),
                  (0, 7), (0, 32), (0, 32), (0, 32)]
        raw, bits = pack_bits(*fields)
        value = decode_exact(raw, bits, "RepMovementByte")
        # The location stays packed: its scale is not on the wire (see
        # BitLevelTypeTests.test_the_location_scale_is_reported_not_assumed).
        self.assertEqual(value["location"], {"packed": (1, 2, 3), "scaled": True})
        self.assertEqual(value["rotation"], (0.0, 90.0, 0.0))
        self.assertIsNone(value["angular_velocity"])
        self.assertIsNone(value["server_frame"])
        with self.assertRaises(ValueError):
            decode_exact(raw, bits, "RepMovementShort")
        raw, bits = pack_bits(*fields[:6], (1, 1), (16384, 16), *fields[8:])
        self.assertEqual(decode_exact(raw, bits, "RepMovementShort")["rotation"], (0.0, 90.0, 0.0))
        with self.assertRaises(ValueError):
            decode_exact(raw, bits, "RepMovementByte")

    def test_rep_movement_flags_gate_angular_velocity_and_packed_frames(self):
        raw, bits = pack_bits((0b0110, 4), (8 | 64, 7), (0, 8), (0, 8), (0, 8), (0, 3),
                              (8 | 64, 7), (1, 8), (1, 8), (1, 8),
                              (8 | 64, 7), (2, 8), (2, 8), (2, 8),
                              (0x59, 8), (0x04, 8))
        value = decode_exact(raw, bits, "RepMovementByte")
        self.assertEqual(value["angular_velocity"], (2.0, 2.0, 2.0))
        self.assertEqual(value["server_frame"], 300)
        self.assertIsNone(value["server_physics_handle"])
        self.assertTrue(value["rep_physics"])

    def test_enum_remaining_bits_takes_every_bit_and_refuses_wider_than_32(self):
        self.assertEqual(decode_exact(None, 0, "EnumRemainingBits"), 0)
        self.assertEqual(decode_exact(*pack_bits((6, 3)), "EnumRemainingBits"), 6)
        with self.assertRaisesRegex(ValueError, "wider than 32"):
            decode_exact(b"\0" * 5, 33, "EnumRemainingBits")

    def test_compare_typed_parses_geometry_strings(self):
        self.assertTrue(exported_matches("VectorNetQuantize100", "(1,1,-1)", (1.0, 1.0, -1.0)))
        self.assertFalse(exported_matches("VectorNetQuantize100", "(1,1,1)", (1.0, 1.0, -1.0)))
        self.assertFalse(exported_matches("VectorNetQuantize100", None, (1.0, 1.0, -1.0)))
        rotation = (90.0, 284.0350341796875, 359.989013671875)
        # Rust prints f32 in its shortest round-trip spelling: "359.989" is not
        # the double 359.989013671875, but it is that single-precision value.
        self.assertTrue(exported_matches("RotationShort", "rot(90,284.03503,359.989)", rotation))
        self.assertFalse(exported_matches("RotationShort", "rot(90,284.03503,359.98)", rotation))
        decoded = {"location": {"packed": (1, 2, 3), "scaled": True},
                   "rotation": (0.0, 90.0, 0.0),
                   "linear_velocity": (0.0, 0.0, 0.0), "angular_velocity": None,
                   "simulated_physics_sleep": False, "rep_physics": False,
                   "server_frame": None, "server_physics_handle": None}
        exported = ('{"linear_velocity":{"x":0,"y":0,"z":0},"angular_velocity":null,'
                    '"location":{"x":0.01,"y":0.02,"z":0.03},'
                    '"rotation":{"pitch":0,"yaw":90,"roll":0},'
                    '"simulated_physics_sleep":false,"rep_physics":false,'
                    '"server_frame":null,"server_physics_handle":null}')
        self.assertEqual(rep_movement_matches(exported, decoded), (True, "/100"))
        for wrong in (exported.replace('"yaw":90', '"yaw":91'),
                      exported.replace('"rep_physics":false', '"rep_physics":true'), "not json"):
            self.assertEqual(rep_movement_matches(wrong, decoded), (False, None))

    def test_validate_reports_vector_component_ranges(self):
        directory = self.tmp()
        first, bits = pack_bits((8 | 64, 7), (100, 8), (100, 8), (100, 8))
        second, _ = pack_bits((8 | 64, 7), (-50, 8), (100, 8), (120, 8))
        write_rows(directory, [
            row("g", "RelativeScale3D", raw, bits, handle=6, compatible_checksum=1992268157, value_str=text)
            for raw, text in ((first, "(1,1,1)"), (second, "(-0.5,1,1.2)"))])
        report = validate(directory, [{"group": "g", "field": "RelativeScale3D",
                                             "type": "VectorNetQuantize100",
                                             "checksum": 1992268157}], compare_typed=True)
        self.assertEqual(report["failure_count"], 0)
        self.assertEqual(report["typed_mismatch_count"], 0)
        field = report["fields"]["g::RelativeScale3D::checksum=1992268157"]
        self.assertEqual(field["min"], (-0.5, 1.0, 1.0))
        self.assertEqual(field["max"], (1.0, 1.0, 1.2))
