# Phase C Reader checkpoint implementation evidence

Base: `588be1effc8dfca2bed8810fd51a006828081c1b`.
This is functional validation only. No throughput, CPU, wall-clock comparison
or peak-memory benchmark was executed or claimed.

## Scope

The Reader and CLI use one checkpoint decoder for container framing,
decompression, table declarations, fresh per-snapshot parser state, frame
walking, rejected partials and block offsets. Their adapters route records to
borrowed callbacks or the existing writers. The CLI's main-first error policy
and staged publication remain unchanged. Reader processing follows file order.

The opt-in `ReadOptions` methods introduce `ReplayEvent::Checkpoint` and
checkpoint-only totals. Old methods still skip snapshots; main counts, GUIDs,
block verdict and record variants remain main-only. See
[`RUST_API.md`](../RUST_API.md#optional-checkpoint-callbacks) for contracts and
the typed `ReplayError::CheckpointSchema` source-match migration.

## Synthetic regression and mutation evidence

`reader_checkpoints.rs` adds eleven tests through actual container, schema,
frame and packet decoding:

- Legacy/default skipping, including an invalid snapshot.
- Nonchronological frame/header times, repeated wire IDs, declared sparse
  schema/FName fields, literal/indexed GUID entries, raw field windows,
  global checkpoint field spans, and independent main/snapshot state.
- Empty snapshot and empty-frame completion.
- Slice/file equivalence with reused encoded and packet buffers.
- Independent main and checkpoint partial-reassembly finalization.
- Both main/checkpoint structural error orderings, including prefetch.
- Original callback error identity despite a later corrupt frame.
- Declaration callback refusal before later corrupt table bytes.
- Snapshot-summary callback refusal without a final replay summary.
- Fresh per-snapshot schema limits and retained typed resource-limit cause.
- A late malformed checkpoint invalidating an earlier provisional completion.

`movement_reader.rs` adds one nonempty checkpoint case using its existing
synthetic 12.10 wire encoder. Two independent checkpoints each reproduce four
movement rows, actual partial reassembly provenance, preserved unresolved
input RPCs, every sink counter (including the nine native component counters),
and every network counter from the same frames in the main stream.

Mutation control: replace the shared decoder's global checkpoint field start
with `field_row_offset.saturating_sub(1)`. The declaration/raw/state test fails
on the second snapshot with actual start 0 versus expected 1 (exit 101). The
source is restored and the core suite passes. This checks that global spans
are observed through actual decoded block callbacks.

## Local checks

Rust 1.86.0, one build job, a unique target directory, no incremental builds,
debug information disabled. Local functional builds used these profile
overrides without editing Cargo.toml:

```bash
export CARGO_PROFILE_DEV_OPT_LEVEL=0 CARGO_PROFILE_TEST_OPT_LEVEL=0
cargo +1.86.0 --config 'profile.dev.package."*".opt-level=0' \
  --config 'profile.test.package."*".opt-level=0' \
  test -p vrfkit --no-default-features --locked
```

The core suite passed after the eleven checkpoint tests were added. The final
additional movement regression then passed together with both existing
movement regressions (3/3). Core-only `cargo check --tests`, strict Clippy
(`--all-targets -- -D warnings`), strict rustdoc (`RUSTDOCFLAGS=-D warnings`)
and rustfmt also passed with the same local build settings.

Three earlier local compilation attempts were killed with SIGKILL while
compiling external dependencies; they are not successful tests. A unique disk
target and the functional profile above subsequently completed. Hosted CI
keeps its normal MSRV/Windows/Linux profiles; the local overrides do not
replace those gates.

All-feature CLI compilation, independent public callback/CLI table parity,
baseline output equality and final Windows/Linux CI are recorded by the
integration acceptance evidence. They were pending at this implementation
checkpoint and must pass before merging it.
