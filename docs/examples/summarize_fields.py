"""Read projected vrfkit field batches and print aggregate counts only.

This example consumes main or checkpoint fields without collecting a table.
It does not establish semantic coverage or validate a replay's quality.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import pyarrow.parquet as pq


COLUMNS = ["bit_count", "raw_bits", "value_i64", "value_f64", "value_bool", "value_str"]


def positive_integer(value: str) -> int:
    count = int(value)
    if not 0 < count <= sys.maxsize:
        raise argparse.ArgumentTypeError("must be a positive integer no larger than sys.maxsize")
    return count


def summarize_fields(source: Path, batch_size: int) -> dict[str, int]:
    counts = {
        "field_rows": 0,
        "rows_with_overlay": 0,
        "rows_without_overlay": 0,
        "rows_with_raw_window": 0,
        "stored_raw_bytes": 0,
    }
    with pq.ParquetFile(source, pre_buffer=False) as parquet:
        if any(parquet.schema_arrow.names.count(name) != 1 for name in COLUMNS):
            raise ValueError("input must have the six documented field columns")
        for batch in parquet.iter_batches(
            columns=COLUMNS, batch_size=batch_size, use_threads=False
        ):
            columns = [batch.column(name) for name in COLUMNS]
            for index in range(batch.num_rows):
                # Keep the declared bit count beside the raw window when
                # extending this example to inspect bits. Do not infer that
                # all bits in the final stored byte are meaningful.
                _bit_count, raw, *overlay = [column[index].as_py() for column in columns]
                counts["field_rows"] += 1
                has_overlay = any(value is not None for value in overlay)
                counts["rows_with_overlay"] += int(has_overlay)
                counts["rows_without_overlay"] += int(not has_overlay)
                if raw is not None:
                    counts["rows_with_raw_window"] += 1
                    counts["stored_raw_bytes"] += len(raw)
        if counts["field_rows"] != parquet.metadata.num_rows:
            raise ValueError("read row count differs from Parquet metadata")
    return counts


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("table", type=Path, help="fields.parquet or checkpoint_fields.parquet")
    parser.add_argument("--batch-size", type=positive_integer, default=2048)
    args = parser.parse_args()
    # Print after the complete table was read, so an exception does not expose
    # an aggregate from a partial read as if it were complete.
    print(json.dumps(summarize_fields(args.table, args.batch_size), sort_keys=True))


if __name__ == "__main__":
    main()
