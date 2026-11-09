import copy, json, tempfile, unittest
from pathlib import Path
import pyarrow as pa, pyarrow.parquet as pq

from support import TempDirTestCase, run_cli
import extract_kill_observations as tool
from wire_fixtures import FIELD_SCHEMA as SCHEMA, array, field_row, write_empty_tables


def row(**kw):
    return field_row(dict(time_ms=1000, packet_id=2, channel_index=3, actor_net_guid=4, object_net_guid=5,
                          group_path=tool.GROUP, handle=15, field_name="KillData[0].bDidKillTriggerFinisher",
                          bit_count=1, raw_bits=b"\1", value_bool=True), **kw)


def fixture(two=False):
    raw, width = array([(0, [(15, 1, b"\1")])])
    child = row()
    parent = row(
        handle=0,
        field_name="KillData",
        compatible_checksum=tool.PARENT[1],
        bit_count=width,
        raw_bits=raw,
        value_bool=None,
    )
    return [child, parent] + (
        [copy.deepcopy(child), copy.deepcopy(parent)] if two else []
    )


DECL = {None: {0: tool.PARENT, **tool.DECL}}


class ParserTests(unittest.TestCase):
    def test_exact_array_and_truncation(self):
        raw, width = array([(0, [(15, 1, b"\1")])])
        self.assertEqual(tool.parse_array(raw, width)[2][0][:3], (0, 15, 1))
        with self.assertRaises(tool.InputError):
            tool.parse_array(raw, width - 1)
        with self.assertRaisesRegex(tool.InputError, "residual"):
            tool.parse_array(raw + b"\0", width + 8)

    def test_nested_unexpected_handle_and_nonexact_ref(self):
        raw, width = array([(0, [(8, 8, b"\0")])])
        with self.assertRaises(tool.InputError):
            tool.parse_array(raw, width, {7})
        with self.assertRaises(tool.InputError):
            tool.exact_ref(b"\0\0", 16)


#: Measured legacy builds, listed explicitly: iterating the tool's own set
#: would test nothing.
LEGACY_BUILDS = [
    "11.06", "11.07", "11.08", "11.09", "11.10", "11.11", "12.00", "12.01",
    "12.02", "12.03", "12.04", "12.05", "12.06", "12.07", "12.08", "12.09",
]


