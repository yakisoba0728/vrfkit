# CLI file input

`vrfkit validate` and `vrfkit export` read container chunks from read-only
file handles. They share the file input source introduced for `ReplayReader`
in Phase A, along with the existing decode pass. They no longer retain an
owned copy of the complete replay. `inspect` and `diag` still use their
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

The native counters, Parquet schemas and manifest contract are unchanged.
`source_size_bytes` is the input file's observed length, including skipped
payloads. `validate` still counts checkpoints as skipped and its verdict
covers the main ReplayData stream only. This change does not introduce Reader
checkpoint callbacks or accept arbitrary non-seekable streams.

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

Resource measurements and public replay comparisons must use the exact
candidate executable and Phase A baseline, with identical export options.
Sparse skip tests establish the skipped-payload behavior; they do not
establish a memory bound for decoded chunks or a throughput improvement.
