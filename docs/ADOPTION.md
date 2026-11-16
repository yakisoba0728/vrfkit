# Adopting vrfkit from source

Use the CLI for main/checkpoint Parquet tables, `ReplayReader` for
borrowed main records and optional snapshots in Rust, and the Python tools for downstream
analysis or JSONL conversion. These paths share the existing decoder and
preservation policy; they expose different output and ownership contracts.

| Need | Entry point | Read next |
| --- | --- | --- |
| Inspect metadata or check ReplayData framing | `vrfkit inspect` / `validate` | [CLI and verdict scope](USAGE.md#2-cli) |
| Analyze main and checkpoint tables | `vrfkit export --checkpoints` | [Output schemas](USAGE.md#3-output) |
| Receive optional snapshot callbacks | `ReplayReader` with `ReadOptions` | [Rust API](RUST_API.md) |
| Read a finite pipe with a byte cap | `ReplayReader::read_bounded` | [Bounded input](RUST_API.md#nonseekable-input-with-an-explicit-byte-cap) |
| Associate observed bodies and movement | `MainIdentityTimeline` | [Gameplay API](GAMEPLAY_API.md) |
| Consume records without Parquet dependencies | `ReplayReader::read_path` | [Rust API](RUST_API.md) |
| Observe the supported round, death and spike declarations | `vrfkit::gameplay` helpers | [Gameplay contract](GAMEPLAY_API.md) |
| Read selected cells in Python or another language | PyArrow batches / `tools/export_jsonl.py` | [Recipes below](#python-parquet-and-jsonl) |

## Select and record the source

This guide describes `integration/vrfkit-overhaul`. Select that branch
explicitly when cloning. Its source API and examples should not be assumed to
exist in `main` or in an older tagged binary.

```bash
git clone --branch integration/vrfkit-overhaul --single-branch https://github.com/yakisoba0728/vrfkit.git
cd vrfkit
git switch --detach
git rev-parse HEAD
rustup toolchain install 1.86.0 --profile minimal
cargo +1.86.0 build --release -p vrfkit --locked
```

Detaching keeps this checkout at the selected commit when the integration
branch advances. Save the full revision printed above with your analysis.
The repository lockfile and Rust 1.86.0 pin the documented build inputs;
record platform and enabled features too when comparing measurements.

From the repository root, run the freshly built executable:

```bash
./target/release/vrfkit inspect match.vrf --redact-identifiers
./target/release/vrfkit validate match.vrf --diagnostics
./target/release/vrfkit export match.vrf --out match-export --checkpoints
```

In Windows PowerShell, use `.\target\release\vrfkit.exe`. If you set
`CARGO_TARGET_DIR`, use its `release` directory instead. These commands do not
require a global install. To install this same source checkout on your Cargo
binary path, run `cargo +1.86.0 install --path crates/vrfkit --locked` and
ensure that directory is on `PATH`.

Keep the replay unchanged until a command returns. `validate` and `export`
read seekable files in chunks; large individual payloads, decoded chunks,
parser state and writers still require memory. `inspect` and `diag` use their
whole-file paths. See [CLI input](CLI_INPUT_STREAMING.md) for details.

`--out` must be new, empty, or an existing export containing only the expected
tables and manifest. Put derived files beside the export directory, because a
later successful export replaces that directory as a unit. Publication is
staged until decoding and every writer succeed; see
[interrupted exports](USAGE.md#if-an-export-is-interrupted).

## Rust without Parquet

For an application next to the pinned checkout, add:

```toml
[dependencies]
vrfkit = { path = "../vrfkit/crates/vrfkit", default-features = false }
```

The workspace crates are not published to crates.io. For a Git dependency,
use a published full commit ID rather than a moving branch. For example, from
that neighboring application's directory in Bash:

```bash
vrfkit_revision=$(git -C ../vrfkit rev-parse HEAD)
cargo +1.86.0 add vrfkit --git https://github.com/yakisoba0728/vrfkit.git \
  --rev "$vrfkit_revision" --no-default-features
```

This requires the selected commit to be available in the remote repository.
An unpublished local verification snapshot is not a fetchable dependency.
Keep the application's `Cargo.lock` and the source revision together. The
workspace package version does not promise Rust source compatibility across
revisions; review migration notes when changing the pin. Registry packaging
and a registry compatibility policy remain separate work, described in
[release maintenance](../CONTRIBUTING.md#tagged-windows-releases).

Disabling default features removes Arrow, Parquet and Zstd from the Reader's
dependency graph. It keeps the main-stream decoder, typed overlay, raw
preservation and quality counters. From the vrfkit checkout, these examples
require no Python packages:

```bash
cargo +1.86.0 run --release -p vrfkit --no-default-features --locked \
  --example stream_replay -- match.vrf
cargo +1.86.0 run --release -p vrfkit --no-default-features --locked \
  --example gameplay_events -- match.vrf
```

The [streaming example](../crates/vrfkit/examples/stream_replay.rs) uses
`read_path`, retains aggregate counters only, and prints after the read
returns. Input storage scales with buffered chunks and parser state, not a
retained complete replay. `read(&bytes, callback)` can instead borrow a
snapshot you already own. `read_bounded` accepts a finite nonseekable `Read`
input with a required encoded-byte cap. It buffers through EOF before
parsing; its cap is not a total process-memory limit. The
[bounded-input contract](RUST_API.md#nonseekable-input-with-an-explicit-byte-cap)
explains the one-byte over-limit probe, I/O errors and caller-owned timeouts.

The [gameplay example](../crates/vrfkit/examples/gameplay_events.rs) opts into
projections by calling `native_event` and `round_phase_event`. There is no
additional Cargo feature to enable. Native round starts, character deaths and
spike plant/defuse/explosion declarations are distinct from derived phase
observations. Nonzero actor references stay unresolved; these helpers do not
join them to players or assign kill credit. Unknown groups remain available
through their original raw Event callback. The complete list and validation
rules are in [Gameplay API](GAMEPLAY_API.md).

The two examples above skip checkpoint decoding and report its count.
`stream_checkpoints` opts into isolated snapshot callbacks; `gameplay_identity`
shows main-stream body/movement association without joining native Events to
accounts. These are source examples, with no extra Cargo feature required:

```bash
cargo +1.86.0 run -p vrfkit --no-default-features --locked \
  --example stream_checkpoints -- match.vrf
cargo +1.86.0 run -p vrfkit --no-default-features --locked \
  --example gameplay_identity -- match.vrf
cargo +1.86.0 run -p vrfkit --no-default-features --locked \
  --example read_bounded_stdin -- 67108864 --checkpoints < match.vrf
```

Use the CLI when you need checkpoint Parquet tables. Callbacks borrow reusable data and are
provisional until the Reader succeeds: clone only records you retain, and
stage external writes when partial publication would be wrong. A successful
read and `block_verdict()` are structural checks with a stated scope; inspect
decode errors, stream failures, rejected partials and lost-block counters as
separate evidence. The [Rust contract](RUST_API.md#scope-and-quality) explains
those distinctions and [schema budgets](RUST_API.md#resource-limits).

Before adopting a new source revision, review the
[Phase C migration and release checklist](PHASE_C_MIGRATION.md). It covers
checkpoint opt-in behavior, identity ordering, explicit input caps and the
required counter/output changes. The [results ledger](PHASE_C_RESULTS.md)
identifies observed acceptance separately from pending work.

## Python, Parquet and JSONL

The Python tools consume an existing export; there is no Python binding for
the Rust Reader in this repository. Install the two declared dependencies
with `python -m pip install -r requirements.txt`.

For a direct, projected read, run the
[aggregate example](examples/summarize_fields.py):

```bash
python docs/examples/summarize_fields.py match-export/fields.parquet --batch-size 2048
python docs/examples/summarize_fields.py match-export/checkpoint_fields.parquet --batch-size 2048
```

The example selects `bit_count`, `raw_bits` and the four `value_*` columns.
It uses `ParquetFile.iter_batches` with read-ahead and threaded column reads
disabled, then processes one row at a time. It prints only the row count,
rows with an overlay, rows without an overlay, rows with a stored raw window
and the number of stored raw bytes. It never prints identities or payloads.
The count of stored bytes is not a byte-coverage ratio: parent and child
windows can overlap, some rows are synthesized, and nullable raw cells are
part of the documented schema.

For another language, convert exactly the selected columns:

```bash
python tools/export_jsonl.py --export match-export --table fields \
  --columns bit_count raw_bits value_i64 value_f64 value_bool value_str \
  --batch-size 2048 --out fields.jsonl
```

`fields.jsonl` starts with a schema envelope, followed by one row envelope
per line. Do not pass the first line to a consumer expecting only data rows.
Python's standard JSON decoder retains exact integer tokens; reconstruct raw
binary with `bytes.fromhex` only for columns marked `hex` by the header.
Keep `bit_count` with `raw_bits`: stored byte padding is preserved and is not
additional meaningful payload. Empty bytes and null are different values.
See [Reading JSONL](JSONL.md#reading-the-output) for the line-by-line consumer.

Both batch examples bound row counts, not bytes or total process memory.
Large cells, Parquet pages and dictionaries still allocate memory. JSONL
conversion preserves selected logical values and row order, then publishes
the finished file atomically. It does not retain Parquet compression,
dictionary indices or row-group layout. Keep the original export for those
properties and the replay for information the export does not preserve.

For richer analytical views, use the existing tools independently:

```bash
python tools/extract_rounds.py --export match-export --out rounds.parquet
python tools/extract_damage_events.py --export match-export --out damage.parquet
```

These tools have their own evidence and join rules. A projected field count,
native Event count or JSONL row count does not establish match completeness.

## Support and preservation limits

The [supported-build table](../README.md#supported-valorant-builds) describes
exact registered global branch strings and links revision-specific audits.
It is not a promise that an arbitrary later build, other region or game mode
has the same layout. Historical private-corpus results remain historical;
consult the dated implementation report for the exact source and replay set
that was rechecked. A metadata read can succeed on an unsupported branch
without establishing that a full decode will succeed.

For fields with walkable boundaries, raw windows remain additive to typed
values, including when an overlay is unknown or fails. Unresolved whole RPCs
and native Event payloads can retain source bytes without interpreting them.
That does not make the output a lossless replay archive:

- `raw_bits` is nullable. Successfully decoded movement and synthesized child
  rows need not have a duplicate raw RPC or independent raw window.
- A whole preserved payload with unknown schema cannot be treated as named
  inner fields. Framing failures, rejected partials and lost content blocks
  are separate counters; raw rows cannot recover absent input.
- A later GUID/path snapshot is not proof that the mapping was known at an
  earlier callback. Zero GUIDs are null references, and unknown identities
  must stay unknown until evidence supports a join.
- The optional gameplay projections cover their listed observations. They do
  not infer missing events, full rounds, damage details, site/planter/defuser
  identities from spike Event payloads, or complete ability semantics.

Before selecting a parser, compare the required input/output contracts in the
[pinned ValorantReplayParser comparison](REFERENCE_COMPARISON.md#integration-contracts-at-the-pinned-revision).
Neither different default row counts nor a default-export timing ratio is an
equal-work coverage or speed result.
