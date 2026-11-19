"""Exact, independently predicted additive Clay child rows and counter delta."""
from copy import deepcopy
from itertools import zip_longest

import pyarrow.parquet as pq

from focus_array_evidence import GROUP, PARENT, MEMBER, decode

COUNTERS = ("route_children_focus_projectiles", "focus_projectiles_empty_trailers")
INCREMENTS = ("fields_emitted", "array_elements_decoded", "array_fields_emitted")


def packed(value):
    result = bytearray()
    while True:
        chunk = (value & 127) << 1
        value >>= 7
        result.append(chunk | bool(value))
        if not value:
            return bytes(result)


def table_rows(path):
    with pq.ParquetFile(path, pre_buffer=False) as file:
        for batch in file.iter_batches(batch_size=4096, use_threads=False):
            yield from batch.to_pylist()


def expected_rows(path, counts):
    for row in table_rows(path):
        if (row["group_path"], row["handle"], row["field_name"], row["compatible_checksum"]) == (GROUP, *PARENT):
            _capacity, values, trailer = decode(row["raw_bits"], row["bit_count"])
            counts["parents"] += 1
            counts["empty_trailers"] += trailer
            for index, value, width in values:
                raw = packed(value)
                assert len(raw) * 8 == width, "unproven nonminimal packed encoding"
                counts["children"] += 1
                yield {**row, "field_name": f"FocusProjectiles[{index}].FocusProjectiles",
                    "handle": MEMBER[0], "compatible_checksum": None,
                    "bit_count": width, "raw_bits": raw, "value_i64": value,
                    "value_f64": None, "value_bool": None, "value_str": None}
        yield row


def compare_fields(previous, current):
    counts = {"parents": 0, "children": 0, "empty_trailers": 0, "unchanged_rows": 0}
    with pq.ParquetFile(previous) as old, pq.ParquetFile(current) as new:
        assert old.schema_arrow.equals(new.schema_arrow, check_metadata=True)
        old_rows, new_rows = old.metadata.num_rows, new.metadata.num_rows
    total = 0
    for expected, actual in zip_longest(expected_rows(previous, counts), table_rows(current)):
        assert expected is not None and actual == expected, "unexpected field value, raw bits, provenance, or order delta"
        total += 1
    assert total == new_rows == old_rows + counts["children"]
    counts["unchanged_rows"] = old_rows
    assert counts["parents"] == 102 and counts["children"] == 53 and counts["empty_trailers"] == 3
    return counts


def normalize_manifest(manifest, delta):
    result = deepcopy(manifest)
    for sink, children, trailers in ((result["quality"]["sink"], delta["children"], delta["empty_trailers"]),
                                    (result["quality"]["checkpoints"]["sink"], 0, 0)):
        assert sink.pop(COUNTERS[0]) == children
        assert sink.pop(COUNTERS[1]) == trailers
        for name in INCREMENTS:
            sink[name] -= children
            assert sink[name] >= 0
    return result


def normalize_counters(counters, delta):
    result = dict(counters)
    for prefix, children, trailers in (("", delta["children"], delta["empty_trailers"]), ("cp_", 0, 0)):
        assert result.pop(prefix + COUNTERS[0]) == children
        assert result.pop(prefix + COUNTERS[1]) == trailers
        for name in INCREMENTS:
            result[prefix + name] -= children
            assert result[prefix + name] >= 0
    return result
