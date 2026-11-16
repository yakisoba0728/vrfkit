# Streaming JSONL export

`tools/export_jsonl.py` converts an existing Parquet table into a single
self-describing JSONL file. It preserves the selected cell values and row order;
it does not infer events, resolve unknown fields or combine checkpoint and main
tables. The first line describes the schema and encodings. Each following line
contains one row.

## Quick start

Install the existing Python dependencies and export a replay with the Rust CLI:

```sh
python -m pip install -r requirements.txt
vrfkit export match.vrf --out match-export
python tools/export_jsonl.py --export match-export --out fields.jsonl
```

Select another table and only the columns needed by the consumer:

```sh
python tools/export_jsonl.py --export match-export --table fields \
  --columns packet_id channel_index handle bit_count raw_bits --out raw-fields.jsonl
python tools/export_jsonl.py --export match-export --table movement.parquet \
  --batch-size 2048 --out movement.jsonl
```

`--table` defaults to `fields`. It accepts a direct child table name with or
without `.parquet`; it must already exist. It does not accept a path or search
subdirectories. Checkpoint tables can be selected after a CLI export with
`--checkpoints`. Column names are exact and must be unique. Their output order
matches `--columns`; when omitted, all columns retain the table's order.

The command prints only the exported row and column counts. It does not print
row contents, player identifiers or the manifest's replay path. Keep resulting
files private when their source export contains private information.

## File format, version 1

This is a small synthetic example, not replay data:

```jsonl
{"type":"schema","format":"vrfkit.parquet-jsonl","version":1,"table":"fields.parquet","rows":2,"columns":[{"name":"bit_count","arrow_type":"uint32","nullable":false,"encoding":"json_integer"},{"name":"raw_bits","arrow_type":"binary","nullable":true,"encoding":"hex"}]}
{"type":"row","values":{"bit_count":7,"raw_bits":"61"}}
{"type":"row","values":{"bit_count":0,"raw_bits":null}}
```

The schema header and row envelopes prevent a table column called `type`,
`values` or `schema` from colliding with the file format. The header's `rows`
equals the Parquet metadata row count, which is checked against the emitted
count before the file is published. `arrow_type` records the Arrow type spelling;
`nullable` records the column declaration. The versioned `encoding` is the
consumer contract. All nullable values are written as JSON `null`.

| Arrow scalar type | Encoding | JSON representation |
| --- | --- | --- |
| Null | `json_null` | `null` |
| Boolean | `json_boolean` | `true` or `false` |
| Signed or unsigned integer | `json_integer` | Exact decimal integer token |
| Floating point | `json_number_finite` | Finite number; non-finite values fail |
| UTF-8 string or large string | `json_string` | Unicode string, with JSON escaping |
| Binary, large binary or fixed-size binary | `hex` | Lowercase hexadecimal string |
| Dictionary of a supported scalar | Encoding of its value type | Decoded scalar value |

Empty binary is `""`; missing binary is `null`. Hex retains every stored byte,
including zero bytes and bytes beyond a field's declared meaningful bit count.
For raw field analysis, retain the existing `bit_count` column alongside `raw_bits`;
the exporter does not reinterpret or repack bits. Strings are not parsed as
nested JSON. Nested, decimal, temporal and other unlisted selected Arrow types
are rejected before publication. An unsupported unselected column does not
prevent exporting a supported projection.

Integer tokens are not converted to floating point or truncated to 53 bits.
Consumers using JavaScript's standard `JSON.parse` must use an integer-aware
JSON parser when columns can exceed the exact range of `Number`. Python's
standard JSON decoder retains these integer values. Finite floats preserve the
numeric values, including signed zero; NaN payload bits are outside this format.
`NaN`, positive infinity and negative infinity fail the whole conversion rather
than becoming a null or a non-standard JSON token.

The file describes selected logical values. It does not retain Parquet physical
encodings, dictionary index assignments, row-group boundaries, compression or
custom metadata. It is not a replacement for the original Parquet export or
the replay archive.

