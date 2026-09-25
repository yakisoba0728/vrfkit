"""Fail-closed checks for the independent ability-array wire validator."""

from collections import Counter
import unittest

from tools import validate_ability_array_evidence as evidence


def packed(value):
    result = bytearray()
    while True:
        more = value >> 7
        result.append(((value & 127) << 1) | bool(more))
        if not more:
            return bytes(result)
        value = more


def path_point(fields=((1, 32), (2, 192), (3, 192))):
    raw = bytearray(packed(1) + packed(1))
    for handle, width in fields:
        raw += packed(handle + 1) + packed(width) + bytes(width // 8)
    raw += packed(0) + packed(0)
    return {"field_name": "MulticastSetPath.NetworkedProjectilePath", "raw_bits": bytes(raw), "bit_count": len(raw) * 8}


class AbilityArrayEvidenceTests(unittest.TestCase):
    def test_blind_empty_delta_admits_only_one_zero_trailer(self):
        key = next(key for key in evidence.ROUTES if key[1] == "ActiveBlinds")
        for raw in (b"\x02\0", b"\x02\0\0", b"\x04\0\0"):
            row = {"field_name": key[1], "raw_bits": raw, "bit_count": len(raw) * 8}
            self.assertEqual(evidence.inspect(row, evidence.ROUTES[key]), (0, Counter(), {}))
        for raw in (b"\x02\0\x01", b"\x02\0\x02", b"\x02\0\0\0"):
            row = {"field_name": key[1], "raw_bits": raw, "bit_count": len(raw) * 8}
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                evidence.inspect(row, evidence.ROUTES[key])

    def test_blind_sparse_delta_decodes_null_actor(self):
        key = next(key for key in evidence.ROUTES if key[1] == "ActiveBlinds")
        raw = b"\x02\x02\x18\x10\0\0\0"
        row = {"field_name": key[1], "raw_bits": raw, "bit_count": len(raw) * 8}
        count, members, children = evidence.inspect(row, evidence.ROUTES[key])
        self.assertEqual(count, 1)
        self.assertEqual(members, Counter({(11, 8): 1}))
        self.assertEqual(children, {"ActiveBlinds[0].CausingActor": (11, 8, b"\0", "value_i64", 0)})
        for altered in (raw + b"\0", raw[:4] + b"\x01" + raw[5:]):
            with self.subTest(raw=altered), self.assertRaises(ValueError):
                evidence.inspect({**row, "raw_bits": altered, "bit_count": len(altered) * 8}, evidence.ROUTES[key])

    def test_complete_path_point_consumes_entire_window(self):
        key = next(key for key in evidence.ROUTES if "NetworkedProjectilePath" in key[1])
        count, members, children = evidence.inspect(path_point(), evidence.ROUTES[key])
        self.assertEqual(count, 1)
        self.assertEqual(members, Counter({(1, 32): 1, (2, 192): 1, (3, 192): 1}))
        self.assertEqual(len(children), 3)

    def test_changed_width_unknown_handle_and_suffix_are_rejected(self):
        key = next(key for key in evidence.ROUTES if "NetworkedProjectilePath" in key[1])
        spec = evidence.ROUTES[key]
        for row in (
            path_point(((1, 32), (2, 184), (3, 192))),
            path_point(((1, 32), (4, 192), (3, 192))),
            {**path_point(), "raw_bits": path_point()["raw_bits"] + b"\0", "bit_count": path_point()["bit_count"] + 8},
        ):
            with self.subTest(row=row["bit_count"]), self.assertRaises(ValueError):
                evidence.inspect(row, spec)

    def test_missing_terminator_or_truncated_member_is_rejected(self):
        key = next(key for key in evidence.ROUTES if "NetworkedProjectilePath" in key[1])
        spec = evidence.ROUTES[key]
        row = path_point()
        for width in (row["bit_count"] - 1, row["bit_count"] - 8, 16):
            with self.subTest(width=width), self.assertRaises(ValueError):
                evidence.inspect({**row, "bit_count": width}, spec)

    def test_missing_member_and_zero_width_unknown_are_rejected(self):
        key = next(key for key in evidence.ROUTES if "NetworkedProjectilePath" in key[1])
        spec = evidence.ROUTES[key]
        for row in (
            path_point(((1, 32), (2, 192))),
            path_point(((1, 32), (2, 192), (3, 192), (4, 0))),
        ):
            with self.subTest(raw=row["raw_bits"]), self.assertRaises(ValueError):
                evidence.inspect(row, spec)

    def test_typed_comparison_fails_if_children_disappear_or_go_null(self):
        key = next(key for key in evidence.ROUTES if "NetworkedProjectilePath" in key[1])
        row = path_point()
        row.update({"time_ms": 1, "packet_id": 2, "channel_index": 3,
                    "actor_net_guid": 4, "object_net_guid": 5, "group_path": key[0]})
        _, _, expected = evidence.inspect(row, evidence.ROUTES[key])
        with self.assertRaisesRegex(ValueError, "emitted children"):
            evidence.compare_children(row, expected, [])
        emitted = []
        for name, (handle, width, raw, column, value) in expected.items():
            child = {key: row[key] for key in (
                "time_ms", "packet_id", "channel_index", "actor_net_guid", "object_net_guid", "group_path")}
            child.update({"field_name": name, "handle": 0, "compatible_checksum": None,
                          "bit_count": width, "raw_bits": raw, "value_i64": None,
                          "value_f64": None, "value_bool": None, "value_str": None})
            if column == "value_str" and isinstance(value, tuple):
                child[column] = "(" + ",".join(str(v) for v in value) + ")"
            else:
                child[column] = value
            emitted.append(child)
        evidence.compare_children(row, expected, emitted)
        emitted[0]["value_f64"] = None
        with self.assertRaisesRegex(ValueError, "null typed value"):
            evidence.compare_children(row, expected, emitted)


if __name__ == "__main__":
    unittest.main()
