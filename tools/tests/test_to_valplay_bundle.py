import base64
import contextlib
import io
import json
import math
import os
import re
import sys
import tempfile
import unittest
from pathlib import Path
from typing import NamedTuple
from unittest import mock

import numpy
import pyarrow as pa
import pyarrow.parquet as pq


from support import TempDirTestCase, read_parquet
import to_valplay_bundle as bundle


OLD_GUARDIAN = "/Game/Equippables/Guns/SniperRifles/Dmr/DMR.DMR_C"
NEW_GUARDIAN = "/Game/Equippables/Guns/SniperRifles/DMR/DMR.DMR_C"


class EquippableResolutionTests(unittest.TestCase):
    def test_guardian_directory_alias_resolves_at_firing_state_seam(self):
        resolved = bundle._resolve_equippable(
            41,
            {41: 42},
            {},
            {42: NEW_GUARDIAN},
        )

        self.assertEqual(
            resolved,
            (42, "Guardian", "rifle", OLD_GUARDIAN),
        )

    def test_unmeasured_guardian_case_variant_stays_unresolved(self):
        resolved = bundle._resolve_equippable(
            41,
            {41: 42},
            {},
            {42: NEW_GUARDIAN.replace("/DMR/", "/dMR/")},
        )

        self.assertIsNone(resolved)

    def test_every_key_shape_resolves_to_the_canonical_entry(self):
        for key in (OLD_GUARDIAN, NEW_GUARDIAN, OLD_GUARDIAN.rpartition(".")[0],
                    NEW_GUARDIAN.rpartition(".")[0], "Default__DMR_C"):
            with self.subTest(key):
                self.assertEqual(bundle.EQUIPPABLE_BY_PATH[key],
                                 ("Guardian", "rifle", OLD_GUARDIAN))


def col(kind, key=None, default=None) -> tuple:
    """One column of a table spec: its type, the row key that fills it (the
    column name unless given) and the value of a row without that key."""
    return kind, key, default


U32, F32, STR = pa.uint32(), pa.float32(), pa.string()
#: vrf-export dictionary-encodes path columns, which the adapter reads on a
#: separate path; a plain string column would exercise the wrong branch.
PATHS = pa.dictionary(pa.int32(), pa.string())

#: Every column the adapter reads from each table, keyed by table.
SPECS = {
    "fields": dict(
        time_ms=col(U32), packet_id=col(U32), channel_index=col(U32, default=7),
        actor_net_guid=col(U32, "actor"), object_net_guid=col(U32, "object"),
        group_path=col(STR), handle=col(U32, default=0), field_name=col(STR),
        bit_count=col(U32), raw_bits=col(pa.binary()), value_i64=col(pa.int64()),
        value_f64=col(pa.float64()), value_bool=col(pa.bool_()), value_str=col(STR)),
    "actors": dict(
        time_ms=col(U32, default=0), packet_id=col(U32, default=0),
        channel_index=col(U32, default=0), actor_net_guid=col(U32, "actor", 0),
        event=col(STR), class_path=col(PATHS), archetype_path=col(PATHS),
        **{f"spawn_{axis}": col(F32) for axis in ("x", "y", "z", "pitch", "yaw", "roll")}),
    "movement": dict(
        time_ms=col(U32, default=0), packet_id=col(U32, default=0),
        character_net_guid=col(U32, "char", 0),
        **{name: col(F32, default=0.0) for name in (
            "pos_x", "pos_y", "pos_z", "yaw", "pitch", "vel_x", "vel_y", "vel_z")}),
    "net_guids": dict(net_guid=col(U32), outer_net_guid=col(U32, "outer"), path=col(STR)),
    # The private columns carry a marker, so a whole-row copy past the
    # timeline allowlist fails loudly.
    "events": dict(
        id=col(STR, default="private-id"), group=col(STR),
        metadata=col(STR, default="private-metadata"), time1=col(U32, default=0),
        time2=col(U32, default=0), payload_size=col(pa.int32(), default=0),
        raw_payload=col(pa.binary(), default=b"private-payload"), word0=col(U32),
        word1=col(U32), payload_tag=col(U32), payload_name=col(STR),
        payload_seconds=col(F32)),
}


def write_table(path: Path, rows, spec: dict) -> None:
    pq.write_table(pa.table({
        name: pa.array([row.get(key or name, default) for row in rows], type=kind)
        for name, (kind, key, default) in spec.items()}), path)


#: One innocuous replicated property, so a case about another table still
#: has a fields.parquet to read.
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

#: A PlayerState row the parser could not name, beside MINIMAL_FIELD_ROWS'
#: one: a dropped row the loss tally must count.
UNNAMED_PROPERTY_ROW = {
    "time_ms": 10, "packet_id": 1, "actor": 101, "group_path": "PlayerState",
    "field_name": None, "bit_count": 3, "raw_bits": b"\x05",
}

MANIFEST = {"replay_version": "5.3.2"}
RPC_GROUP = "/Script/ShooterGame.DamageableComponent_ClassNetCache"
SHOT_RPC = "/Script/ShooterGame.ShooterCharacter_ClassNetCache"


