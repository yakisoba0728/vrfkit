# Bounded nonseekable Reader: author verification

The additive `ReplayReader::read_bounded` entry point accepts `Read` without
`Seek`, requires an encoded-byte cap and explicit `ReadOptions`, and delegates
decoding to `read_with_options` after EOF. Its public contract is documented in
[the Rust API guide](../../../RUST_API.md#nonseekable-input-with-an-explicit-byte-cap).
The implementation introduces no second parser or temporary-file path.

The tested source commit is `872a60c91738aba4eb923337100e188fa33068ed`, with
tree `d5ef8c7dac80d1ec61d82192d9fb98f90d16249b`, based on Reader/checkpoint
commit `0cdb2e8c7e524a31ed42a55471e23305541d4634`. The machine-readable
[author receipt](author_checks.json) pins source hashes, checks and the public
fixture identity. This evidence addition changes documentation only.

## Checks performed

All Rust commands used Rust 1.86.0 on Linux, a dedicated Cargo target,
one build job, incremental compilation disabled and test/dev debug information
disabled. The repository's test optimization level remained 2.

| Check | Observed result |
| --- | --- |
| `cargo +1.86.0 test -p vrfkit --no-default-features --locked` | 192 passed, 0 failed, 0 ignored, including the crate doctest. |
| New `reader_bounded` integration suite | 11 passed; test count increases by 11 for both feature configurations. |
| `cargo +1.86.0 clippy -p vrfkit --all-targets --no-default-features --locked -- -D warnings` | Exit 0. |
| `RUSTDOCFLAGS="-D warnings" cargo +1.86.0 doc -p vrfkit --no-default-features --no-deps --locked` | Exit 0. |
| `cargo +1.86.0 fmt --check` | Exit 0. |
| `python -W error tools/check_ascii.py --check` | Exit 0. |
| `python -W error tools/check_docs.py --fast` | Exit 0; this mode does not run the workspace or Python suites. |

The focused tests use a reader that implements only `Read`. They check short
reads, repeated `Interrupted` errors including the EOF probe, first-zero EOF,
zero and maximum `usize` limits, exact-size input, one-excess-byte refusal,
zero callbacks before EOF or after input failure, original I/O and consumer
error identities, provisional callbacks on a later parser failure, checkpoint
callback order and complete summary equivalence, and typed main/checkpoint
schema-limit causes. The exact-size cases cross allocation growth boundaries.

Three deliberate production-source mutations were compiled and exercised:

| Mutation | Required failure observed |
| --- | --- |
| Skip the EOF probe when the cap is reached. | The exact-limit test fails because a callback runs before EOF. |
| Refuse `count >= remaining` instead of only `count > remaining`. | The exact-limit test fails because valid input is rejected. |
| Replace the caller's `ReadOptions` with defaults. | The checkpoint order/equivalence test fails. |

Each mutated test exited 101 with a test assertion failure, not a compiler
error. The original reader source hash was restored, and all 11 focused tests
passed again. The mutations were never committed.

## Public replay check

One public 12.10 fixture, SHA-256
`a4ce1b72f9bdf99492162013c1c909e6994a0d22bef1899e687fde71fbc86606`, was
passed through a subprocess stdin pipe to the new `read_bounded_stdin`
example. Its encoded size is 525,616 bytes.

At a cap of 525,616 bytes with checkpoints enabled, the example exited 0 and
reported 19,119 main fields, 7 completed checkpoints, 13,980 checkpoint fields
and 0 skipped checkpoints. These three row/snapshot counts matched the
existing file-backed `stream_checkpoints` example on the same fixture.
At a cap of 525,615 bytes, the stdin example exited 1 with `InputLimit` and
empty stdout. Only aggregate counts and the public content hash are retained
in this receipt.

This author check covers one replay and aggregate file/pipe parity. Independent
raw/typed validation, full-feature workspace checks, Windows runtime checks
and the final merged-candidate checks are separate integration gates.

## Contract limits

This mode holds the complete encoded replay and starts callbacks only after
EOF. The cap bounds retained input length and requested vector capacity; an
8 KiB read buffer, allocator behavior, decoded chunks, parser state and consumer
storage are separate. The one-byte EOF probe can block, and callback rejection
cannot cancel the earlier input-buffering phase. Applications configure source
timeouts or cancellation themselves. Once parsing begins, the existing
provisional callback, checkpoint ordering and error-priority contracts apply.
