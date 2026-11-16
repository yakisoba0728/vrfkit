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

`read_path` opens the file read-only and reads it one container chunk at a
time. It retains a reusable encoded-chunk buffer, the current decompressed
chunk and at most one prepared chunk ahead. Checkpoint, repeated Header and
unknown payloads are range-checked before seeking past them. Checkpoints are
counted as skipped by default. `read_path_with_options` can explicitly enable
snapshot callbacks, as described below. Metadata parsing buffers the format's
bounded info prefix and the declared Header payload, then releases them.

If you already have the bytes, `reader.read(&bytes, callback)` borrows them
without another whole-file copy. Both input paths use the same decode pass,
callback ordering and quality counters. The CLI's `validate` and `export`
commands now reuse the same file input source; `export --checkpoints` adds an
independent checkpoint cursor. See [CLI file input](CLI_INPUT_STREAMING.md).

Input memory scales with the largest encoded chunk, current and prefetched
decoded chunks, metadata and parser state, rather than total file size.
An individual Event payload and Header payload are still buffered in full;
there is no new fixed total-memory guarantee. The consumer controls how much
output it retains. Generic streaming from an arbitrary `Read` source is not
part of the high-level Reader API; the file path requires seeking.

Keep the input file unchanged until the read returns. File-backed input is
not an immutable snapshot. A detected length change or I/O failure aborts the
read, but a same-length concurrent rewrite is not guaranteed to be detected.
Use `read(&bytes, ...)` on an owned snapshot if that isolation is required.

The [input-streaming measurements](READER_INPUT_STREAMING.md) record the
memory comparison, rejected scheduling experiment and exact evidence scope.

## Callback and ownership contract

| Notification | Meaning |
| --- | --- |
| `Metadata(&Preamble)` | Info and header, once before any decoded output. A full read has already checked the branch is supported. |
| `Records(ReplayRecords)` | Borrowed fields, movement samples, actor lifecycle rows and rejected partials from one packet. Empty batches are omitted. A final batch flushes unfinished partials. |
| `ContainerEvent(EventChunk)` | The native Event chunk and its exact declared payload. The payload is borrowed; unknown groups stay available. This is not an inferred gameplay event. |
| `Checkpoint(CheckpointEvent)` | Opt-in independent snapshot metadata, declarations, records and completion; never mixed with `Records`. |

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
The decompression helper notices cancellation at its next send. It may finish
its current decompression or scan intervening non-ReplayData headers first;
callback cancellation does not promise immediate termination of all I/O.
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

The default API decodes the **ReplayData stream and native Event chunks**.
Checkpoint snapshots are independent streams; default methods report them in
`skipped_checkpoint_chunks`. Opt-in checkpoint callbacks use the exact decoder
that drives `export --checkpoints`, including declarations and block boundaries.

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
consumer storage. `read_path_with_schema_limits` uses the same chunk-based
file input as `read_path`. These legacy methods still skip checkpoints. The
`ReadOptions` methods combine limits with optional snapshot decoding; each
snapshot receives its own fresh cache with the same limits.

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
The shared implementation does not add a second decoder. Rust consumers can
replace subprocess execution with callbacks for main and checkpoint records;
the CLI remains available for staged Parquet publication.

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

## Optional checkpoint callbacks

```rust
use std::convert::Infallible;
use vrfkit::{CheckpointEvent, ReadOptions, ReplayEvent, ReplayReader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let summary = ReplayReader.read_path_with_options(
        "match.vrf",
        ReadOptions { checkpoints: true, ..ReadOptions::default() },
        |event| {
            if let ReplayEvent::Checkpoint(CheckpointEvent::Records {
                checkpoint, records, blocks,
            }) = event {
                // These packet/GUID namespaces belong only to this snapshot.
                let identity = &checkpoint.identity;
                let _ = (identity, records.fields, blocks);
            }
            Ok::<(), Infallible>(())
        },
    )?;
    println!("{} snapshots completed", summary.checkpoints.chunks);
    Ok(())
}
```

`read_with_options(&bytes, options, callback)` provides the equivalent borrowed
slice input. `stream_checkpoints` is a runnable example that prints aggregate
counts only and works without default features:

```bash
cargo +1.86.0 run -p vrfkit --no-default-features --example stream_checkpoints -- match.vrf
```

Every snapshot is decoded at its container position. Its callbacks are:

1. `Metadata`: observed zero-based checkpoint index, wire ID, group, metadata,
   both wire header timestamps, archive size and trailing-byte count. Wire IDs
   may repeat; use index plus ID. Frame timestamps remain unchanged in records.
2. `GuidEntry`, `ExportGroup`, `ExportField`: exact declaration order, including
   original path variants, flags, sparse slots, checksums and FName metadata.
3. `Records`: one packet's field/actor/movement/provenance/partial rows and
   exact content-block boundaries. Batches with only empty blocks are delivered.
   Rejected partials are flushed before completion, labelled `checkpoint` and
   with their own checkpoint ID. Movement is a snapshot observation, not a
   timeline update; the CLI continues to count and omit checkpoint movement.
4. `Summary`: this snapshot's detached counters, final GUID snapshot and limited
   diagnostic lists. Clone it inside the callback to keep it. Empty snapshots
   still produce metadata and a summary, with zero frames or records as observed.

Each snapshot starts a fresh parser, channel map, schema cache and GUID map.
Its packet IDs restart from zero; main packet IDs continue independently.
The `CheckpointBlockRecord.field_row_start` value addresses the combined
checkpoint field stream across all snapshots, matching the CLI table contract.
The block index restarts for each snapshot. Main summary counters, final GUIDs
and `block_verdict()` retain their original main-only meaning. The separate
`summary.checkpoints` contains completed snapshot totals; detailed counters
are available in each snapshot summary, including all nine native component
counters. Enabling checkpoints makes `skipped_checkpoint_chunks` zero because
every encountered snapshot must complete for a successful return.

File input buffers one checkpoint body in the reusable event/checkpoint buffer
and its decoded archive. It does not retain all snapshots or a whole-file copy.
Record slices and declaration references are callback-scoped. Callers own any
retained copies. Schema limits bound groups and field slots in each independent
cache; they do not impose a fixed total memory bound.

### Failure ordering and migration

Reader traversal stops at the first structural failure in file order. An
earlier invalid checkpoint wins over a later main chunk, even if the main
prefetch worker already found its error; an earlier main failure wins over a
later checkpoint. An original consumer error outranks a later error encountered
while the current frame walk unwinds. No later callback runs after that error.
No final `ReplaySummary` is returned on any failure. A checkpoint `Summary`
signals only that snapshot's completion and remains provisional until the
enclosing call succeeds. In contrast, the CLI retains its existing concurrent
two-pass policy: it joins the checkpoint worker and a main-pass error takes
precedence. The callback ordering contract is sequential and intentionally
does not promise that parallel CLI error ordering.

Existing reader constructors and methods remain valid and retain checkpoint
skipping. `ReplayEvent` was already `#[non_exhaustive]`; external exhaustive
matches already needed a wildcard. Handle its new `Checkpoint` variant only
when opting in. `ReplayError::CheckpointSchema { checkpoint, error }` replaces
the old CLI-only rendered usage error for checkpoint table/schema failures.
Its displayed text is unchanged, and its source retains the typed
`SchemaError`, including resource refusals. Consumers matching every
`ReplayError` variant must add this variant or a wildcard. CLI commands,
Parquet columns and existing main counters are unchanged.