def build_export(root, build):
    """A minimal export directory: one finisher-only KillData update."""
    root.mkdir()
    fields = [{"handle": h, "name": v[0], "compatible_checksum": v[1]}
              for h, v in DECL[None].items()]
    manifest = {"net_field_export_groups": [{"path": tool.GROUP, "fields": fields}]}
    if build is not None:
        manifest["replay_build"] = build
    (root / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    pq.write_table(pa.Table.from_pylist(fixture(), schema=SCHEMA), root / "fields.parquet")
    write_empty_tables(root)
    pq.write_table(
        pa.Table.from_pylist([{"actor_net_guid": 4}],
                             schema=pa.schema([("actor_net_guid", pa.uint32())])),
        root / "actors.parquet",
    )
    return root


class BuildScopeTests(unittest.TestCase):
    def test_measured_builds_are_accepted(self):
        for build in (*LEGACY_BUILDS, "13.01", "13.02", "13.04", "13.05", "13.06", "13.08", "13.09"):
            self.assertIn(f"++Ares-Core+release-{build}", tool.MEASURED_BUILDS)
        for build in ("11.06", "13.06"):
            branch = f"++Ares-Core+release-{build}"
            with self.subTest(build=build), tempfile.TemporaryDirectory() as t:
                export = build_export(Path(t) / "export", branch)
                out = Path(t) / "observations.json"
                code, printed, _ = run_cli(tool.main, "--export", export, "--out", out, merged=True)
                self.assertEqual(code, 0, printed)
                self.assertIn("1 serialized updates", printed)
                result = json.loads(out.read_text(encoding="utf-8"))
                self.assertEqual(result["provenance"]["replay_build"], branch)
                self.assertEqual(result["provenance"]["wire_bits_sha256"],
                                 tool.sha(Path(tool.__file__).with_name("wire_bits.py")))
                self.assertEqual(result["counts"]["fields"]["parent_rows"], 1)
                [record] = result["observations"]
                self.assertTrue(record["members"]["did_kill_trigger_finisher"])

    def test_unmeasured_builds_are_still_refused(self):
        # 12.10, 12.11 and 13.00 export no KillData children; 13.07 matches
        # every measured identity, but a build must be measured before it is read.
        for build in ("++Ares-Core+release-12.10", "++Ares-Core+release-12.11",
                      "++Ares-Core+release-13.00", "++Ares-Core+release-13.07",
                      "12.09", "++Ares-Core+release-12.09 ", None):
            with self.subTest(build=build), tempfile.TemporaryDirectory() as t:
                export = build_export(Path(t) / "export", build)
                with self.assertRaisesRegex(tool.InputError, "outside the measured"):
                    tool.extract(export)


class ExtractionTests(TempDirTestCase):
    def run_rows(self, rows, decl=DECL, refs=None):
        root = self.tmp()
        pq.write_table(
            pa.Table.from_pylist(rows, schema=SCHEMA), root / "fields.parquet"
        )
        return tool.extract_table(
            root, "fields", decl, refs or {None: ({4}, set())}
        )

    def test_partial_stays_null_and_repeated_parents_keep_ordinals(self):
        got, counts = self.run_rows(fixture(True))
        self.assertEqual([x["physical_parent_row_ordinal"] for x in got], [1, 3])
        self.assertFalse(got[0]["members_complete"])
        self.assertIn("victim_ref", got[0]["members"])
        self.assertIsNone(got[0]["members"]["victim_ref"])
        self.assertEqual(got[0]["members"]["victim_ref_resolution"], "missing")
        self.assertIsNone(got[0]["members"]["assisting_players"])
        self.assertEqual(counts["partial_updates"], 2)

    def test_observed_empty_assistant_container_is_an_empty_list(self):
        inner, inner_width = array([])
        outer, outer_width = array([(0, [(6, inner_width, inner)])])
        container = row(
            handle=6,
            field_name="KillData[0].AssistingPlayers",
            bit_count=inner_width,
            raw_bits=inner,
            value_bool=None,
        )
        parent = row(
            handle=0,
            field_name="KillData",
            compatible_checksum=tool.PARENT[1],
            bit_count=outer_width,
            raw_bits=outer,
            value_bool=None,
        )
        got, _ = self.run_rows([container, parent])
        self.assertEqual(got[0]["members"]["assisting_players"], [])

    def test_float_negative_zero_is_bit_exact(self):
        raw_value = b"\x00\x00\x00\x80"
        outer, outer_width = array([(0, [(10, 32, raw_value)])])
        child = row(
            handle=10,
            field_name="KillData[0].DamageTaken",
            bit_count=32,
            raw_bits=raw_value,
            value_bool=None,
            value_f64=0.0,
        )
        parent = row(
            handle=0,
            field_name="KillData",
            compatible_checksum=tool.PARENT[1],
            bit_count=outer_width,
            raw_bits=outer,
            value_bool=None,
        )
        with self.assertRaisesRegex(tool.InputError, "typed value"):
            self.run_rows([child, parent])

    def test_wrong_scope_coordinates_are_rejected(self):
        rows = fixture()
        rows[0]["packet_id"] = 99
        with self.assertRaisesRegex(tool.InputError, "scope/coordinates"):
            self.run_rows(rows)

    def test_direct_wrong_null_and_extra_typed_slots_are_rejected(self):
        for changes in ({"value_bool": False}, {"value_bool": None}, {"value_i64": 1}):
            with self.subTest(changes=changes):
                rows = fixture()
                rows[0].update(changes)
                with self.assertRaises(tool.InputError):
                    self.run_rows(rows)

    def test_nonadjacent_child_is_rejected(self):
        rows = fixture()
        rows.insert(
            1,
            row(
                group_path=tool.GROUP,
                field_name="KillData[9].Victim",
                handle=3,
                value_bool=None,
                value_i64=7,
                raw_bits=b"\x0e",
                bit_count=8,
            ),
        )
        with self.assertRaises(tool.InputError):
            self.run_rows(rows)

    def test_wrong_typed_value_is_rejected_for_nested_reference(self):
        inner, iw = array([(0, [(7, 8, b"\x64")])])
        outer, ow = array([(0, [(6, iw, inner)])])
        container = row(
            handle=6,
            field_name="KillData[0].AssistingPlayers",
            bit_count=iw,
            raw_bits=inner,
            value_bool=None,
        )
        nested = row(
            handle=7,
            field_name="KillData[0].AssistingPlayers[0].AssistingPlayers",
            bit_count=8,
            raw_bits=b"\x64",
            value_i64=51,
            value_bool=None,
        )
        parent = row(
            handle=0,
            field_name="KillData",
            compatible_checksum=tool.PARENT[1],
            bit_count=ow,
            raw_bits=outer,
            value_bool=None,
        )
        with self.assertRaisesRegex(tool.InputError, "typing mismatch"):
            self.run_rows([container, nested, parent], refs={None: ({50}, set())})

    def test_main_and_checkpoint_declarations_are_distinct(self):
        bad = {None: DECL[None], 0: dict(DECL[None])}
        bad[0][15] = ("wrong", tool.DECL[15][1])
        cp = [dict(r, checkpoint_index=0, checkpoint_id="cp") for r in fixture()]
        schema = pa.schema(
            [
                ("checkpoint_index", pa.uint32()),
                ("checkpoint_id", pa.string()),
                *list(SCHEMA),
            ]
        )
        root = self.tmp()
        pq.write_table(
            pa.Table.from_pylist(cp, schema=schema),
            root / "checkpoint_fields.parquet",
        )
        with self.assertRaisesRegex(tool.InputError, "declaration"):
            tool.extract_table(root, "checkpoint_fields", bad, {0: ({4}, set())})
