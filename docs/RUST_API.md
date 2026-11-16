# Streaming Rust reader

The `vrfkit` crate exposes `ReplayReader` alongside its CLI. It runs the same
main-stream parser, raw preservation and typed overlay as `validate` and
`export`, with synchronous callbacks instead of output files. There is no
second decoder to keep in sync.

This API is currently available from source. The workspace crates are not
published to crates.io; API compatibility follows the source revision you pin.
For a neighboring checkout:

```toml
[dependencies]
vrfkit = { path = "../vrfkit/crates/vrfkit", default-features = false }
```

Disabling default features removes Arrow, Parquet and Zstd. It does not turn
off the main-stream decoder or omit rows. Enabling the default `export`
feature also builds the CLI's Parquet writers.

## Read a replay

```rust
use std::convert::Infallible;
use vrfkit::{ReplayEvent, ReplayReader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut unknown = 0_u64;
    let summary = ReplayReader::new().read_path("match.vrf", |event| {
        if let ReplayEvent::Records(batch) = event {
            for field in batch.fields {
                if field.value_i64.is_none()
                    && field.value_f64.is_none()
                    && field.value_bool.is_none()
                    && field.value_str.is_none()
                {
                    unknown += 1;
                    // field.raw_bits and field.bit_count still describe the
                    // preserved window. Clone a row only if you need to keep it.
                }
            }
        }
        Ok::<(), Infallible>(())
    })?;

    println!("{} fields without a typed overlay", unknown);
    println!("block verdict: {:?}", summary.block_verdict());
    Ok(())
}
```

The complete runnable example prints aggregate counts without printing
player identifiers or payload contents:

```bash
cargo +1.86.0 run --release -p vrfkit --no-default-features --example stream_replay -- match.vrf
```

`read_path` reads the whole file into memory. If you already have the bytes,
`reader.read(&bytes, callback)` borrows them without another whole-file copy.
Output is streamed; this API does not yet stream the input from `Read`/`Seek`.
The parser keeps replay state, reusable packet buffers, the current decompressed
chunk and at most one prefetched decompressed ReplayData chunk. The consumer
controls how much output it retains.

## Callback and ownership contract

| Notification | Meaning |
| --- | --- |
| `Metadata(&Preamble)` | Info and header, once before any decoded output. A full read has already checked the branch is supported. |
| `Records(ReplayRecords)` | Borrowed fields, movement samples, actor lifecycle rows and rejected partials from one packet. Empty batches are omitted. A final batch flushes unfinished partials. |
| `ContainerEvent(EventChunk)` | The native Event chunk and its exact declared payload. The payload is borrowed; unknown groups stay available. This is not an inferred gameplay event. |

Callbacks run synchronously in file order. Timestamps need not be sorted.
Field, actor and movement slices use reusable parser buffers and cannot outlive
the callback. Copy only the records you need. The final summary is detached
from the input and includes the final GUID/path snapshot, sorted by GUID.
An object path learned later must not be interpreted as proof that the same
mapping was already known at an earlier timestamp.

**Callbacks are provisional until `read` returns successfully.** A later
container or frame error cannot undo records already delivered. If publishing
partial output would be wrong for your application, stage it and commit only
after the read succeeds and the relevant quality checks pass.

Return `Err(your_error)` from a callback to stop delivery. `ReadError::Consumer`
returns the original error type and value; no later callback runs. A frame
reader may finish unwinding its current chunk, but a later parser error does
not replace the consumer error. Parser and I/O errors use `ReadError::Replay`.
The API does not print errors or diagnostics automatically. Consumer panics
propagate normally.

`ReplayReader` is stateless: each call starts a fresh replay state. The same
reader can be reused and independent calls may run concurrently. Parallelism
across replays does not change the sequential ordering within one replay.

