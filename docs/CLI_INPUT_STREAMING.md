# CLI file input

`vrfkit validate` and `vrfkit export` read container chunks from read-only
file handles. They share the file input source of `ReplayReader::read_path`
([Reader file input](READER_INPUT_STREAMING.md)), along with the existing
decode pass. They do not retain an owned copy of the complete replay. `inspect` and `diag` still use their
existing whole-file paths.

## Input and parallelism

The main cursor validates each chunk header against the file's original
length before reading or seeking over its body. It reads native Event bodies
and skips checkpoints, repeated Headers and unknown chunks. A second,
independent cursor reads and decompresses ReplayData bodies with the existing
one-chunk lookahead. Each body has one reader; the cursors duplicate only the
small container-header scan.

For `export --checkpoints`, a third independent cursor reads only checkpoint
bodies. The checkpoint decode pass still runs beside the main pass, starts a
fresh parser state for each snapshot and writes separate checkpoint tables.
It retains one reusable encoded checkpoint buffer, plus the current decoded
snapshot. Main data, checkpoint identity and output ordering are unchanged.

Moving the CLI to file input left native counters, Parquet schemas and
manifest semantics unchanged. The 12.05 Clay FocusProjectiles route and its
required counters, a separate 0.6.0 change, do change output; see
[the output migration](PHASE_C_MIGRATION.md#export-and-counter-migration).
`source_size_bytes` is the input file's observed length, including skipped
payloads. `validate` still counts checkpoints as skipped and its verdict
covers the main ReplayData stream only. Reader checkpoint callbacks and
nonseekable input are separate, explicit library options; [the Reader contract](RUST_API.md) distinguishes
chunk-buffered paths from finite whole-input buffering with a required cap.

## Memory and errors

Input memory depends on the largest buffered metadata, native Event,
ReplayData and checkpoint payloads, rather than the total input length.
Decoded chunks, parser state and Parquet writer buffers are additional costs;
this is not a fixed process-memory limit. An individual large chunk can still
require substantial memory. Skipped payloads are checked against the file
extent before seeking, so sparse or malformed chunk lengths cannot bypass
container validation.

Keep the input unchanged until the command completes. Independent file
handles do not provide an immutable snapshot. A detected length change or
read failure aborts the command; same-length concurrent rewrites are not
guaranteed to be detected. Use a stable local copy when the source may change.

Export continues to stage every output and publish only after both passes
and every writer have succeeded. A late container, checkpoint or I/O error
discards staging and leaves a previous export in place. A main-pass error
waits for the checkpoint worker and takes precedence, as before. Prefetch
cancellation is cooperative: a helper can finish its current decompression
or scan skipped headers before observing the closed receiver.

## Verification

The `cli_input` integration tests exercise the real binary with sparse
skipped payloads, malformed late headers, checkpoint decoding and output
rollback. They compare all 13 Parquet files from compact and padded inputs,
and check the manifest's actual source size and checkpoint count. Existing
Reader, checkpoint, publication and unread-byte tests remain applicable.

The comparison below used the file-input implementation and the preceding
Reader-only revision as baseline, with identical export options. For the current source, use an explicit
schema/output-compatible baseline;
unmodified historical binaries lack the new required counter keys.
Sparse skip tests establish the skipped-payload behavior; they do not
establish a memory bound for decoded chunks or a throughput improvement.

### Public input equivalence

The file-input implementation was measured at a tree equal to commit
`47978d8635a7b7b301928df9ca811a8ea79a141d` and compared with
`9c133fb423c521d7df25b6ef19cb546fa489de4e`. Fresh exports from both
executables covered the six public 12.05, 12.08, 12.09, 12.10, 12.11 and
13.00 inputs and their 76 checkpoints. All 78 Parquet tables were
byte-identical, and all nonvolatile manifest values and required summary
counters matched. `validate` output matched after removing only its elapsed
time line. Input hashes remained unchanged.

This tests input
path equivalence using the shared decoder; it is not a new independent
reinterpretation of gameplay or the wire format. Timings captured during
these functional runs are not used as performance evidence.

The implementation adds four CLI input regressions. Full debug builds and
cross-platform CI are separate checks.

### Build identity and bounded-input proof

An initial shared Cargo target reused the profiling worktree's old
instrumented CLI even after `cargo build` reported `Fresh`. Its binary hash
matched the profiling executable exactly. The initial focused tests, public
comparisons and resource runs were therefore rejected as evidence for this
implementation. An earlier
broad release-test build also had two concurrent fat-LTO compiler processes
killed by SIGKILL; neither attempt is claimed as a full test-suite pass.

All workspace packages were then cleaned from the current release target;
external dependency caches were retained. A one-job Rust 1.86 release build
freshly compiled every runtime workspace crate. The frozen CLI's SHA-256 is
`c64d63107f79f843a58ab5b1b2db8682553f072c7ece4269bfffe044c776e219`.
The 180 tracked runtime input files matched their recorded hashes after
the build. The public comparisons above were rerun with this executable.

A synthetic file with two 1 GiB sparse unknown chunks was then run under a
512 MiB address-space limit. The baseline `validate` executable failed with
an allocation error. The new `validate` counted both chunks and returned its
expected `NoContentBlocks` status; `export --checkpoints` succeeded and
published its empty tables. This is a
skipped-input memory proof, not a bound on decoded chunks or a claim of
real-replay throughput or peak-RSS improvement. Such measurements require
separate, correctly attributed repeated runs.