## Reading the output

Read one line at a time; do not load the complete file unless the application
needs to retain every row:

```python
import json

with open("raw-fields.jsonl", encoding="utf-8") as stream:
    schema = json.loads(next(stream))
    assert schema["format"] == "vrfkit.parquet-jsonl"
    assert schema["version"] == 1
    encodings = {field["name"]: field["encoding"] for field in schema["columns"]}
    for line in stream:
        record = json.loads(line)
        assert record["type"] == "row"
        row = record["values"]
        for name, encoding in encodings.items():
            if encoding == "hex" and row[name] is not None:
                row[name] = bytes.fromhex(row[name])
        # Analyze this row, then release it before reading the next one.
```

The Python helper can also be called directly after adding `tools` to the
application's import path:

```python
from pathlib import Path
from export_jsonl import export_jsonl

counts = export_jsonl(
    Path("match-export"), Path("raw-fields.jsonl"),
    table="fields", columns=["bit_count", "raw_bits"], batch_size=2048,
)
assert counts["columns"] == 2
```

## Resource and failure behavior

The implementation reads `ParquetFile.iter_batches`, with read-ahead buffering
and threaded column reads disabled. It holds one Arrow batch and converts one
row at a time. It never calls `read_table`, materializes a complete table or
collects the batch iterator. The default batch limit is 8,192 rows;
`--batch-size` must be a positive integer representable on the platform.

This is a row-count bound, not a fixed byte or RSS limit. A single large binary
value, Parquet pages and dictionaries, schema metadata, the source manifest and
JSON encoding buffers still require memory. Hex doubles the payload's character
count and JSONL usually takes more disk space than compressed Parquet. Select
columns and lower the batch size when memory is constrained. No throughput or
peak-memory improvement over Parquet is claimed.

Output is written to a temporary sibling, flushed and atomically replaced only
after every row succeeds. A non-finite value, unsupported type, read failure,
row-count mismatch or write failure preserves an existing complete destination
and removes the incomplete temporary file. There is no stdout streaming mode,
because stdout cannot provide this publication guarantee. A successful return
means the whole selected table was published.

The existing atomic-I/O helpers reject destinations aliasing the source table,
any other Parquet table in the export, the manifest or tool implementation.
Replay extensions, detected replay file headers and replay paths named in an
available manifest are also protected. Relative manifest paths are checked
against the current working directory and the export directory; the original
export command's working directory is not recorded. Hard links are safe because
publication replaces the destination name, leaving the input's bytes intact.

## Verification

The focused tests build synthetic Parquet inputs. They cover exact binary and
integer values, null versus empty values, Unicode and escaped newlines,
dictionaries, projection order, empty tables, strict floating-point handling,
input aliases, failed reads and writes, and reader closure. A streaming test
requires earlier rows to be written before the next batch is requested, so an
implementation that first collects all batches fails the test.

```sh
python -W error -m unittest discover -s tools/tests -p test_export_jsonl.py -v
```

The public-fixture check converted all main and checkpoint tables from baseline
exports at `67c0a2d94fbcbeeefa56761f1bc13d9f6c829fb0`, using batches of 257 rows.
An independent PyArrow read supplied the expected values; each JSONL binary
value was hex-decoded before comparison, and finite floats were also compared
by their packed value to include signed zero. Input SHA-256 values were
unchanged before and after conversion.

| Public build | Tables | Rows compared | Binary bytes compared |
| --- | ---: | ---: | ---: |
| 12.10 | 13 | 59,180 | 123,895 |
| 12.11 | 13 | 40,710 | 97,616 |
| 13.00 | 13 | 38,837 | 76,622 |

All selected values matched across these 138,727 rows. This verifies conversion
of the existing raw and typed tables; it does not independently validate the
game interpretation that produced those tables. Replacing streaming iteration
with collection of every batch made the streaming regression fail; the original
implementation passed the same test.