def write_export(path: Path, fields=MINIMAL_FIELD_ROWS, *, actors=(), movement=(),
                 net_guids=(), timeline=(), manifest=MANIFEST) -> Path:
    """An export as `vrfkit export` writes one: every table (empty unless
    given; `timeline` is events.parquet) and the manifest."""
    path.mkdir(parents=True)
    for name, rows in (("fields", fields), ("actors", actors), ("movement", movement),
                       ("net_guids", net_guids), ("events", timeline)):
        write_table(path / f"{name}.parquet", list(rows), SPECS[name])
    (path / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    return path


class Converted(NamedTuple):
    """One conversion: `convert`'s result, each bundle file's bytes and what
    it printed."""

    summary: dict
    files: dict
    printed: str

    @property
    def tally(self):
        return self.summary["tally"]

    @property
    def manifest(self) -> dict:
        return json.loads(self.files["manifest.json"])

    def events(self, event_type=None) -> list[dict]:
        events = [json.loads(line) for line in self.files["events.ndjson"].splitlines()]
        return [e for e in events if event_type in (None, e["type"])]

    def movement(self) -> list[str]:
        return self.files["movement.ndjson"].decode("utf-8").splitlines()


def run(fields=MINIMAL_FIELD_ROWS, *, verbose=False, **tables) -> Converted:
    """Convert a fresh `write_export` of `fields` and `tables`."""
    printed = io.StringIO()
    with tempfile.TemporaryDirectory() as tmp, contextlib.redirect_stdout(printed):
        export = write_export(Path(tmp) / "export", fields, **tables)
        out = Path(tmp) / "bundle"
        summary = bundle.convert(export, out, verbose=verbose)
        files = {path.name: path.read_bytes() for path in out.iterdir()}
    return Converted(summary, files, printed.getvalue())


class RequiredInputTests(unittest.TestCase):
    """`vrfkit export` writes every table and the manifest in one transaction;
    an export missing one is refused, naming it, before anything is written."""

    def test_each_missing_input_is_named_and_nothing_is_written(self):
        for name in ("manifest.json", "fields.parquet", "actors.parquet",
                     "net_guids.parquet", "events.parquet", "movement.parquet"):
            with self.subTest(name), tempfile.TemporaryDirectory() as tmp:
                export = write_export(Path(tmp) / "export")
                (export / name).unlink()
                with self.assertRaisesRegex(FileNotFoundError, f"^{re.escape(name)} not found"):
                    bundle.convert(export, Path(tmp) / "out" / "bundle")
                self.assertFalse((Path(tmp) / "out").exists())


class MovementCollapseTests(unittest.TestCase):
    """The final move per PACKET is kept, never per millisecond (the reason is
    `_write_movement`'s)."""

    def test_two_packets_in_one_millisecond_each_keep_their_final_move(self):
        rows = [
            # Packet 1: two sub-moves, only the second survives.
            {"time_ms": 100, "packet_id": 1, "char": 42, "pos_x": 1.0},
            {"time_ms": 100, "packet_id": 1, "char": 42, "pos_x": 2.0},
            # Packet 2, same ms and character: its own final move.
            {"time_ms": 100, "packet_id": 2, "char": 42, "pos_x": 3.0},
        ]
        lines = run(movement=rows).movement()

        self.assertEqual(len(lines), 2, lines)
        self.assertIn('"x":2', lines[0])
        self.assertIn('"x":3', lines[1])


#: float32 values where a vectorised shortcut and the per-value encoder can
#: disagree, compared with the per-value rule, never with a literal, so the
#: test pins the contract and not today's spelling of it.
TEXT_RULE_EDGES = (
    float("inf"), float("-inf"), float("nan"),
    # >= 2**24 every float32 is an integer, but not every one prints as its
    # exact integer: 123456792 round-trips as 123456790.
    123456792.0, 1e16, 1e20,
    3.4028234663852886e38, -3.4028234663852886e38,        # +/- FLT_MAX
    16777216.0, 16777215.0, -16777215.0, 33554436.0,
    8388608.0, 12345670.0,
    # numpy writes non-integral values from 1e6 up in scientific notation;
    # the rule writes them positional. 999999.94 is the last one both agree on.
    8388607.5, 1234567.5, -1234567.5, 1000000.0625, 999999.94,
    # float32(1e-4) is 9.99999975e-05: numpy judges the binary value and
    # writes '1e-04', repr judges the decimal and writes '0.0001'.
    1e-4, -1e-4, 1.0000000474974513e-04, 9.9e-05,
    1e-05, 1e-07, 1e-45, 1.1754943508222875e-38,          # exponent form,
    #                                                       subnormal, min normal
    2382.2, 349.99, -349.99, 253.289794921875, 0.1, 1.5,
    0.0, -0.0,
)


def column_text(values, *, shorten):
    """Per-row text of `_json_scalar_column` for `values` as float32, fanned
    back out through its inverse as the writer does."""
    arr = numpy.array(values, dtype=numpy.float32)
    texts, inverse = bundle._json_scalar_column(arr, shorten=shorten)
    return texts.take(inverse).to_pylist()


def per_value_text(values, *, shorten):
    """The oracle: one encoder call per row, the rule before vectorisation."""
    encode = bundle._JSON.encode
    widened = [float(v) for v in numpy.array(values, dtype=numpy.float32)]
    if shorten:
        return [encode(bundle._f32_shortest(v)) for v in widened]
    return [encode(v) for v in widened]


class MovementTextRuleTests(unittest.TestCase):
    """Each distinct value's text is the per-value encoder's, the contract of
    `_json_scalar_column`."""

    def test_shortened_columns_match_the_per_value_encoder(self):
        self.assertEqual(
            column_text(TEXT_RULE_EDGES, shorten=True),
            per_value_text(TEXT_RULE_EDGES, shorten=True),
        )

    def test_unshortened_columns_match_the_per_value_encoder(self):
        self.assertEqual(
            column_text(TEXT_RULE_EDGES, shorten=False),
            per_value_text(TEXT_RULE_EDGES, shorten=False),
        )

    def test_a_candidate_rounded_past_float32_is_skipped_on_every_python(self):
        # struct.pack("f", x) past FLT_MAX returns inf on 3.12 and raises
        # OverflowError on 3.13, and shortening FLT_MAX tries 3.403e+38.
        # Emulate 3.13, so the property is checked whichever Python runs.
        real = bundle._struct

        class Strict:
            unpack = staticmethod(real.unpack)

            @staticmethod
            def pack(fmt, *values):
                out = real.pack(fmt, *values)
                if fmt.endswith("f") and any(
                        math.isfinite(v) and math.isinf(real.unpack(fmt, real.pack(fmt, v))[0])
                        for v in values):
                    raise OverflowError("float too large to pack with f format")
                return out

        flt_max = 3.4028234663852886e38
        with mock.patch.object(bundle, "_struct", Strict):
            self.assertEqual(bundle._f32_shortest(flt_max), 3.4028235e38)
            self.assertEqual(bundle._f32_shortest(-flt_max), -3.4028235e38)
            self.assertEqual(
                column_text(TEXT_RULE_EDGES, shorten=True),
                per_value_text(TEXT_RULE_EDGES, shorten=True),
            )

    def test_each_zero_keeps_its_own_sign(self):
        """-0.0 == 0.0; both orders, since which sign a value-level unique
        keeps depends on the sort's tie-break."""
        for values in ([0.0, -0.0, 5.0, -0.0], [-0.0, 0.0, 5.0, 0.0]):
            with self.subTest(values=values):
                self.assertEqual(column_text(values, shorten=False),
                                 per_value_text(values, shorten=False))
                self.assertEqual(column_text(values, shorten=True),
                                 per_value_text(values, shorten=True))


#: The movement line as the per-row writer spelled it: a COPY, not
#: `_MOVEMENT_LINE`, so a change to the constant or to how the writer derives
#: its fragments turns the oracle red instead of moving both sides at once.
ORACLE_MOVEMENT_LINE = (
    '{"time_ms":%s,"shooter_character_net_guid":%s,'
    '"position":{"x":%s,"y":%s,"z":%s},'
    '"velocity":{"x":%s,"y":%s,"z":%s},'
    '"yaw":%s,"pitch":%s}\n'
)


def oracle_movement_bytes(rows: list[dict]) -> bytes:
    """movement.ndjson as the per-row writer produced it: the last row per
    (packet_id, character), one encoder call per value, os.linesep endings."""
    last = {}
    for i, row in enumerate(rows):
        last[(row.get("packet_id", 0), row.get("char", 0))] = i
    encode = bundle._JSON.encode

    def f32(row, name):
        return float(numpy.float32(row.get(name, 0.0)))

    lines = []
    for i in sorted(last.values()):
        row = rows[i]
        lines.append(ORACLE_MOVEMENT_LINE % (
            row.get("time_ms", 0), row.get("char", 0),
            *(encode(bundle._f32_shortest(f32(row, n)))
              for n in ("pos_x", "pos_y", "pos_z", "vel_x", "vel_y", "vel_z")),
            encode(f32(row, "yaw")), encode(f32(row, "pitch")),
        ))
    return "".join(lines).replace("\n", os.linesep).encode("ascii")


def oracle_rows() -> list[dict]:
    """Integral, -0.0, exponent-form, large, non-finite and ordinary values,
    sub-moves to collapse, two characters, and large ids -- enough rows to
    span several write blocks at the block sizes the test patches in."""
    values = [0.0, -0.0, 1.0, -3.0, 1e-05, 1e-07, 2382.2, 349.99, -349.99,
              0.1, 1234567.5, 123456792.0, 16777215.0, 1e-4, 51292.77,
              float("inf"), float("-inf"), float("nan"), 253.289794921875]
    rows = []
    for i in range(61):
        v = values[i % len(values)]
        w = values[(i * 7 + 3) % len(values)]
        rows.append({
            "time_ms": 1000 + i // 3, "packet_id": 1 + i // 3,
            "char": (40, 4294967295)[i % 2],
            "pos_x": v, "pos_y": w, "pos_z": -v,
            "vel_x": w, "vel_y": v * 0.5, "vel_z": 0.0 if i % 5 else -0.0,
            "yaw": (0.0, -0.0, 359.9945068359375, 253.289794921875)[i % 4],
            "pitch": (-0.0, 0.0, 1e-05, 90.5)[i % 4],
        })
    return rows


class MovementLineAssemblyTests(TempDirTestCase):
    """movement.ndjson, assembled in Arrow, is the per-row oracle's bytes."""

    def write(self, rows: list[dict], block_rows: int) -> bytes:
        root = self.tmp()
        write_table(root / "movement.parquet", rows, SPECS["movement"])
        out = root / "out"
        out.mkdir()
        with mock.patch.object(bundle, "_MOVEMENT_BLOCK_ROWS", block_rows):
            bundle._write_movement(root / "movement.parquet", out, False)
        return (out / "movement.ndjson").read_bytes()

    def test_bytes_equal_the_per_row_writer(self):
        rows = oracle_rows()
        expected = oracle_movement_bytes(rows)
        # Guards on the oracle itself: sub-moves were collapsed, and the kept
        # lines span several of the 7-row blocks below with a partial last one.
        lines = expected.count(os.linesep.encode())
        self.assertLess(lines, len(rows))
        self.assertGreater(lines, 14)
        self.assertNotEqual(lines % 7, 0)
        # One block, full blocks plus a partial last one, one row per block.
        for block_rows in (1 << 18, 7, 1):
            with self.subTest(block_rows=block_rows):
                self.assertEqual(self.write(rows, block_rows), expected)

    def test_lines_end_the_way_the_text_mode_file_ended_them(self):
        data = self.write(oracle_rows(), 7)
        self.assertEqual(data.count(os.linesep.encode()), data.count(b"\n"))
        self.assertTrue(data.endswith(os.linesep.encode()))

    def test_an_empty_table_writes_an_empty_file(self):
        self.assertEqual(self.write([], 7), b"")

    def test_a_null_in_a_movement_column_stops_the_conversion(self):
        """A null would reach `to_numpy` as float64 NaN: float-spelled times."""
        for column in ("time_ms", "pos_x"):
            with self.subTest(column=column):
                root = self.tmp()
                path = root / "movement.parquet"
                write_table(path, oracle_rows()[:3], SPECS["movement"])
                table = read_parquet(path)
                index = table.schema.get_field_index(column)
                values = table.column(column).to_pylist()
                values[1] = None
                table = table.set_column(index, column, pa.array(
                    values, type=table.schema.field(column).type))
                pq.write_table(table, path)
                out = root / "out"
                out.mkdir()
                with self.assertRaisesRegex(ValueError, column):
                    bundle._write_movement(path, out, False)

    def test_only_the_arrays_own_bytes_are_written(self):
        """A slice shares its parent's data buffer, so the buffer holds bytes
        before and after the slice's values; only the values are the text."""
        sliced = pa.array(["a", "bb", "ccc", "dddd"], type=pa.string()).slice(1, 2)
        self.assertEqual(bytes(bundle._string_bytes(sliced)), b"bbccc")


class TransactionalConversionTests(TempDirTestCase):
    @staticmethod
    def snapshot(path: Path) -> dict[str, bytes]:
        return {
            item.relative_to(path).as_posix(): item.read_bytes()
            for item in sorted(path.rglob("*"))
            if item.is_file()
        }

    @staticmethod
    def make_export(path: Path) -> bytes:
        return (write_export(path) / "manifest.json").read_bytes()

    def test_overlapping_trees_are_rejected_before_writing(self):
        # The same directory, the output inside the input, the input inside
        # the output.
        for export, output in (("export", "export"), ("export", "export/bundle"),
                               ("output/export", "output")):
            with self.subTest(output=output), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                self.make_export(root / export)
                before = (sorted(root.rglob("*")), self.snapshot(root))

                with self.assertRaises(ValueError):
                    bundle.convert(root / export, root / output)

                self.assertEqual((sorted(root.rglob("*")), self.snapshot(root)), before)

    def test_conversion_failure_preserves_an_existing_complete_bundle(self):
        root = self.tmp()
        export = write_export(root / "export")
        (export / "fields.parquet").write_bytes(b"not parquet")
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
        root = self.tmp()
        export = root / "export"
        original = self.make_export(export)

        bundle.convert(export, root / "bundle")

        self.assertEqual((export / "manifest.json").read_bytes(), original)

    def test_the_summary_names_the_published_bundle(self):
        """Not the `.bundle.*` staging directory the publish renames away."""
        root = self.tmp()
        self.make_export(root / "export")
        output = root / "bundle"
        printed = io.StringIO()
        with contextlib.redirect_stdout(printed):
            bundle.convert(root / "export", output)
        summary = [ln for ln in printed.getvalue().splitlines()
                   if ln.startswith("Conversion ")]
        self.assertEqual(len(summary), 1, printed.getvalue())
        self.assertTrue(summary[0].endswith(": " + str(output.resolve())), summary)
        self.assertTrue(output.is_dir())

    def test_a_failed_publish_prints_no_completion_claim(self):
        root = self.tmp()
        self.make_export(root / "export")
        printed = io.StringIO()
        with (mock.patch.object(bundle, "_publish_bundle",
                                side_effect=OSError("disk full")),
              contextlib.redirect_stdout(printed),
              self.assertRaises(OSError)):
            bundle.convert(root / "export", root / "bundle")
        self.assertNotIn("Conversion ", printed.getvalue())

    def test_backup_cleanup_failure_does_not_turn_a_committed_publish_into_failure(self):
        root = self.tmp()
        staging = root / ".bundle.staging"
        output = root / "bundle"
        staging.mkdir()
        output.mkdir()
        (staging / "manifest.json").write_text("new", encoding="utf-8")
        (output / "manifest.json").write_text("old", encoding="utf-8")

        def fail_cleanup(path, parent):
            raise OSError(f"cannot remove {path} under {parent}")

        stderr = io.StringIO()
        with mock.patch.object(bundle, "remove_tree", fail_cleanup), contextlib.redirect_stderr(stderr):
            bundle._publish_bundle(staging, output)

        self.assertEqual(
            (output / "manifest.json").read_text(encoding="utf-8"), "new"
        )
        backups = list(root.glob(".bundle.backup.*"))
        self.assertEqual(len(backups), 1, backups)
        self.assertEqual(
            (backups[0] / "manifest.json").read_text(encoding="utf-8"), "old"
        )
        self.assertIn("backup", stderr.getvalue().lower())


class DefaultOutputTests(TempDirTestCase):
    def test_a_windows_source_file_names_the_bundle_directory_on_any_os(self):
        tmp = self.tmp()
        (tmp / "manifest.json").write_text(
            json.dumps({"source_file": "D:\\replays\\match.vrf"}), encoding="utf-8")
        with (mock.patch.object(sys, "argv", ["to_valplay_bundle.py", str(tmp)]),
              mock.patch.object(bundle.gc, "disable"),
              mock.patch.object(bundle, "convert") as convert):
            bundle.main()
        self.assertEqual(convert.call_args.args[1].name, "match")


class ShotEventTests(unittest.TestCase):
    def build_shot(self, scalar_params: dict) -> dict:
        return bundle._build_shot_event(
            bundle._ShotContext({}),
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

    def test_the_firing_state_chain_names_the_weapon_and_fire_mode(self):
        def decoded(blob, spec, tag_table, tally=None):
            return {"FiringState.FiringState": 41} if spec is bundle._EFFECT_OBJECTS else {}

        ctx = bundle._ShotContext({}, {41: 42}, {41: "ZoomedFiringState"}, {42: NEW_GUARDIAN})
        with mock.patch.object(bundle, "_decode_effect_blob", decoded):
            shot = bundle._build_shot_event(ctx, 1, 2, 3, 4, 5, {}, bundle._EffectBlobs())["shot"]
        self.assertEqual(shot["equippable"], {"net_guid": 42, "name": "Guardian",
                                              "category": "rifle", "class_path": OLD_GUARDIAN})
        self.assertEqual((shot["fire_mode"], shot["fire_mode_evidence"]),
                         ("alternate", "firing-state:ZoomedFiringState"))

    def test_wire_booleans_are_published_as_sent(self):
        shot = self.build_shot({"bTransient": False, "bLocalEffect": True})
        self.assertEqual((shot["is_transient"], shot["is_local_effect"]), (False, True))
        self.assertIs(self.build_shot({})["is_transient"], True)

    def test_an_undecoded_alliance_filter_passes_through_unchanged(self):
        """An untyped AllianceFilter's raw blob passes through, never a repr."""
        blob = {"BitCount": 3, "Data": "Aw=="}
        self.assertEqual(self.build_shot({"AllianceFilter": blob})["alliance_filter"], blob)
        self.assertIsNone(self.build_shot({})["alliance_filter"])


class EffectBlobBitLengthTests(unittest.TestCase):
    """The bit length comes from the parser, not the byte length; no corpus
    blob tells the two apart (see `_EffectBlob`)."""

    SPEC = bundle._EFFECT_FLOATS

    # A real FloatValues payload lifted from 02d4d478's fields.parquet: 50
    # bytes, declared 400 bits, four complete tag/value pairs. It is also the
    # first of the vectors pinned in crates/vrf-decode/src/effect/tests.rs.
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
        # The same 50 bytes; at a declared 350 bits the fourth pair is cut off.
        self.assertEqual(self.decode(self.BLOB, 400), self.FOUR_PAIRS)
        self.assertEqual(self.decode(self.BLOB, 350), self.THREE_PAIRS)

    def test_a_declared_length_past_the_bytes_stops_the_conversion(self):
        # 400 bits declared over 45 bytes: the fourth value runs off the end.
        # The per-bit reader raised IndexError there, as the bulk one must; a
        # bare slice would read the missing bytes as zeros, a plausible value.
        with self.assertRaises(IndexError):
            self.decode(self.BLOB[:45], 400)

    def test_a_short_read_leaves_the_position_at_the_declared_end(self):
        # As the per-bit reader left it; the decoder's consumed/skip_bits
        # resync after a failed value read counts on it.
        reader = bundle._BitReader(b"\xff\xff", 12)
        reader.read_bits(4)
        with self.assertRaises(EOFError):
            reader.read_bits(16)
        self.assertEqual(reader.tell(), 12)

    def test_a_shot_row_is_decoded_to_its_declared_length(self):
        tags = {"path": "NetworkGameplayTagNodeIndex",
                "fields": [{"handle": 285, "name": "FiringState.AmmoRemaining"}]}
        row = {"time_ms": 1, "packet_id": 1, "actor": 2, "group_path": SHOT_RPC,
               "field_name": "ReplayPlayContinuousEffectAtLocation.FloatValues",
               "raw_bits": self.BLOB}
        for bits, ammo in ((400, -1509722752), (350, None)):
            with self.subTest(bits=bits):
                (shot,) = run([{**row, "bit_count": bits}],
                              manifest={"net_field_export_groups": [tags]},
                              ).events("valorant_shot_received")
                self.assertEqual(shot["shot"]["ammo_remaining"], ammo)

    def test_absent_blob_decodes_to_an_empty_mapping(self):
        self.assertEqual(bundle._decode_effect_blob(None, self.SPEC, {}), {})


class EffectFramingTallyTests(unittest.TestCase):
    """Bits a blob's framing leaves unaccounted for reach
    `effect_array_residual_bits`, including after an unreadable or oversized
    element count."""

    SPEC = bundle._EFFECT_FLOATS
    # One float element (tag 284, value 1.0), element and array terminators.
    ONE_ELEMENT = bytes.fromhex("02021020390412400000803f0000")

    def residual(self, data: bytes, bit_count: int | None = None):
        tally = bundle._Tally()
        elements = bundle._decode_effect_elements(
            data, len(data) * 8 if bit_count is None else bit_count, self.SPEC, tally)
        return elements, tally["effect_array_residual_bits"]

    def test_a_well_formed_array_leaves_nothing(self):
        self.assertEqual(self.residual(self.ONE_ELEMENT), ([(284, 1.0)], 0))

    def test_a_tail_after_the_terminator_is_counted(self):
        self.assertEqual(self.residual(self.ONE_ELEMENT + bytes(6)), ([(284, 1.0)], 1))

    def test_a_sub_byte_tail_is_counted(self):
        # Rust rejects any leftover bit: ResidualBits { remaining: 4 }.
        self.assertEqual(self.residual(self.ONE_ELEMENT + b"\x00", len(self.ONE_ELEMENT) * 8 + 4),
                         ([(284, 1.0)], 1))

    def test_an_oversized_count_is_counted(self):
        # IntPacked 383, past Rust's MAX_ARRAY_COUNT of 256, then 40 bytes.
        self.assertEqual(self.residual(bytes([0xFF, 0x04]) + bytes(40), 336), ([], 1))

    def test_an_unreadable_count_with_bits_left_is_counted(self):
        # Six continuation bytes overflow IntPacked's 35-bit limit.
        self.assertEqual(self.residual(bytes([0xFF] * 6) + bytes(10)), ([], 1))

    def test_an_empty_array_is_not_counted(self):
        # Count 0 is the whole blob, or is followed by the array terminator
        # the Rust decoder also accepts.
        self.assertEqual(self.residual(b"\x00"), ([], 0))
        self.assertEqual(self.residual(b"\x00\x00"), ([], 0))

    def test_a_declared_element_that_never_arrives_is_a_half_read_pair(self):
        # Count 2, but only element 0 arrives before the array terminator.
        tally = bundle._Tally()
        blob = bytes.fromhex("04" + self.ONE_ELEMENT.hex()[2:])
        elements = bundle._decode_effect_elements(blob, len(blob) * 8, self.SPEC, tally)
        self.assertEqual(elements, [(284, 1.0), (None, None)])
        self.assertEqual(tally["effect_half_read_pairs"], 1)
        self.assertEqual(tally["effect_array_residual_bits"], 0)


class EffectPayloadWindowTests(unittest.TestCase):
    def test_recorded_short_float_is_rejected_but_full_width_decodes(self):
        for declared, raw, expected, half_read in (
            (1, "020210100212020000803f0000", [(1, None)], 1),
            (32, "020210100212400000803f0000", [(1, 1.0)], 0),
        ):
            with self.subTest(declared=declared):
                # Given: the same float bytes, with different declared windows.
                data = bytes.fromhex(raw)
                tally = bundle._Tally()

                # When
                elements = bundle._decode_effect_elements(
                    data, len(data) * 8, bundle._EFFECT_FLOATS, tally)

                # Then: neighboring bytes cannot supply a short float.
                self.assertEqual(elements, expected)
                self.assertEqual(tally["effect_half_read_pairs"], half_read)
                self.assertEqual(tally["effect_array_residual_bits"], half_read)

    def test_short_tag_preserves_its_value_and_the_next_element(self):
        # Given: a one-bit tag, a full float 1.0, then tag 2 / float 2.0.
        data = bytes.fromhex("0402100225800000007f0008202008248000000080000000")
        tally = bundle._Tally()

        # When: the blob ends at bit 185, not at the padded byte boundary.
        elements = bundle._decode_effect_elements(data, 185, bundle._EFFECT_FLOATS, tally)

        # Then
        self.assertEqual(elements, [(None, 1.0), (2, 2.0)])
        self.assertEqual(tally["effect_half_read_pairs"], 1)
        self.assertEqual(tally["effect_array_residual_bits"], 0)

    def test_short_values_preserve_the_next_element_for_each_array_kind(self):
        for spec, raw, bits, value in (
            (bundle._EFFECT_FLOATS, "040210100212020008202008248000000080000000",
             161, 2.0),
            (bundle._EFFECT_OBJECTS, "040220100222020008402008442008000000",
             137, 2),
            (bundle._EFFECT_VECTORS,
             "04021810021a020008302008340205000000000000000000000000000000000000000000000000000000",
             329, (0.0, 0.0, 0.0)),
        ):
            with self.subTest(spec=spec):
                # Given: tag 1 / a one-bit value, followed by a complete pair.
                data = bytes.fromhex(raw)
                tally = bundle._Tally()

                # When
                elements = bundle._decode_effect_elements(data, bits, spec, tally)

                # Then: the second pair survives even when not byte-aligned.
                self.assertEqual(elements, [(1, None), (2, value)])
                self.assertEqual(tally["effect_half_read_pairs"], 1)
                self.assertEqual(tally["effect_array_residual_bits"], 0)

    def test_malformed_shot_keeps_scalar_parameters_and_raw_rpc_evidence(self):
        # Given: malformed ammo plus an independent scalar on the same shot.
        data = bytes.fromhex("020210100212020000803f0000")
        common = {"time_ms": 30, "packet_id": 3, "actor": 2, "object": 22,
                  "group_path": SHOT_RPC, "handle": 9}
        rows = [
            {**common, "field_name": "ReplayPlayContinuousEffectAtLocation.FloatValues",
             "raw_bits": data, "bit_count": 96},
            {**common, "field_name": "ReplayPlayContinuousEffectAtLocation.EffectID",
             "value_i64": 42, "bit_count": 8},
        ]
        manifest = {"net_field_export_groups": [{
            "path": "NetworkGameplayTagNodeIndex",
            "fields": [{"handle": 1, "name": "FiringState.AmmoRemaining"}],
        }]}

        # When
        result = run(rows, manifest=manifest)

        # Then: no fabricated ammo, and the raw source remains available.
        (shot,) = result.events("valorant_shot_received")
        (rpc,) = result.events("rpc_received")
        self.assertIsNone(shot["shot"]["ammo_remaining"])
        self.assertEqual(shot["shot"]["effect_id"], 42)
        self.assertEqual(rpc["payload"]["FloatValues"],
                         {"BitCount": 96, "Data": base64.b64encode(data).decode("ascii")})
        self.assertEqual(result.tally["effect_half_read_pairs"], 1)


class ShotEffectRawSourceTests(unittest.TestCase):
    """An additive Rust JSON overlay must not replace the shot wire source."""

    def rows(self, typed_json: bool = False, **typed) -> list[dict]:
        values = (
            ("FloatValues", EffectBlobBitLengthTests.BLOB),
            ("ObjectValues", b"\x00"),
            ("VectorValues", b"\x00"),
        )
        return [{
            "time_ms": 30, "packet_id": 3, "actor": 2, "object": 22,
            "channel_index": 1, "group_path": SHOT_RPC, "handle": 9,
            "field_name": f"ReplayPlayContinuousEffectAtLocation.{name}",
            "bit_count": len(raw) * 8, "raw_bits": raw,
            **({"value_str": "[]"} if typed_json else {}), **typed,
        } for name, raw in values]

    def shot_and_rpc(self, rows: list[dict]) -> tuple[dict, dict, dict]:
        result = run(rows)
        (shot,) = result.events("valorant_shot_received")
        (rpc,) = result.events("rpc_received")
        return result.summary, shot, rpc

    def test_typed_json_overlay_keeps_raw_shot_and_rpc_payload(self):
        raw_summary, raw_shot, raw_rpc = self.shot_and_rpc(self.rows())
        typed_summary, typed_shot, typed_rpc = self.shot_and_rpc(
            self.rows(typed_json=True)
        )
        self.assertEqual(typed_shot, raw_shot)
        self.assertEqual(typed_rpc, raw_rpc)
        for name, raw, bits in (("FloatValues", EffectBlobBitLengthTests.BLOB, 400),
                                ("ObjectValues", b"\x00", 8), ("VectorValues", b"\x00", 8)):
            with self.subTest(name=name):
                self.assertEqual(typed_rpc["payload"][name], raw_rpc["payload"][name])
                self.assertEqual(typed_rpc["payload"][name], {
                    "BitCount": bits,
                    "Data": base64.b64encode(raw).decode("ascii"),
                })
        self.assertEqual(typed_summary["tally"]["multi_typed_rows"], raw_summary["tally"]["multi_typed_rows"])

    def test_malformed_typed_overlay_still_uses_raw_and_keeps_counter(self):
        _raw_summary, raw_shot, raw_rpc = self.shot_and_rpc(self.rows())
        summary, shot, rpc = self.shot_and_rpc(
            self.rows(value_i64=7, value_str="not-json")
        )
        self.assertEqual(shot, raw_shot)
        self.assertEqual(rpc, raw_rpc)
        self.assertEqual(summary["tally"]["multi_typed_rows"], 3)


class DeathMontageBlobTests(unittest.TestCase):
    """The death-montage pair, typed ObjectNetGuid, keeps the reference's
    {BitCount, Data, TypeName} blob shape, not a bare integer."""

    # 16-bit IntPacked 5055 (an FXC finisher class) and 8-bit 0 (null).
    OVERRIDE = (b"\x7f\x4e", 16, 5055)
    CONTEXT = (b"\x00", 8, 0)

    def payload(self, typed: bool) -> dict:
        rows = []
        for param, (raw, bits, guid) in (
            ("DeathMontageEffectOverride", self.OVERRIDE),
            ("DeathMontageEffectOverrideContext", self.CONTEXT),
        ):
            rows.append({
                "time_ms": 40, "packet_id": 4, "actor": 2, "object": 22,
                "group_path": RPC_GROUP, "handle": 2,
                "field_name": f"MulticastNotifyDamage_Point.{param}",
                "bit_count": bits, "raw_bits": raw,
                **({"value_i64": guid} if typed else {}),
            })
        rows.append({
            "time_ms": 40, "packet_id": 4, "actor": 2, "object": 22,
            "group_path": RPC_GROUP, "handle": 2,
            "field_name": "MulticastNotifyDamage_Point.bDeathMontageEffectOverrideIsQueued",
            "bit_count": 1, "raw_bits": b"\x00", "value_bool": False,
        })
        (rpc,) = run(rows).events("rpc_received")
        return rpc["payload"]

    def test_typed_rows_keep_the_reference_blob(self):
        typed = self.payload(typed=True)
        self.assertEqual(typed, self.payload(typed=False))
        for param, (raw, bits, _guid) in (
            ("DeathMontageEffectOverride", self.OVERRIDE),
            ("DeathMontageEffectOverrideContext", self.CONTEXT),
        ):
            self.assertEqual(typed[param], {
                "BitCount": bits,
                "Data": base64.b64encode(raw).decode("ascii"),
                "TypeName": param,
            })
        # The Bool sibling is not swept into the blob branch.
        self.assertIs(typed["bDeathMontageEffectOverrideIsQueued"], False)


class BlockPayloadExclusionTests(unittest.TestCase):
    marker = "__vrfkit_unresolved_class_net_cache_payload__"

    def test_block_payload_row_is_excluded_before_grouping(self):
        ordinary = [
            *MINIMAL_FIELD_ROWS,
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
        base, marked = run(ordinary), run(ordinary + [marker_row])

        self.assertEqual(marked.summary, base.summary)
        # The data files are byte-identical: the marker changes no event
        # or position.
        for name in ("events.ndjson", "movement.ndjson"):
            self.assertEqual(marked.files[name], base.files[name], name)
        # The manifest is NOT: it counts the skipped preservation row.
        base_manifest = base.manifest["adapter"]
        marked_manifest = marked.manifest["adapter"]
        self.assertEqual(base_manifest["field_rows_read"], 2)
        self.assertEqual(base_manifest["field_rows_unresolved_class_net_cache"], 0)
        self.assertEqual(marked_manifest["field_rows_read"], 3)
        self.assertEqual(marked_manifest["field_rows_unresolved_class_net_cache"], 1)
        events = marked.files["events.ndjson"]
        self.assertIn(b'"actor_net_guid":202', events)
        self.assertNotIn(b'"actor_net_guid":303', events)
        self.assertNotIn(self.marker.encode(), events)


class CombatReportLeafNameTests(unittest.TestCase):
    """Combat-report leaves are keyed on the handle, not the wire name (see
    "Combat report leaf labels" in the adapter)."""

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


class CombatReportLayoutTests(unittest.TestCase):
    GROUP = CombatReportLeafNameTests.GROUP

    def test_damage_icon_references_keep_the_declared_name_in_both_interactions(self):
        manifest = {**MANIFEST, "net_field_export_groups": [
            {"path": self.GROUP, "fields": [
                {"handle": 36, "name": "DamageIcon"},
                {"handle": 72, "name": "DamageIcon"},
            ]}]}
        members = [
            (handle, "Rounds[0].Reports[0].Interactions[0]." + name +
             "[0].DamageIcon", "value_i64", value)
            for handle, name, value in ((36, "DealtInteractions", 300),
                                        (72, "ReceivedInteractions", 800))
        ]
        report, _ = self.convert_rows(members, manifest=manifest)
        interaction = report["Interactions"][0]
        self.assertEqual(interaction["DealtInteractions"][0]["DamageIcon"], 300)
        self.assertEqual(interaction["ReceivedInteractions"][0]["DamageIcon"], 800)

    def convert_rows(self, members, *, manifest=MANIFEST):
        rows = [{"time_ms": 20, "packet_id": 2, "actor": 2,
                 "group_path": self.GROUP, "handle": handle,
                 "field_name": name, column: value}
                for handle, name, column, value in members]
        converted = run(rows, manifest=manifest)
        event, = converted.events("export_group_received")
        return event["payload"]["Rounds"][0]["Reports"][0], converted

    def test_shifted_regional_members_keep_their_values_and_reference_paths(self):
        members = []
        expected = {"Index": 0, "Region": 1, "Hits": 3, "Damage": 120.0,
                    "IsWallPen": True, "IsKill": False, "DestroyedArmor": 4628}
        leaves = [("Region", "value_i64", 1), ("Hits", "value_i64", 3),
                  ("Damage", "value_f64", 120.0), ("bIsWallPen", "value_bool", True),
                  ("bIsKill", "value_bool", False),
                  ("DestroyedArmor", "value_i64", 4628)]
        for interaction, start in (("DealtInteractions", 46),
                                   ("ReceivedInteractions", 82)):
            prefix = ("Rounds[0].Reports[0].Interactions[0]." + interaction
                      + "[0].RegionalDamageInteractions[0].")
            members.extend((start + offset, prefix + name, column, value)
                           for offset, (name, column, value) in enumerate(leaves))
        report, converted = self.convert_rows(members)
        interaction = report["Interactions"][0]
        for name in ("DealtInteractions", "ReceivedInteractions"):
            self.assertEqual(interaction[name][0], {"Index": 0, "Regions": [expected]})
        self.assertTrue(all(value == 0 for value in converted.tally.values()))

    def test_declared_names_resolve_shifted_nonregional_leaves(self):
        fields = [{"handle": h, "name": name} for h, name in
                  ((97, "CombatReportIndex"), (99, "ResurrectorPlayerState"),
                   (104, "bDied"), (6, "HUDConfig"), (100, "HUDConfig"))]
        manifest = {**MANIFEST, "net_field_export_groups": [
            {"path": self.GROUP, "fields": fields}]}
        members = [(h, "Rounds[0].Reports[0]." + name, column, value)
                   for h, name, column, value in
                   ((97, "CombatReportIndex", "value_i64", 4),
                    (99, "ResurrectorPlayerState", "value_i64", 4628),
                    (104, "bDied", "value_bool", True),
                    (6, "HUDConfig", "value_i64", 10),
                    (100, "HUDConfig", "value_i64", 20))]
        report, _ = self.convert_rows(members, manifest=manifest)
        self.assertEqual(report, {"Index": 0, "CombatReportIndex": 4,
                                 "ResurrectorPlayerState": 4628, "Died": True,
                                 "_h6": 10, "_h100": 20})

    def test_declarations_override_legacy_handle_meanings(self):
        manifest = {**MANIFEST, "net_field_export_groups": [
            {"path": self.GROUP, "fields": [{"handle": 47, "name": "HUDConfig"}]}]}
        report, _ = self.convert_rows(
            [(47, "Rounds[0].Reports[0].HUDConfig", "value_i64", 7)],
            manifest=manifest)
        self.assertEqual(report, {"Index": 0, "_h47": 7})

    def test_legacy_declarations_preserve_the_existing_event_bytes(self):
        prefix = ("Rounds[0].Reports[0].Interactions[0].DealtInteractions[0]"
                  ".Regions[0].")
        members = [(46, prefix + "Hits", "value_i64", 3),
                   (47, prefix + "Damage", "value_f64", 120.0),
                   (48, prefix + "bIsWallPen", "value_bool", True),
                   (6, "Rounds[0].Reports[0].HUDConfig", "value_i64", 10)]
        fields = [{"handle": h, "name": name.rsplit(".", 1)[-1]}
                  for h, name, _, _ in members]
        manifest = {**MANIFEST, "net_field_export_groups": [
            {"path": self.GROUP, "fields": fields}]}
        _, old = self.convert_rows(members)
        _, declared = self.convert_rows(members, manifest=manifest)
        self.assertEqual(declared.files["events.ndjson"], old.files["events.ndjson"])

    def test_regional_unknown_and_raw_leaves_keep_visible_names(self):
        prefix = ("Rounds[0].Reports[0].Interactions[0].DealtInteractions[0]"
                  ".RegionalDamageInteractions[0].")
        for name, handle in (("_h47", 47), ("_raw", (1 << 32) - 1)):
            with self.subTest(name=name):
                self.assertEqual(bundle._combat_report_leaf_name(
                    self.GROUP, prefix + name, handle),
                    prefix.replace("RegionalDamageInteractions", "Regions") + name)


class UnnamedRowTallyTests(unittest.TestCase):
    """A row the parser could not name is dropped and counted, as is an RPC
    whose rows are all unnamed (no row names its function)."""

    def test_an_unnamed_property_row_is_counted(self):
        tally = run(MINIMAL_FIELD_ROWS + [UNNAMED_PROPERTY_ROW]).tally
        self.assertEqual(tally["unnamed_property_rows"], 1)

    def test_an_rpc_of_only_unnamed_rows_counts_the_rows_and_the_invocation(self):
        rows = [
            {
                "time_ms": 20, "packet_id": 2, "actor": 202,
                "group_path": RPC_GROUP, "handle": 5,
                "field_name": None, "bit_count": 3, "raw_bits": b"\x05",
            },
            {
                "time_ms": 20, "packet_id": 2, "actor": 202,
                "group_path": RPC_GROUP, "handle": 5,
                "field_name": None, "bit_count": 4, "raw_bits": b"\x06",
            },
        ]
        result = run(rows)
        # The invocation really is gone -- the count is its only trace.
        self.assertEqual(result.events("rpc_received"), [])
        self.assertEqual(result.tally["unnamed_rpc_rows"], 2)
        self.assertEqual(result.tally["unnamed_rpc_invocations"], 1)


class TagTableTallyTests(unittest.TestCase):
    """Shots decoded without the manifest's gameplay-tag table are counted."""

    SHOT_ROWS = [
        {
            "time_ms": 30, "packet_id": 3, "actor": 2, "object": 22,
            "channel_index": 1, "group_path": SHOT_RPC, "handle": 9,
            "field_name": "ReplayPlayContinuousEffectAtLocation.FloatValues",
            "bit_count": len(EffectBlobBitLengthTests.BLOB) * 8,
            "raw_bits": EffectBlobBitLengthTests.BLOB,
        },
    ]

    def test_shots_decoded_with_no_tag_table_are_counted(self):
        self.assertEqual(run(self.SHOT_ROWS).tally["empty_gameplay_tag_table"], 1)

    def test_a_tag_table_from_the_manifest_clears_the_count(self):
        manifest = {
            "net_field_export_groups": [
                {
                    "path": "NetworkGameplayTagNodeIndex",
                    "fields": [{"handle": 263, "name": "FiringState.AmmoRemaining"}],
                }
            ]
        }
        tally = run(self.SHOT_ROWS, manifest=manifest).tally
        self.assertEqual(tally["empty_gameplay_tag_table"], 0)


class RpcCollisionTallyTests(unittest.TestCase):
    """Two same-packet invocations of one function by one actor collide into
    one group; the overwrite is counted, never split."""

    def test_a_repeated_parameter_in_one_group_is_counted(self):
        common = {
            "time_ms": 40, "packet_id": 4, "actor": 404,
            "group_path": RPC_GROUP, "handle": 12,
            "field_name": "MulticastNotifyKilledEnemy.MultikillLevel",
        }
        result = run([{**common, "bit_count": 8, "value_i64": 1},
                      {**common, "bit_count": 8, "value_i64": 2}])
        # One rpc_received for what were two calls, carrying only the
        # second call's value -- the loss the count names.
        rpcs = result.events("rpc_received")
        self.assertEqual(result.tally["rpc_param_collisions"], 1)
        self.assertEqual(len(rpcs), 1, rpcs)
        self.assertEqual(rpcs[0]["payload"], {"MultikillLevel": 2})

    def test_distinct_parameters_in_one_group_are_not_counted(self):
        common = {
            "time_ms": 40, "packet_id": 4, "actor": 404,
            "group_path": RPC_GROUP, "handle": 12, "bit_count": 8,
        }
        rows = [
            {**common, "field_name": "MulticastNotifyKilledEnemy.KillerCharacter",
             "value_i64": 1},
            {**common, "field_name": "MulticastNotifyKilledEnemy.KilledCharacter",
             "value_i64": 2},
        ]
        self.assertEqual(run(rows).tally["rpc_param_collisions"], 0)


class RpcObjectIdentityTests(unittest.TestCase):
    def test_distinct_subobjects_keep_their_own_parameters(self):
        # Given: the packet, actor, group and handle agree, but objects differ.
        common = {
            "time_ms": 40, "packet_id": 5, "actor": 3,
            "group_path": RPC_GROUP, "handle": 7, "bit_count": 8,
        }
        rows = [
            {**common, "object": 20, "field_name": "Fn.A", "value_i64": 11},
            {**common, "object": 30, "field_name": "Fn.B", "value_i64": 22},
        ]

        # When: real Parquet rows are converted and their NDJSON is read.
        result = run(rows)

        # Then: no consumer sees a combined invocation.
        self.assertEqual(
            [e["payload"] for e in result.events("rpc_received")],
            [{"A": 11}, {"B": 22}],
        )
        self.assertEqual(
            [(e["object_net_guid"], e["payload"]) for e in result.events("rpc_received")],
            [(20, {"A": 11}), (30, {"B": 22})],
        )
        self.assertEqual(result.tally["rpc_param_collisions"], 0)

    def test_shots_preserve_actor_rows_and_each_real_object_identity(self):
        # Given: None is an actor row, not object 0 or the actor's own GUID.
        objects = (None, 0, 3, 20, 30)
        rows = [{
            "time_ms": 40, "packet_id": 5, "actor": 3, "object": obj,
            "group_path": SHOT_RPC, "handle": 7, "bit_count": 8,
            "field_name": "ReplayPlayContinuousEffectAtLocation.EffectID",
            "value_i64": index,
        } for index, obj in enumerate(objects)]

        # When
        result = run(rows)

        # Then: both consumers retain identity and the corresponding parameter.
        self.assertEqual(len(result.events("valorant_shot_received")), 5)
        self.assertEqual(
            [(e["object_net_guid"], e["payload"]["EffectID"])
             for e in result.events("rpc_received")],
            list(zip(objects, range(5))),
        )
        self.assertEqual(
            [(e["object_net_guid"], e["shot"]["effect_id"])
             for e in result.events("valorant_shot_received")],
            list(zip(objects, range(5))),
        )
        self.assertEqual(result.tally["rpc_param_collisions"], 0)


class PropertyKeyCollisionTallyTests(unittest.TestCase):
    """Same-named rows in one property event (struct members or array
    elements only `handle` tells apart, like the crosshair profile's
    LineLength at 63/75/110): the last wins, and each overwrite is counted."""

    GROUP = "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C"

    def row(self, name, handle, value, column="value_f64", **extra):
        return {"time_ms": 25, "packet_id": 25, "actor": 7, "group_path": self.GROUP,
                "handle": handle, "field_name": name, "bit_count": 32,
                column: value, **extra}

    @staticmethod
    def convert(rows) -> tuple[dict, dict]:
        result = run(rows)
        (event,) = result.events("export_group_received")
        return event["payload"], result.tally

    def test_a_repeated_name_in_one_event_is_counted(self):
        payload, tally = self.convert(
            [self.row("LineLength", 63, 10.0), self.row("LineLength", 75, 2.0)])
        # Still last-wins: the count is what makes the loss visible.
        self.assertEqual(payload, {"LineLength": 2.0})
        self.assertEqual(tally["property_key_collisions"], 1)
        self.assertEqual(tally["payload_shape_conflicts"], 0)

    def test_distinct_names_and_separate_events_are_not_counted(self):
        rows = [self.row("LineLength", 63, 10.0), self.row("Opacity", 70, 0.5),
                self.row("LineLength", 63, 2.0, packet_id=26, time_ms=26)]
        self.assertEqual(run(rows).tally["property_key_collisions"], 0)

    def test_a_top_level_row_replacing_a_nested_value_is_a_shape_conflict(self):
        """'Foo' after 'Foo.Bar' is assigned directly, never by `_set_nested`."""
        payload, tally = self.convert([self.row("Foo.Bar", 1, 2.0), self.row("Foo", 2, 1.0)])
        self.assertEqual(payload, {"Foo": 1.0})
        self.assertEqual(tally["payload_shape_conflicts"], 1)
        self.assertEqual(tally["property_key_collisions"], 0)

    def test_a_repeated_blob_is_a_key_collision_not_a_shape_conflict(self):
        """Handles 23/24 of TrackedRewards[i].Rewards and 25/29 of A on
        02d4d478: two raw blobs under one name, nested or not, are one value
        overwriting another."""
        payload, tally = self.convert([
            self.row("TrackedRewards[0].Rewards", 23, b"\x01", "raw_bits", bit_count=8),
            self.row("TrackedRewards[0].Rewards", 24, b"\x02", "raw_bits", bit_count=8),
            self.row("A", 25, b"\x03", "raw_bits", bit_count=8),
            self.row("A", 29, b"\x04", "raw_bits", bit_count=8)])
        self.assertEqual(payload, {
            "TrackedRewards": [{"Index": 0, "Rewards": {"BitCount": 8, "Data": "Ag=="}}],
            "A": {"BitCount": 8, "Data": "BA=="}})
        self.assertEqual(tally["property_key_collisions"], 2)
        self.assertEqual(tally["payload_shape_conflicts"], 0)

    def test_an_array_element_is_a_value_a_structure_or_a_filler(self):
        """A value replacing a blob element collides, one replacing an element
        with members is a shape conflict, one landing on a filler is neither."""
        blob = self.row("Arr[0]", 1, b"\x01", "raw_bits", bit_count=8)
        last = self.row("Arr[0]", 9, 3.0)
        cases = (
            ("blob", [blob, last], [3.0], 1, 0),
            ("members", [self.row("Arr[0].X", 1, 1.0), last], [3.0], 0, 1),
            ("filler", [self.row("Arr[1].X", 1, 1.0), last],
             [3.0, {"Index": 1, "X": 1.0}], 0, 0),
            ("members over a filler", [self.row("Arr[1]", 1, 5.0),
                                       self.row("Arr[0].X", 2, 1.0), last],
             [3.0, 5.0], 0, 1),
        )
        for label, rows, array, collisions, conflicts in cases:
            with self.subTest(label):
                payload, tally = self.convert(rows)
                self.assertEqual(payload, {"Arr": array})
                self.assertEqual(tally["property_key_collisions"], collisions)
                self.assertEqual(tally["payload_shape_conflicts"], conflicts)

    def test_a_nested_leaf_replacing_members_is_a_shape_conflict(self):
        payload, tally = self.convert(
            [self.row("Foo.Bar.Baz", 1, 1.0), self.row("Foo.Bar", 2, 2.0)])
        self.assertEqual(payload, {"Foo": {"Bar": 2.0}})
        self.assertEqual(tally["payload_shape_conflicts"], 1)
        self.assertEqual(tally["property_key_collisions"], 0)

    def test_a_repeated_nested_leaf_is_counted(self):
        payload, tally = self.convert([self.row("Foo.Bar", 1, 1.0), self.row("Foo.Bar", 2, 2.0),
                                       self.row("Arr[0]", 3, 3.0), self.row("Arr[0]", 4, 4.0)])
        self.assertEqual(payload, {"Foo": {"Bar": 2.0}, "Arr": [4.0]})
        self.assertEqual(tally["property_key_collisions"], 2)
        self.assertEqual(tally["payload_shape_conflicts"], 0)

    def test_a_nested_container_row_is_skipped_before_or_after_its_elements(self):
        """`Sel[1].Att` carries the blob of `Sel[1].Att[i]`; the elements win."""
        container = self.row("Sel[1].Att", 1, b"\x01", "raw_bits", bit_count=8)
        elements = [self.row("Sel[1].Att[0]", 2, 5.0), self.row("Sel[1].Att[1]", 3, 6.0)]
        for label, rows in (("first", [container, *elements]), ("last", [*elements, container])):
            with self.subTest(label):
                payload, tally = self.convert(rows)
                self.assertEqual(payload, {"Sel": [{"Index": 1, "Att": [5.0, 6.0]}]})
                self.assertEqual(tally["payload_shape_conflicts"], 0)

    def test_a_real_index_member_replacing_the_injected_one_is_not_counted(self):
        """13.01, 12.05 and 11.06 exports carry real `TeamEconomy[i].Index`
        rows; replacing the injected Index loses nothing."""
        payload, tally = self.convert([self.row("TeamEconomy[0].Index", 1, 0, "value_i64"),
                                       self.row("TeamEconomy[0].Money", 2, 800, "value_i64"),
                                       self.row("TeamEconomy[1].Money", 3, 900, "value_i64"),
                                       self.row("TeamEconomy[1].Index", 4, 1, "value_i64")])
        self.assertEqual(payload, {"TeamEconomy": [
            {"Index": 0, "Money": 800}, {"Index": 1, "Money": 900}]})
        self.assertEqual(tally["property_key_collisions"], 0)


class FlatPathTallyTests(unittest.TestCase):
    """Unread subscripts ('Rounds[0][1]') and shape conflicts ('Foo' vs
    'Foo.Bar') are counted; non-identifier leaves are not."""

    def test_a_bare_numeric_segment_is_not_counted(self):
        # '248' is an unnamed handle's spelling, not a parse failure.
        tally = bundle._Tally()
        self.assertEqual(bundle._parse_field_path("248", tally), [("248", None)])
        self.assertEqual(tally["unparsable_path_segments"], 0)

    def test_a_blueprint_name_with_spaces_is_not_counted(self):
        # A leaf, correctly a literal key (the corpus spellings are in
        # `_parse_field_path_cached`).
        tally = bundle._Tally()
        for name in ("Victim FXC", "Set skeletal Collision", "Socket Name"):
            self.assertEqual(bundle._parse_field_path(name, tally),
                             [(name, None)])
        self.assertEqual(tally["unparsable_path_segments"], 0)

    def test_a_segment_whose_subscripts_did_not_parse_is_counted(self):
        # Unread brackets flatten their nesting into one literal key.
        tally = bundle._Tally()
        parts = bundle._parse_field_path("Rounds[0][1].Damage", tally)
        self.assertEqual(parts, [("Rounds[0][1]", None), ("Damage", None)])
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

    def test_members_rebuilt_after_a_conflict_are_still_members(self):
        """'Foo', 'Foo.Bar', 'Foo': both replacements restructure the key."""
        tally = bundle._Tally()
        payload = {}
        for path, value in (("Foo", 1), ("Foo.Bar", 2), ("Foo", 3)):
            bundle._set_nested(payload, bundle._parse_field_path(path), value, tally)
        self.assertEqual(payload, {"Foo": 3})
        self.assertEqual(tally["payload_shape_conflicts"], 2)
        self.assertEqual(tally["property_key_collisions"], 0)

    def test_ordinary_nesting_is_not_counted(self):
        tally = bundle._Tally()
        payload = {}
        for path, value in (("Rounds[0].Damage", 5), ("Rounds[1].Damage", 7),
                            ("Rounds[0].Kills", 2), ("Plain", 1)):
            bundle._set_nested(payload, bundle._parse_field_path(path), value, tally)
        self.assertEqual(tally["payload_shape_conflicts"], 0)
        self.assertEqual(tally["unparsable_path_segments"], 0)
        self.assertEqual(tally["property_key_collisions"], 0)

    def test_life_change_child_rows_are_still_dropped(self):
        # Members spelled with '[0]' never reach the payload as flat keys.
        for param in ("LifeChangeEvents[0].LifeResult",
                      "LifeChangeBySection[1].Amount"):
            self.assertIsNone(
                bundle._normalize_rpc_param(
                    "MulticastNotifyDamage_Point", param, 3, False
                )
            )


class TypedColumnTallyTests(unittest.TestCase):
    """More than one typed column set is counted as `multi_typed_rows`;
    raw_bits beside one typed value is not."""

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
        tally = bundle._Tally()
        bundle._get_value(1, None, None, None, b"\x01", 8, tally)
        self.assertEqual(tally["multi_typed_rows"], 0)


class FabricatedShotGeometryTests(unittest.TestCase):
    """A shot with no readable location or rotation gets zeros, counted; a
    parsed (0,0,0) is real and not counted."""

    #: RotationShort pitch and yaw set; 17 declared bits hold only the pitch.
    ROTATION = "l/pPkgE="

    def test_only_a_fabricated_value_is_counted(self):
        origin = {"x": 0, "y": 0, "z": 0}
        zero = {"pitch": 0, "yaw": 0, "roll": 0}
        pitch_yaw = {"pitch": 356.1932373046875, "yaw": 141.4324951171875, "roll": 0.0}
        for key, value, published, fabricated in (
            ("248", None, origin, 1),
            ("248", "(1,2)", origin, 1),
            ("248", "(1,2,3)", {"x": 1, "y": 2, "z": 3}, 0),
            ("248", "(0,0,0)", origin, 0),
            ("249", {"BitCount": 35, "Data": self.ROTATION}, pitch_yaw, 0),
            ("249", {"BitCount": 17, "Data": self.ROTATION}, zero, 1),
        ):
            with self.subTest(key=key, value=value):
                tally = bundle._Tally()
                params = {} if value is None else {key: value}
                shot = bundle._build_shot_event(
                    bundle._ShotContext({}), 1, 2, 3, 4, 5, params,
                    bundle._EffectBlobs(), tally=tally)["shot"]
                field, counter = (("location", "fabricated_shot_locations") if key == "248"
                                  else ("rotation", "fabricated_shot_rotations"))
                self.assertEqual(shot[field], published)
                self.assertEqual(tally[counter], fabricated)


class RawSourcedFieldTests(unittest.TestCase):
    """A field whose consumer decodes the raw wire blob gets the blob, typed
    or not, and a blob that cannot be built is counted (RAW_BLOB_PREFERRED)."""

    OEPI = "/Script/ShooterGame.OwnerExclusivePlayerInfo"
    RI_RAW = bytes.fromhex("0102030405")
    RI_BLOB = {
        "BitCount": 40,
        "Data": base64.b64encode(RI_RAW).decode("ascii"),
        "TypeName": "TArray<FAresPlayerRoundInfo>",
    }

    def roundinfos(self, **overrides) -> dict:
        row = {
            "time_ms": 10, "packet_id": 1, "actor": 5, "object": 5,
            "group_path": self.OEPI, "handle": 39, "field_name": "RoundInfos",
            "bit_count": 40, "raw_bits": self.RI_RAW,
        }
        row.update(overrides)
        return row

    def roundinfos_child(self) -> dict:
        return {
            "time_ms": 10, "packet_id": 1, "actor": 5, "object": 5,
            "group_path": self.OEPI, "handle": 43,
            "field_name": "RoundInfos[0].EndOfRoundMoney",
            "bit_count": 32, "value_i64": 900,
        }

    @staticmethod
    def payload(rows, event_type="export_group_received") -> tuple[dict, dict]:
        result = run(rows)
        (event,) = result.events(event_type)
        return event["payload"], result.tally

    def test_an_untyped_roundinfos_row_publishes_its_blob(self):
        """Children first, then the parent row, as stream.rs writes them."""
        payload, tally = self.payload([self.roundinfos_child(), self.roundinfos()])
        self.assertEqual(payload, {"RoundInfos": self.RI_BLOB})
        self.assertEqual(tally["raw_blobs_unavailable"], 0)
        self.assertEqual(tally["property_key_collisions"], 0)

    def test_a_repeated_roundinfos_row_in_one_event_is_counted(self):
        """Two RoundInfos rows in one event: only the last blob survives."""
        second = bytes.fromhex("0a0b0c0d0e")
        payload, tally = self.payload([self.roundinfos(), self.roundinfos(raw_bits=second)])
        self.assertEqual(payload["RoundInfos"]["Data"],
                         base64.b64encode(second).decode("ascii"))
        self.assertEqual(tally["property_key_collisions"], 1)

    def test_a_typed_roundinfos_row_still_publishes_its_raw_blob(self):
        """The defect: a value_str beside raw_bits made the payload `{}`."""
        payload, tally = self.payload(
            [self.roundinfos_child(), self.roundinfos(value_str="[]")])
        self.assertEqual(payload, {"RoundInfos": self.RI_BLOB})
        self.assertEqual(tally["raw_blobs_unavailable"], 0)

    def test_a_roundinfos_row_without_raw_bits_is_counted(self):
        """No raw bits, no blob: counted, and the typed value is not thrown away."""
        payload, tally = self.payload(
            [self.roundinfos(raw_bits=None, bit_count=None, value_str="[]")])
        self.assertEqual(tally["raw_blobs_unavailable"], 1)
        self.assertEqual(payload, {"RoundInfos": "[]"})

    def test_roundinfos_children_without_their_blob_are_counted(self):
        """Children are dropped (the blob carries them): without it, a loss."""
        payload, tally = self.payload([self.roundinfos_child()])
        self.assertEqual(payload, {})
        self.assertEqual(tally["raw_blobs_unavailable"], 1)

    DAMAGE = "MulticastNotifyDamage_Point"
    LCE_RAW = bytes.fromhex("02021620772418400000ce421a400000b0c11c02010000")
    LCE_BLOB = {
        "BitCount": 177,
        "Data": base64.b64encode(LCE_RAW).decode("ascii"),
        "TypeName": "LifeChangeEvents",
    }

    def damage_payload(self, with_blob=True, **parent) -> tuple[dict, dict]:
        """One damage invocation shaped like the corpus: every row carries the
        function's handle; decoded members precede the parent blob row."""
        common = {"time_ms": 20, "packet_id": 2, "actor": 7,
                  "group_path": RPC_GROUP, "handle": 1}
        lce = {**common, "field_name": f"{self.DAMAGE}.LifeChangeEvents",
               "bit_count": 177, "raw_bits": self.LCE_RAW, **parent}
        rows = [
            {**common, "field_name": f"{self.DAMAGE}.DamageTaken",
             "bit_count": 32, "value_f64": 30.0},
            {**common,
             "field_name": f"{self.DAMAGE}.LifeChangeEvents[0].LifeResult",
             "bit_count": 32, "value_f64": 70.0, "raw_bits": b"\x00\x00\x8cB"},
        ]
        return self.payload(rows + [lce] * with_blob, "rpc_received")

    def test_an_untyped_life_change_blob_is_unchanged(self):
        payload, tally = self.damage_payload()
        self.assertEqual(payload, {"DamageTaken": 30.0,
                                   "LifeChangeEvents": self.LCE_BLOB})
        self.assertEqual(tally["raw_blobs_unavailable"], 0)

    def test_a_typed_life_change_row_still_publishes_its_raw_blob(self):
        """valplay's HP decoder reads the blob, never the typed value."""
        payload, tally = self.damage_payload(value_str="[{}]")
        self.assertEqual(payload["LifeChangeEvents"], self.LCE_BLOB)
        self.assertEqual(tally["raw_blobs_unavailable"], 0)

    def test_a_life_change_row_without_raw_bits_is_counted(self):
        payload, tally = self.damage_payload(raw_bits=None, bit_count=None,
                                             value_str="[{}]")
        self.assertEqual(tally["raw_blobs_unavailable"], 1)
        self.assertEqual(payload["LifeChangeEvents"], "[{}]")

    def test_life_change_members_without_their_blob_are_counted(self):
        payload, tally = self.damage_payload(with_blob=False)
        self.assertNotIn("LifeChangeEvents", payload)
        self.assertEqual(tally["raw_blobs_unavailable"], 1)

    def test_shot_arrays_without_raw_bits_are_counted(self):
        """Their consumer is this file's own effect decoder; it gets nothing."""
        rows = [{
            "time_ms": 30, "packet_id": 3, "actor": 2, "object": 22,
            "channel_index": 1, "group_path": SHOT_RPC,
            "handle": 9,
            "field_name": f"ReplayPlayContinuousEffectAtLocation.{name}",
            "value_str": "[]",
        } for name in ("FloatValues", "ObjectValues", "VectorValues")]
        self.assertEqual(run(rows).tally["raw_blobs_unavailable"], 3)


class DamagedBoneTests(unittest.TestCase):
    """A decoded DamagedBone passes through; an undecoded one is null and
    counted, never rendered from the bytes."""

    FIELD = "MulticastNotifyDamage_Point.DamagedBone"
    # An FName "Head" as it sits in the corpus (105 bits).
    HEAD_RAW = bytes.fromhex("0a00000090cac2c8aa00000000")

    def bone_payload(self, **row) -> tuple[dict, dict]:
        return RawSourcedFieldTests.payload([{
            "time_ms": 20, "packet_id": 2, "actor": 7, "group_path": RPC_GROUP,
            "handle": 1, "field_name": self.FIELD, "bit_count": 105,
            "raw_bits": self.HEAD_RAW, **row}], "rpc_received")

    def test_a_decoded_bone_is_passed_through(self):
        payload, tally = self.bone_payload(value_str="Head")
        self.assertEqual(payload, {"DamagedBone": "Head"})
        self.assertEqual(tally["damaged_bone_undecoded"], 0)

    def test_an_undecoded_bone_is_null_and_counted(self):
        payload, tally = self.bone_payload()
        self.assertEqual(payload, {"DamagedBone": None})
        self.assertEqual(tally["damaged_bone_undecoded"], 1)


class RawGateHardeningTests(unittest.TestCase):
    """A typed row keeps its value through the `is_raw` gates."""

    def test_a_typed_container_row_does_not_replace_its_decoded_elements(self):
        """Element rows first, then the container, as stream.rs writes them."""
        common = {"time_ms": 10, "packet_id": 1, "actor": 5,
                  "group_path": "/Game/Test/Holder.Holder_C"}
        payload, tally = RawSourcedFieldTests.payload([
            {**common, "field_name": "Items[0].Count", "bit_count": 32,
             "value_i64": 3},
            {**common, "field_name": "Items", "bit_count": 40,
             "raw_bits": b"\x01\x02\x03\x04\x05", "value_str": "[3]"},
        ])
        self.assertEqual(payload, {"Items": [{"Index": 0, "Count": 3}]})
        self.assertEqual(tally["payload_shape_conflicts"], 0)

    def test_a_typed_function_row_is_carried(self):
        """A row that IS the function carries its value, typed or raw."""
        payload, _ = RawSourcedFieldTests.payload([{
            "time_ms": 20, "packet_id": 2, "actor": 7, "group_path": RPC_GROUP,
            "handle": 4, "field_name": "MulticastSomething", "bit_count": 8,
            "value_i64": 5}], "rpc_received")
        self.assertEqual(payload, {"MulticastSomething": 5})


class AdapterMappingTests(unittest.TestCase):
    """The mappings to the reference's shape, each pinned."""

    GROUP = "/Game/Test/Holder.Holder_C"
    DAMAGE = "MulticastNotifyDamage_Point"

    def property_payload(self, rows: list[dict]) -> dict:
        common = {"time_ms": 10, "packet_id": 1, "actor": 5,
                  "group_path": self.GROUP, "bit_count": 8}
        return RawSourcedFieldTests.payload([{**common, **row} for row in rows])[0]

    def test_regional_damage_ordinals_follow_the_enum(self):
        for ordinal, name in ((0, "regional_damage__normal"),
                              (1, "regional_damage__headshot"),
                              (2, "regional_damage__legshot"),
                              (5, "regional_damage__invalid"),
                              (9, "regional_damage__unknown_9")):
            with self.subTest(ordinal=ordinal):
                self.assertEqual(
                    bundle._normalize_rpc_param(self.DAMAGE, "RegionalDamage", ordinal, False),
                    {"RegionalDamage": name})

    def test_alliance_ordinals_follow_the_enum(self):
        for ordinal, name in ((0, "alliance_ally"), (1, "alliance_enemy"),
                              (3, "alliance_any"), (9, "alliance_unknown_9")):
            with self.subTest(ordinal=ordinal):
                shot = bundle._build_shot_event(
                    bundle._ShotContext({}), 1, 2, 3, 4, 5,
                    {"AllianceFilter": ordinal}, bundle._EffectBlobs())["shot"]
                self.assertEqual(shot["alliance_filter"], name)

    def test_damage_booleans_lose_their_b_prefix(self):
        for wire in ("bDamageKilledTarget", "bAliveAfterDamage", "bIsWallPenetration",
                     "bEquippableUsedZoomed", "bEquippableUsedInFocusMode"):
            with self.subTest(param=wire):
                self.assertEqual(bundle._normalize_rpc_param(self.DAMAGE, wire, True, False),
                                 {wire[1:]: True})

    def test_equippable_used_takes_the_reference_shape(self):
        self.assertEqual(
            bundle._normalize_rpc_param(self.DAMAGE, "EquippableUsed", 1234, False),
            {"EquippableUsed": {"NetGuid": 1234, "Name": None, "ClassPath": None,
                                "Category": "unknown"}})
        blob = {"BitCount": 16, "Data": "fwE="}
        self.assertEqual(
            bundle._normalize_rpc_param(self.DAMAGE, "EquippableUsed", blob, True),
            {"EquippableUsed": blob})

    def test_fire_mode_comes_from_the_firing_state_name(self):
        def mode(path, source_id=None):
            return bundle._resolve_fire_mode(41, source_id, {}, {41: path} if path else {})
        self.assertEqual(mode("FiringState"), ("primary", "firing-state:FiringState"))
        self.assertEqual(mode("ZoomedFiringState"),
                         ("alternate", "firing-state:ZoomedFiringState"))
        self.assertEqual(mode("FiringState", "Gun_AltFire_1"),
                         ("alternate", "source:Gun_AltFire_1"))
        self.assertEqual(mode(None), ("unknown", None))

    def test_package_path_drops_only_the_class_suffix(self):
        self.assertEqual(
            bundle._to_package_path("/Game/Characters/Hunter/Hunter_PC.Hunter_PC_C"),
            "/Game/Characters/Hunter/Hunter_PC")
        self.assertEqual(bundle._to_package_path("/Game/A.B/Thing"), "/Game/A.B/Thing")

    def test_property_values_are_reshaped_by_name(self):
        payload = self.property_payload([
            {"handle": 1, "field_name": "ReplicatedGravityDirection", "value_str": "(0,0,-1)"},
            {"handle": 2, "field_name": "ReplicatedMovement",
             "value_str": '{"location":{"x":1,"y":2,"z":3}}'},
            {"handle": 3, "field_name": "bUltimateActive", "value_bool": True},
            {"handle": 4, "field_name": "bottomless", "value_bool": True},
            {"handle": 5, "field_name": "bIsCounted", "value_i64": 2},
        ])
        self.assertEqual(payload, {
            "ReplicatedGravityDirection": {"x": 0, "y": 0, "z": -1},
            "ReplicatedMovement": {"location": {"x": 1, "y": 2, "z": 3}},
            "UltimateActive": True,
            "bottomless": True,
            "bIsCounted": 2,
        })

    def test_array_fillers_are_dropped_but_scalar_positions_are_kept(self):
        """Element [1] without [0]: the `{}` filler goes, a scalar None stays."""
        payload = self.property_payload([
            {"handle": 1, "field_name": "Teams[1].Score", "value_i64": 5},
            {"handle": 2, "field_name": "Scores[1]", "value_i64": 7},
        ])
        self.assertEqual(payload, {"Teams": [{"Index": 1, "Score": 5}],
                                   "Scores": [None, 7]})


class SummaryReportingTests(unittest.TestCase):
    """The summary must not say 'complete' about a conversion that lost rows."""

    def test_a_lossy_conversion_names_every_loss_in_its_summary(self):
        tally = run(MINIMAL_FIELD_ROWS + [UNNAMED_PROPERTY_ROW]).tally
        counts = self.printed_counts(tally.lines())
        self.assertEqual(list(counts), list(bundle._Tally.REASONS))
        self.assertEqual(counts["unnamed_property_rows"], "1")
        self.assertEqual(counts["rpc_param_collisions"], "0")

    def test_a_clean_conversion_prints_every_counter_as_zero(self):
        """A line printed only when non-zero could not tell "nothing was lost"
        from "this counter stopped running"."""
        result = run()
        self.assertEqual(result.tally.total, 0)
        counts = self.printed_counts(result.printed.splitlines())
        self.assertEqual(list(counts), list(bundle._Tally.REASONS))
        self.assertEqual(set(counts.values()), {"0"})

    @staticmethod
    def printed_counts(lines) -> dict:
        """{counter: its count as printed} from the summary's loss lines."""
        return dict(match.groups() for match in map(
            re.compile(r"  (\w+): ([\d,]+) -- ").match, lines) if match)


# ---------------------------------------------------------------------------
# The seam: what the bundle manifest carries and omits, and the order of
# events.ndjson. valplay cannot check any of it, so it fails here instead.
# ---------------------------------------------------------------------------

#: Shaped like crates/vrfkit/src/manifest.rs's quality object, with enough
#: members to prove the forwarding verbatim rather than a hand-picked subset.
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

UPSTREAM_LEVELS = [
    {"name": "/Game/Maps/Infinity/Infinity", "time_ms": 0},
]

#: An account UUID shaped like the manifest's `players` entries, present in
#: the EXPORT manifest so the omission test proves something absent.
UPSTREAM_PLAYERS = [
    {
        "actor_net_guid": 101,
        "subject": "11111111-2222-3333-4444-555555555555",
        "character_net_guid": 501,
    },
]


def full_manifest(**overrides) -> dict:
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
        "level_names_and_times": json.loads(json.dumps(UPSTREAM_LEVELS)),
        "game_specific_data": [
            '{"subject":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"}',
        ],
    }
    manifest.update(overrides)
    return manifest


#: A settled Sage wall opens, then goes DORMANT, still standing: no `close`,
#: because nothing destroyed it. A second wall is destroyed early.
SAGE_WALL_CLASS = "/Game/Characters/Sage/S0/Ability_Barrier/BarrierProjectile_C"

DORMANCY_ACTOR_ROWS = [
    {"time_ms": 100, "packet_id": 1, "actor": 501, "event": "open",
     "class_path": SAGE_WALL_CLASS, "archetype_path": "Default__Barrier",
     "spawn_x": 1.0, "spawn_y": 2.0, "spawn_z": 3.0},
    # Destroyed for real, early.
    {"time_ms": 200, "packet_id": 2, "actor": 502, "event": "open",
     "class_path": SAGE_WALL_CLASS, "archetype_path": "Default__Barrier",
     "spawn_x": 4.0, "spawn_y": 5.0, "spawn_z": 6.0},
    {"time_ms": 3200, "packet_id": 3, "actor": 502, "event": "close"},
    # Settles into dormancy, still alive.
    {"time_ms": 40100, "packet_id": 4, "actor": 501, "event": "dormant"},
]


class ActorLifecycleEventTests(unittest.TestCase):
    """The three `actors.event` values: dormant is its own event, never a
    despawn (see `_ACTOR_EVENT_TYPES`)."""

    @staticmethod
    def lifecycle(result: Converted) -> list[dict]:
        return [e for e in result.events() if e["type"].startswith("actor_")]

    def test_each_row_is_one_event_and_dormant_is_not_a_despawn(self):
        """Nor dropped; each carries its wire value, and spawns keep their
        class, archetype and location."""
        result = run(actors=DORMANCY_ACTOR_ROWS)
        spawn = {"type": "actor_spawned", "channel": 0,
                 "replication_class_path": SAGE_WALL_CLASS,
                 "archetype_path": "Default__Barrier", "rotation": None}
        self.assertEqual(self.lifecycle(result), [
            {**spawn, "time_ms": 100, "actor_net_guid": 501,
             "location": {"x": 1, "y": 2, "z": 3}},
            {**spawn, "time_ms": 200, "actor_net_guid": 502,
             "location": {"x": 4, "y": 5, "z": 6}},
            {"type": "actor_closed", "time_ms": 3200, "actor_net_guid": 502,
             "channel": 0, "actor_event": "close"},
            {"type": "actor_dormant", "time_ms": 40100, "actor_net_guid": 501,
             "channel": 0, "actor_event": "dormant"},
        ])
        self.assertEqual(result.manifest["adapter"]["losses"]["unknown_actor_lifecycle_events"], 0)

    def test_an_unknown_event_value_is_not_folded_into_a_close(self):
        """A fourth value must be a visible unknown, never a plausible despawn."""
        rows = [
            {"time_ms": 100, "packet_id": 1, "actor": 601, "event": "open",
             "class_path": SAGE_WALL_CLASS, "archetype_path": "Default__X"},
            {"time_ms": 200, "packet_id": 2, "actor": 601,
             "event": "torn_off_in_a_future_build"},
        ]
        result = run(actors=rows, verbose=True)
        events = self.lifecycle(result)
        self.assertEqual([e["type"] for e in events],
                         ["actor_spawned", "actor_lifecycle_unknown"], events)
        self.assertEqual(events[1]["actor_event"], "torn_off_in_a_future_build")
        self.assertEqual(
            result.manifest["adapter"]["losses"]["unknown_actor_lifecycle_events"], 1,
            "an unrecognised lifecycle value was published without a count",
        )
        # The verbose breakdown prints the types that did not occur, as zeros.
        for line in ("actor_closed: 0", "actor_dormant: 0", "actor_lifecycle_unknown: 1"):
            self.assertIn(f"    {line}\n", result.printed)

    def test_spawn_channel_and_rotation_cross_without_fabrication(self):
        rows = [
            {
                "time_ms": 100, "packet_id": 1, "channel_index": 17,
                "actor": 701, "event": "open",
                "class_path": SAGE_WALL_CLASS,
                "archetype_path": "Default__Rotated",
                "spawn_pitch": 12.5, "spawn_yaw": 90.0,
                "spawn_roll": -3.25,
            },
            {
                "time_ms": 200, "packet_id": 2, "channel_index": 18,
                "actor": 702, "event": "open",
                "class_path": SAGE_WALL_CLASS,
                "archetype_path": "Default__Static",
            },
            {
                "time_ms": 300, "packet_id": 3, "channel_index": 17,
                "actor": 701, "event": "close",
            },
        ]
        events = run(actors=rows).events()
        rotated = next(e for e in events if e.get("actor_net_guid") == 701
                       and e["type"] == "actor_spawned")
        static = next(e for e in events if e.get("actor_net_guid") == 702)
        closed = next(e for e in events if e["type"] == "actor_closed")
        self.assertEqual(rotated["channel"], 17)
        self.assertEqual(rotated["rotation"], {
            "pitch": 12.5, "yaw": 90, "roll": -3.25,
        })
        self.assertIsNone(static["rotation"])
        self.assertEqual(closed["channel"], 17)


class UpstreamAccountingForwardingTests(unittest.TestCase):
    """vrfkit's own accounting reaches the bundle manifest; private data does
    not."""

    def test_accounting_crosses_verbatim_and_the_shape_is_pinned(self):
        result = run(manifest=full_manifest())
        published, raw = result.manifest, result.files["manifest.json"]
        self.assertEqual(published["quality"], UPSTREAM_QUALITY)
        self.assertEqual(published["net_field_export_groups"], UPSTREAM_GROUPS)
        # `players` stays behind (why: docs/USAGE.md, "What the bundle
        # manifest carries").
        self.assertNotIn("players", published)
        self.assertNotIn(b"11111111-2222-3333-4444-555555555555", raw)
        # LF on every platform, like the extractors' receipts.
        self.assertIn(b"\n", raw)
        self.assertNotIn(b"\r\n", raw)
        # The bundle's shape is a contract, so it is spelled out once.
        self.assertEqual(
            sorted(published),
            sorted([
                "adapter",
                "bundle_schema_version",
                "converter",
                "duration_ms",
                "level_names_and_times",
                "net_field_export_groups",
                "quality",
                "replay_build",
                "replay_changelist",
                "replay_version",
                "source_file",
                "source_size_bytes",
            ]),
        )

    def test_public_level_names_are_forwarded_without_private_header_data(self):
        """The level root identifies the map; the adjacent header blob may identify players."""
        manifest = full_manifest()
        manifest["level_names_and_times"][0]["account_subject"] = (
            "99999999-8888-7777-6666-555555555555"
        )
        result = run(manifest=manifest)
        raw = result.files["manifest.json"]
        self.assertEqual(result.manifest["level_names_and_times"], UPSTREAM_LEVELS)
        self.assertNotIn("game_specific_data", result.manifest)
        self.assertNotIn(b"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee", raw)
        self.assertNotIn(b"99999999-8888-7777-6666-555555555555", raw)

    def test_an_export_without_quality_publishes_null_not_zeroes(self):
        """A visible absence, never zeroes that claim nothing was lost."""
        result = run(manifest={"replay_version": "5.3.2"})
        published = result.manifest
        self.assertIsNone(published["quality"])
        self.assertIsNone(published["net_field_export_groups"])
        self.assertIsNone(published["level_names_and_times"])
        self.assertIn("quality", published)
        # Nothing was dropped, so this must NOT read as a lossy conversion.
        self.assertEqual(result.tally.total, 0)


class AdapterAccountingTests(unittest.TestCase):
    """What this adapter measured, kept apart from what vrfkit declared."""

    def test_events_written_equals_the_lines_on_disk(self):
        """The identity valplay recounts: a difference means truncation."""
        result = run(manifest=full_manifest())
        lines = result.files["events.ndjson"].splitlines()
        self.assertEqual(result.manifest["adapter"]["events_written"], len(lines))
        self.assertGreater(len(lines), 0)

    def test_movement_rows_read_is_the_table_height_not_the_written_count(self):
        """Both are published; only rows read compares with the declaration."""
        movement = [
            {"time_ms": 1, "packet_id": 1, "char": 5, "pos_x": 1.0},
            {"time_ms": 1, "packet_id": 1, "char": 5, "pos_x": 2.0},
            {"time_ms": 2, "packet_id": 2, "char": 5, "pos_x": 3.0},
        ]
        adapter = run(movement=movement, manifest=full_manifest()).manifest["adapter"]
        self.assertEqual(adapter["movement_rows_read"], 3)
        self.assertEqual(adapter["movement_rows_written"], 2)
        self.assertTrue(adapter["upstream_row_counts"]["movement_rows"]["agrees"])

    def test_event_rows_are_reconciled_against_events_parquet_height(self):
        quality = json.loads(json.dumps(UPSTREAM_QUALITY))
        quality["event_rows"] = 2
        timeline = [
            {"group": "roundStarted", "time1": 100, "time2": 100,
             "word0": 7},
            {"group": "spikePlanted", "time1": 200, "time2": 200},
        ]
        adapter = run(timeline=timeline,
                      manifest=full_manifest(quality=quality)).manifest["adapter"]
        self.assertEqual(adapter["server_timeline_rows_read"], 2)
        self.assertEqual(adapter["server_timeline_events_written"], 2)
        self.assertEqual(
            adapter["upstream_row_counts"]["event_rows"],
            {"declared": 2, "observed": 2, "agrees": True},
        )

    def test_a_declared_count_its_own_table_contradicts_is_reported(self):
        """Counted with both numbers published; never repaired or refused."""
        quality = json.loads(json.dumps(UPSTREAM_QUALITY))
        quality["movement_rows"] = 999
        quality["event_rows"] = 1
        result = run(
            movement=[{"time_ms": 1, "packet_id": 1, "char": 5}],
            net_guids=[{"net_guid": 1, "outer": 2, "path": "a"},
                       {"net_guid": 2, "outer": 3, "path": "b"}],
            timeline=[{
                "group": "spikePlanted", "time1": 1, "time2": 1,
            }],
            manifest=full_manifest(quality=quality),
        )
        counts = result.manifest["adapter"]["upstream_row_counts"]
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
        self.assertEqual(
            counts["event_rows"],
            {"declared": 1, "observed": 1, "agrees": True},
        )
        self.assertEqual(result.tally["upstream_row_count_disagreement"], 1)
        self.assertGreater(result.tally.total, 0)

    def test_nothing_declared_reads_as_unknown_not_as_agreement(self):
        result = run(manifest={"replay_version": "5.3.2"})
        for name in ("movement_rows", "net_guid_rows", "event_rows"):
            check = result.manifest["adapter"]["upstream_row_counts"][name]
            self.assertIsNone(check["declared"], name)
            self.assertIsNone(
                check["agrees"],
                "an export that declared nothing must not certify itself",
            )
        self.assertEqual(result.tally["upstream_row_count_disagreement"], 0)

    def test_the_loss_tally_reaches_the_manifest(self):
        rows = MINIMAL_FIELD_ROWS + [UNNAMED_PROPERTY_ROW]
        losses = run(rows, manifest=full_manifest()).manifest["adapter"]["losses"]
        self.assertEqual(losses["unnamed_property_rows"], 1)
        # Present and zero, not absent: a key that appears only when non-zero
        # cannot distinguish "clean" from "this counter stopped running".
        self.assertEqual(losses["unnamed_rpc_rows"], 0)

    def test_the_loss_counter_set_is_pinned(self):
        """Every counter reaches the manifest, zero or not. Spelled out rather
        than read from `_Tally.REASONS`: comparing the manifest with its own
        source could not fail."""
        self.assertEqual(
            sorted(run(manifest=full_manifest()).manifest["adapter"]["losses"]),
            sorted([
                "unnamed_property_rows",
                "unnamed_rpc_rows",
                "unnamed_rpc_invocations",
                "rpc_param_collisions",
                "property_key_collisions",
                "unparsable_path_segments",
                "payload_shape_conflicts",
                "multi_typed_rows",
                "fabricated_shot_locations",
                "fabricated_shot_rotations",
                "effect_half_read_pairs",
                "effect_array_residual_bits",
                "empty_gameplay_tag_table",
                "events_time_ms_regressions",
                "upstream_row_count_disagreement",
                "unknown_actor_lifecycle_events",
                "non_finite_movement_rows",
                "raw_blobs_unavailable",
                "damaged_bone_undecoded",
            ]),
        )


class NonFiniteMovementTests(unittest.TestCase):
    """A non-finite movement value is spelled as the encoder spells it and its
    row counted (see `_write_movement`)."""

    def test_non_finite_values_are_encoded_and_counted(self):
        inf, nan = float("inf"), float("nan")
        movement = [
            {"time_ms": 1, "packet_id": 1, "char": 5, "pos_x": inf},
            {"time_ms": 2, "packet_id": 2, "char": 5, "vel_z": nan},
            {"time_ms": 3, "packet_id": 3, "char": 5, "yaw": -inf},
            {"time_ms": 4, "packet_id": 4, "char": 5, "pos_x": 1.5},
        ]
        result = run(movement=movement)
        lines = result.movement()
        rows = [json.loads(line) for line in lines]  # stdlib: non-strict
        self.assertEqual(len(rows), 4)
        self.assertIn('"position":{"x":Infinity,', lines[0])
        self.assertEqual(rows[0]["position"]["x"], inf)
        self.assertTrue(math.isnan(rows[1]["velocity"]["z"]))
        self.assertEqual(rows[2]["yaw"], -inf)
        self.assertEqual(rows[3]["position"]["x"], 1.5)
        # Rows, not values: three rows carry one non-finite value each.
        self.assertEqual(result.tally["non_finite_movement_rows"], 3)
        self.assertEqual(
            result.manifest["adapter"]["losses"]["non_finite_movement_rows"], 3)
        self.assertTrue(any(line.startswith("  non_finite_movement_rows: 3 -- ")
                            for line in result.tally.lines()))

    def test_a_row_is_counted_once_however_many_of_its_values_are_bad(self):
        inf = float("inf")
        movement = [{"time_ms": 1, "packet_id": 1, "char": 5,
                     "pos_x": inf, "pos_y": -inf, "pitch": float("nan")}]
        self.assertEqual(run(movement=movement).tally["non_finite_movement_rows"], 1)

    def test_a_collapsed_sub_move_is_not_counted(self):
        """Only rows the bundle writes are counted; movement.parquet keeps the rest."""
        movement = [
            {"time_ms": 1, "packet_id": 1, "char": 5, "pos_x": float("inf")},
            {"time_ms": 1, "packet_id": 1, "char": 5, "pos_x": 2.0},
        ]
        result = run(movement=movement)
        self.assertEqual(len(result.movement()), 1)
        self.assertEqual(result.tally["non_finite_movement_rows"], 0)
        self.assertEqual(
            result.manifest["adapter"]["losses"]["non_finite_movement_rows"], 0)

    def test_finite_movement_counts_nothing(self):
        movement = [{"time_ms": 1, "packet_id": 1, "char": 5, "pos_x": 1e-05}]
        result = run(movement=movement)
        self.assertEqual(result.tally["non_finite_movement_rows"], 0)
        self.assertIn("non_finite_movement_rows", result.manifest["adapter"]["losses"])


class ServerTimelineEventTests(unittest.TestCase):
    """Only non-private, structurally validated Event-chunk fields cross."""

    PRIVATE_MARKER = "DO-NOT-PUBLISH-ACCOUNT-SUBJECT"

    def convert_timeline(self, rows):
        marked = [{"id": self.PRIVATE_MARKER, "metadata": self.PRIVATE_MARKER,
                   "raw_payload": self.PRIVATE_MARKER.encode(), **row} for row in rows]
        result = run(timeline=marked, manifest=full_manifest())
        return (result.files["events.ndjson"], result.events("server_timeline_event"),
                result.manifest)

    def test_private_event_columns_never_cross_the_allowlist(self):
        raw, timeline, _ = self.convert_timeline([
            {"group": "characterDeath", "time1": 500, "time2": 500,
             "word0": 101, "word1": 202, "payload_tag": 8,
             "payload_name": self.PRIVATE_MARKER, "payload_seconds": 0.5},
        ])
        self.assertNotIn(self.PRIVATE_MARKER.encode(), raw)
        self.assertEqual(timeline, [{
            "type": "server_timeline_event",
            "time_ms": 500,
            "time2_ms": 500,
            "event_group": "characterDeath",
            "word0": 101,
            "word1": 202,
        }])

    def test_public_structural_payload_fields_cross_as_neutral_values(self):
        _, timeline, _ = self.convert_timeline([
            {"group": "roundStarted", "time1": 100, "time2": 100,
             "word0": 4, "payload_tag": 2,
             "payload_name": "EReplayEventGroup::RoundStart",
             "payload_seconds": 0.1},
        ])
        self.assertEqual(timeline, [{
            "type": "server_timeline_event",
            "time_ms": 100,
            "time2_ms": 100,
            "event_group": "roundStarted",
            "word0": 4,
            "payload_tag": 2,
            "payload_name": "EReplayEventGroup::RoundStart",
            "payload_seconds": 0.1,
        }])

    def test_structural_payload_tuple_fails_closed_on_tag_or_time_drift(self):
        _, timeline, _ = self.convert_timeline([
            {"group": "roundStarted", "time1": 100, "time2": 100,
             "payload_tag": 99,
             "payload_name": "EReplayEventGroup::RoundStart",
             "payload_seconds": 0.1},
            {"group": "roundStarted", "time1": 100, "time2": 100,
             "payload_tag": 2,
             "payload_name": "EReplayEventGroup::RoundStart",
             "payload_seconds": 0.2},
        ])
        for event in timeline:
            self.assertNotIn("payload_tag", event)
            self.assertNotIn("payload_name", event)
            self.assertNotIn("payload_seconds", event)

    def test_words_are_forwarded_only_for_rusts_fixed_layout_groups(self):
        _, timeline, _ = self.convert_timeline([
            {"group": "roundStarted", "time1": 100, "time2": 100,
             "word0": 4, "word1": 999},
            {"group": "spikePlanted", "time1": 200, "time2": 200,
             "word0": 888, "word1": 999},
            {"group": "futureUnknown", "time1": 300, "time2": 301,
             "word0": 777, "word1": 999},
        ])
        self.assertEqual(timeline[0].get("word0"), 4)
        self.assertNotIn("word1", timeline[0])
        self.assertNotIn("word0", timeline[1])
        self.assertNotIn("word1", timeline[1])
        self.assertNotIn("word0", timeline[2])
        self.assertNotIn("word1", timeline[2])
        self.assertEqual(timeline[2]["time2_ms"], 301)

    def test_shuffled_timeline_rows_follow_packet_time_order(self):
        _, timeline, published = self.convert_timeline([
            {"group": "spikePlanted", "time1": 300, "time2": 300},
            {"group": "roundStarted", "time1": 100, "time2": 100,
             "word0": 1},
            {"group": "characterDeath", "time1": 200, "time2": 200,
             "word0": 10, "word1": 20},
        ])
        self.assertEqual([e["time_ms"] for e in timeline], [100, 200, 300])
        self.assertEqual(published["adapter"]["events_time_ms_regressions"], 0)

    def test_the_packet_key_is_the_greatest_packet_at_or_before_the_time(self):
        # Packet 3 arrives at 30 ms after packet 5 at 20 ms: the key stays 5.
        cols = mock.Mock(time_ms=[20, 10, 30, 20], packet_id=[5, 1, 3, 2])
        self.assertEqual(bundle._packets_at_or_before(cols, [5, 10, 15, 20, 30, 99]),
                         [0, 1, 1, 5, 5, 5])


class EventOrderingContractTests(unittest.TestCase):
    """events.ndjson's order (see `_write_events`), pinned where it is set."""

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
    #: Each actor opens in the packet and millisecond of its property row.
    SPAWNS = [{**row, "event": "open", "class_path": "/Game/Test/Test_C"}
              for row in SHUFFLED]

    def test_events_are_written_in_non_decreasing_time_order(self):
        result = run(self.SHUFFLED, actors=self.SPAWNS)
        times = [e["time_ms"] for e in result.events()]
        self.assertEqual(times, sorted(times), result.events())
        self.assertEqual(result.manifest["adapter"]["events_time_ms_regressions"], 0)

    def test_a_spawn_precedes_the_property_event_at_the_same_millisecond(self):
        """The phase order is the tie-break: no actor is described before it
        exists, so the bundle reads in one pass."""
        events = run(self.SHUFFLED, actors=self.SPAWNS).events()
        for actor in (101, 202, 303):
            types = [e["type"] for e in events if e.get("actor_net_guid") == actor]
            self.assertEqual(
                types, ["actor_spawned", "export_group_received"],
                f"actor {actor} is described before it exists: {types}",
            )

    def test_a_time_ms_regression_is_counted_and_published(self):
        """A non-finite frame time exports as 0: packet order is kept and the
        regression reported, not sorted away."""
        rows = [
            {"time_ms": 10, "packet_id": 1, "actor": 101,
             "group_path": "PlayerState", "field_name": "Health",
             "bit_count": 32, "value_i64": 1},
            {"time_ms": 0, "packet_id": 2, "actor": 101,
             "group_path": "PlayerState", "field_name": "Health",
             "bit_count": 32, "value_i64": 2},
        ]
        result = run(rows)
        self.assertGreater(result.manifest["adapter"]["events_time_ms_regressions"], 0)
        self.assertGreater(result.tally["events_time_ms_regressions"], 0)
        # Packet order is preserved: the regression is reported, not repaired.
        self.assertEqual(
            [e["time_ms"] for e in result.events("export_group_received")],
            [10, 0],
        )
