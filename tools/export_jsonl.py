"""Stream an existing vrfkit Parquet table to an atomic, self-describing JSONL file.

The first line describes the selected columns and encodings; subsequent lines
carry rows without interpreting game semantics. See docs/JSONL.md.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

import atomic_io

DEFAULT_BATCH_SIZE = 8_192
FORMAT = "vrfkit.parquet-jsonl"
FORMAT_VERSION = 1
# vrf-container/src/limits.rs: FILE_MAGIC, serialized little-endian.
REPLAY_MAGIC = bytes.fromhex("dd ef f4 43")


def positive_integer(value: str) -> int:
    """An argparse row-count option, never zero or negative."""
    try:
        count = int(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("must be a positive integer") from exc
    if not 0 < count <= sys.maxsize:
        raise argparse.ArgumentTypeError("must be a positive integer no larger than sys.maxsize")
    return count


def table_path(export: Path, table: str) -> Path:
    """Select one direct child Parquet file, without accepting path traversal."""
    if not table or any(char in table for char in ("/", "\\", "\x00")):
        raise ValueError("table must be a name within the export directory")
    name = table if table.endswith(".parquet") else table + ".parquet"
    if name in (".parquet", "..parquet") or Path(name).name != name:
        raise ValueError("table must be a name within the export directory")
    source = export / name
    if not source.is_file():
        raise ValueError("selected Parquet table does not exist")
    return source


def protect_inputs(export: Path, source: Path, output: Path) -> None:
    """Protect export tables, the manifest, known replay paths and this tool."""
    resolved = output.resolve()
    if output.suffix.lower() in (".vrf", ".replay") or resolved.suffix.lower() in (".vrf", ".replay"):
        raise ValueError("output must not be a replay path")
    manifest = export / "manifest.json"
    protected = [source, manifest, Path(__file__), Path(atomic_io.__file__),
                 *export.glob("*.parquet")]
    if manifest.exists():
        document = json.loads(manifest.read_text(encoding="utf-8"))
        if not isinstance(document, dict):
            raise ValueError("manifest must be a JSON object")
        replay = document.get("source_file")
        if replay is not None:
            if not isinstance(replay, str) or not replay:
                raise ValueError("manifest source_file must be a nonempty string")
            replay_path = Path(replay)
            protected.append(replay_path)
            if not replay_path.is_absolute():
                # Existing manifests retain the CLI's source spelling. Guard
                # both the current working directory and export-relative use.
                protected.append(export / replay_path)
    try:
        atomic_io.refuse_input_path(output, protected)
    except ValueError as exc:
        raise ValueError("output aliases a protected input or implementation file") from exc
    if resolved.is_file():
        with resolved.open("rb") as existing:
            if existing.read(4) == REPLAY_MAGIC:
                raise ValueError("output contains a replay header")


def encoding_for(data_type: pa.DataType) -> str:
    """Only known flat scalar types receive an explicit, reversible encoding."""
    if pa.types.is_dictionary(data_type):
        return encoding_for(data_type.value_type)
    if pa.types.is_null(data_type):
        return "json_null"
    if pa.types.is_boolean(data_type):
        return "json_boolean"
    if pa.types.is_integer(data_type):
        return "json_integer"
    if pa.types.is_floating(data_type):
        return "json_number_finite"
    if pa.types.is_string(data_type) or pa.types.is_large_string(data_type):
        return "json_string"
    if (pa.types.is_binary(data_type) or pa.types.is_large_binary(data_type)
            or pa.types.is_fixed_size_binary(data_type)):
        return "hex"
    raise ValueError(f"unsupported selected Arrow type: {data_type}")


def selected_fields(schema: pa.Schema, columns: list[str] | None) -> list[pa.Field]:
    """Reject ambiguous or missing names instead of silently dropping data."""
    names = schema.names if columns is None else columns
    if not names:
        raise ValueError("select at least one column")
    if len(names) != len(set(names)):
        raise ValueError("selected column names must be unique")
    fields = []
    for name in names:
        index = schema.get_field_index(name)
        if index < 0:
            raise ValueError(f"selected column is missing or ambiguous: {name}")
        fields.append(schema.field(index))
    return fields


def encoded_value(value, encoding: str, *, row: int, column: str):
    """Keep null distinct from empty bytes and fail without printing values."""
    if value is None:
        return None
    if encoding == "hex":
        return value.hex()
    if encoding == "json_number_finite" and not math.isfinite(value):
        raise ValueError(f"non-finite number at row {row}, column {column}")
    return value


def write_line(handle, document: dict) -> None:
    """Always strict JSON, UTF-8, one logical record per line."""
    handle.write((json.dumps(document, ensure_ascii=False, allow_nan=False,
                             separators=(",", ":")) + "\n").encode("utf-8"))


def export_jsonl(export: Path, output: Path, *, table: str = "fields",
                 columns: list[str] | None = None,
                 batch_size: int = DEFAULT_BATCH_SIZE) -> dict:
    """Publish selected rows atomically; retain only one Arrow batch and row.

    The batch size bounds row counts, not bytes or Parquet decoder metadata.
    Unsupported types or non-finite floating values fail without replacing a
    previous output. No table values or original replay paths are printed.
    """
    if type(batch_size) is not int or not 0 < batch_size <= sys.maxsize:
        raise ValueError("batch_size must be a positive integer no larger than sys.maxsize")
    export, output = Path(export), Path(output)
    source = table_path(export, table)
    protect_inputs(export, source, output)
    with pq.ParquetFile(source, pre_buffer=False) as parquet:
        fields = selected_fields(parquet.schema_arrow, columns)
        names = [field.name for field in fields]
        encodings = [encoding_for(field.type) for field in fields]
        expected_rows = parquet.metadata.num_rows
        header = {
            "type": "schema", "format": FORMAT, "version": FORMAT_VERSION,
            "table": source.name, "rows": expected_rows,
            "columns": [
                {"name": field.name, "arrow_type": str(field.type),
                 "nullable": field.nullable, "encoding": encoding}
                for field, encoding in zip(fields, encodings)
            ],
        }
        rows = 0

        def write(handle):
            nonlocal rows
            write_line(handle, header)
            for batch in parquet.iter_batches(batch_size=batch_size, columns=names,
                                              use_threads=False):
                # Arrow column projection can include a name prefix. Verify
                # the exact selected flat schema rather than exporting extras.
                if batch.schema.names != names:
                    raise ValueError("Parquet batch columns differ from the selected schema")
                arrays = [batch.column(index) for index in range(len(names))]
                for offset in range(batch.num_rows):
                    values = {
                        name: encoded_value(array[offset].as_py(), encoding,
                                            row=rows, column=name)
                        for name, array, encoding in zip(names, arrays, encodings)
                    }
                    write_line(handle, {"type": "row", "values": values})
                    rows += 1
                del arrays, batch
            if rows != expected_rows:
                raise ValueError("decoded row count differs from Parquet metadata")

        atomic_io.atomic_write_file(output, write)
    return {"rows": rows, "columns": len(names), "table": source.name}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--export", required=True, type=Path, help="source export directory")
    parser.add_argument("--out", required=True, type=Path, help="atomic JSONL destination")
    parser.add_argument("--table", default="fields", help="existing table name, with or without .parquet")
    parser.add_argument("--columns", nargs="+", help="exact column names, in output order")
    parser.add_argument("--batch-size", type=positive_integer, default=DEFAULT_BATCH_SIZE,
                        help=f"maximum rows per Arrow batch (default: {DEFAULT_BATCH_SIZE})")
    args = parser.parse_args(argv)
    try:
        result = export_jsonl(args.export, args.out, table=args.table,
                              columns=args.columns, batch_size=args.batch_size)
    except (OSError, ValueError, TypeError) as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        return 1
    print(f"OK: {result['rows']} rows, {result['columns']} columns")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
