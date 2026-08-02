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


if __name__ == "__main__":
    unittest.main()