## Metadata without decoding

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read("match.vrf")?;
    let metadata = vrfkit::ReplayReader::new().read_metadata(&bytes)?;
    println!("{}", metadata.header.replay_version.branch);
    Ok(())
}
```

Metadata parsing can describe an unsupported branch. A full read rejects it
before the metadata callback. Metadata-only success is not a decode check.

## Scope and quality

The initial API decodes the **ReplayData stream and native Event chunks**.
Checkpoint snapshots are independent streams and are not decoded here:
`skipped_checkpoint_chunks` reports how many were encountered. Use the existing
`export --checkpoints` command for checkpoint output and verification.

`summary.block_verdict()` uses the exact `validate` acceptance rule. It covers
framed ReplayData blocks and its documented transport failures; it does not
cover checkpoints, all partial-reassembly rejections, type correctness or
complete gameplay understanding. `NoContentBlocks` is different from a pass.

Inspect the separate evidence:

- `network` contains complete replication counters, preserved unresolved RPC
  counts, lost-block counts, partial rejections and capped diagnostic events.
- `decode_counters` contains every existing sink counter, including zeroes.
  Missing names are not zero; new counters may be added in later revisions.
- `decode_error_count` counts all overlay failures. `decode_errors` retains up
  to 32 frequent identities; `stream_failures` retains up to 32 descriptions.
- `frame_skips`, trailing-byte counts, unknown chunks and non-finite frame
  times identify material that was not fully interpreted.
- Row counts measure emitted data, not gameplay-semantic coverage. Raw field
  windows remain additive to typed values. Successfully decoded movement does
  not necessarily retain a duplicate raw RPC, as in the existing export.

The shared CLI implementation is also callable through `run_cli`, which
preserves command-line output and exit codes. Use `ReplayReader` for a
library integration that owns its output and error handling.

### Resource limits

`ReplayReader` uses the shared parser's default schema limits: at most 4,096
registered export groups and 1,048,576 declared field slots, including empty
slots. These are resource policies, not game-format limits or a bound on total
process memory. The input buffer, decompressed chunks, strings, GUID mappings
and retained consumer output require additional memory. See
[Schema allocation limits](SCHEMA_LIMITS.md).

Use `read_with_schema_limits` or `read_path_with_schema_limits` to choose
budgets for one complete main-stream read:

```rust
use std::convert::Infallible;
use vrfkit::{ReplayReader, SchemaLimits};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let limits = SchemaLimits {
        max_groups: 1_024,
        max_field_slots: 65_536,
    };
    let summary = ReplayReader::new().read_path_with_schema_limits(
        "match.vrf",
        limits,
        |_| Ok::<(), Infallible>(()),
    )?;
    println!("{} movement rows", summary.movement_rows);
    Ok(())
}
```

For bytes already in memory, call
`reader.read_with_schema_limits(&bytes, limits, callback)`. The configured
cache accumulates declarations across every ReplayData chunk of this read.
Each later read starts fresh; its limits never inherit another call's choices.
`read` and `read_path` keep using `SchemaLimits::default()`.

Zero is a real limit: zero groups rejects even empty group declarations, and
zero slots allows only empty groups within the group limit. Raising a budget
increases the allocation an input can request. These limits do not constrain
the input file, native Event payloads, decompressed chunks, GUID mappings or
consumer storage. `read_path_with_schema_limits` still buffers the complete
file before parsing. Checkpoint contents remain skipped and outside this
Reader API's validation scope.

A refused declaration fails the read before allocating its field slots.
Earlier callbacks remain provisional, and no successful summary is returned.
The error reaches the Reader through `ReadError::Replay`; its
`std::error::Error::source()` chain retains `SchemaError::ResourceLimit` with
the actual resource, requested amount and configured limit. `SchemaError` is
re-exported by `vrfkit` so callers can downcast sources without parsing error
messages. Malformed schema errors retain their distinct typed causes too.
A consumer error still returns the original `E` and takes precedence over a
later schema refusal encountered while the current frame chunk unwinds.

Lower-level integrations can continue using
`vrf_schema::NetGuidCache::with_limits` directly. Creating a separate cache
does not configure a `ReplayReader` call.

## Movement source identities

Each `ReplayEvent::Records` batch supplies `movement_provenance` in the same
order and with the same length as `movement`. It identifies the actual initial
and completing packets, actor/object/channel, content block, RPC callback,
sparse update index, property occurrences and move ordinal. Every Reader call
captures this companion slice; consumers can borrow it or clone selected rows.
The CLI's existing Parquet schema and collection path remain unchanged.

See [Movement provenance](MOVEMENT_PROVENANCE.md) for field definitions,
repeated-property limits and the streaming diagnostic example. Numeric values
and timestamps are not used to guess transport identities. A context describes
the emission observed by this decoder; later empty replacement properties or
failed streams must not be treated as proof of a final gameplay state.

## Compatibility

Introducing the reader leaves existing CLI arguments and Parquet column
schemas unchanged. The library and CLI share the parser, so both inherit the
fixes and resource policies of the source revision in use. Pin a revision
when integrating the source API and review its migration notes before
updating. Commit the application's Cargo lockfile for reproducible dependencies.
The shared implementation does not add a separate decoding pass. Rust consumers can replace subprocess
execution with the reader when they need main-stream records, while keeping
the existing CLI for checkpoint exports.

### Typed schema errors

`FrameError::Schema` now contains `vrf_schema::SchemaError` instead of a
rendered `String`. This is a source API change for callers that construct or
destructure that variant. Match the schema variant to distinguish resource
policy from corrupt data; call `error.to_string()` when a string is needed.
The formatted CLI message is unchanged. The standard error source chain now
retains the schema cause instead of ending at the frame boundary.

Existing `ReplayReader`, `ReplayReader::new()`, `Default`, `read` and
`read_path` calls remain valid. The configurable methods are additive, and
default limits, CLI arguments and Parquet schemas are unchanged.
