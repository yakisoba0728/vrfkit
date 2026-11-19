# Phase A independent Reader acceptance

Agent E reviewed and tested the combined Native fix and file-input Reader on
2026-11-16. The Reader implementation passes this focused independent review.
Final hosted CI on the published integration tree remains a separate merge
gate; this receipt does not claim a local Windows run.

## Reviewed source

The local combined runtime commit was
`fc1f30a9c4a3a4c49183d246fdf0a31d415570d0`, tree
`931d8a8db1675f624876472cc4f88bed977872d7`. It comprises Native QA base
`ba1b2adfb30d6e3fa68c5fdf3c87042135a250c6` and Reader commits
`d78f30abc4ff9de837e53d5cd9273d96c2b416f8` and
`6a9ea4eb485ff9de6abfa12c2cf036904ba83d91`, cherry-picked into the isolated QA
branch. All eight changed Reader crate files were compared byte for byte with
the author revision. Their SHA-256 values are in
[the independent review receipt](../evidence/phase-a-reader-independent-review.json).
The withdrawn movement inline experiment is absent.

## Fresh executions

The independently authored file-input helper passed **6 tests, 0 failures**
under Rust 1.86.0 with locked dependencies and no default features. Three
tests exercise the 31 previously independent Native cases through `read_path`:
unsupported A3/A4 residues, valid intentional raw omission and invalid signed
envelopes. They retain full RPC bytes, Movement prefix values and provenance,
and verify recovery for a later component and a later ordinary RPC.

Two new behavioral tests verify that stopping at a Records callback returns
the exact original boxed error, suppresses later callbacks and takes
precedence over a later prefetched container failure; and that a later typed
container truncation leaves earlier callbacks delivered without returning a
successful summary. The sixth test constructs the sparse input used below.
These are synthetic transport fixtures, not a second real replay corpus.

The reviewer also freshly ran the existing 15 Reader API tests, six file-input
tests and five preamble tests on this combined runtime: **26 passed**. These
include every truncated prefix, typed I/O and container errors, Event ordering
and stop behavior, negative and out-of-file skipped lengths, schema limits,
maximum admitted info fields and a large Header with counted input reads.
The independent helpers live under `docs/verification`; their temporary Cargo
copies were removed after execution, so they do not change the normal suite
counts.

## Independent memory control

The generated fixture contains a valid preamble, a 1GiB unknown payload, a
1GiB checkpoint payload and a final Event. Payload holes occupy no actual
replay data. Its logical length is **2,147,483,988 bytes** and allocated size is
**12,288 bytes**. The generated input is outside the repository and is not
committed.

The same independently built executable ran both modes under Linux
`RLIMIT_AS = 536,870,912` bytes and `RLIMIT_CORE = 0`:

| Input mode | Result | Measured child peak RSS |
| --- | --- | ---: |
| `ReplayReader::read_path` | Exit 0; exactly two callbacks, one Event, one unknown chunk and one skipped checkpoint | 7,630,848 bytes |
| `std::fs::read` followed by `ReplayReader::read` | Exit 1; `Error: Kind(OutOfMemory)` | 7,634,944 bytes |

This negative control shows that the acceptance property fails when the
entire input must be buffered. The small failed-buffer RSS is expected:
allocation is refused before the 2GiB buffer is created. This is a synthetic
skipped-payload proof, not a speed benchmark or a bound on decoded chunk,
Header, Event, parser or consumer memory.

The [memory receipt](../evidence/phase-a-reader-memory-independent.json)
records the executable and script hashes, exact limits, exit codes and output.
The helper verifies that the executable hash and generated input size/mtime
remain unchanged across both modes.

## Review of measured results

The reviewer recomputed each paired ratio, metric median, CPU total and CPU
utilization from the recorded samples, checked the report hashes and executable
identities, and checked the six author-produced file/slice parity entries.
This is a review of those executions, not an additional timing run or six
independent combined-source replay decodes.

The final Reader's paired RSS ratios are about 0.4632 on 12.09 and 0.3647 on
12.05. Its paired wall ratios are about 0.9741 and 1.0170; the same-command
control's timing variation prevents a general speed claim. The rejected first
prototype remains in the reviewed evidence.

The performance experiment's four series contain 60 successful export
receipts and 668 recorded Parquet table fingerprints. The reverse-label CPU
ratio changes the direction of the apparent benefit, and the identical-binary
control also varies. The decision to withdraw the inline attribute is
supported. The reviewer checked that the withdrawal commit
`325acda3092600aa3944c7d123f12a4fcdd6890b` has no crate changes against its
unannotated baseline. **There are zero accepted runtime optimizations from
that experiment.** Actual output files were not reread during this receipt
review; the earlier independent Native QA separately reread its 78 Parquet
pairs.

See [the independent measurement review](../evidence/phase-a-measurements-independent.json)
for recomputed values and a precise distinction between new executions and
reviewed author evidence.

## Contracts and limits

The code uses two independently opened read-only file cursors, avoiding shared
OS offsets from `try_clone`. Both scan eight-byte headers; one reads Events
and the other reads and decompresses ReplayData. Payloads are read once apart
from the bounded metadata probe. A rendezvous channel retains at most one
prepared chunk ahead of the current chunk.

Input must remain unchanged during the call. Same-length concurrent rewrites
are outside the documented snapshot guarantee. Memory depends on maximum
chunk sizes, full Header/Event payloads and parser/consumer state; there is no
fixed global memory cap. Consumer stop prevents further callbacks, but the
scoped worker can finish current decoding or scan a header-only tail before
joining. No bounded cancellation latency was measured or promised.

The CLI and Parquet input-loading paths are unchanged by this Reader work.
Final Windows/Linux CI, feature checks, Clippy, rustdoc, Python and public
replay validation must run on the published final tree. These checks are
managed by the coordinator, not inferred from this local acceptance.

## Reproduce the independent file and memory checks

Run from the repository root on Linux with Rust 1.86.0 and Python available.
The two temporary Cargo paths must not already exist. The generated sparse
fixture must be a new path; the helper refuses to overwrite one.

```bash
set -e
QA_RUN_DIR="$(mktemp -d)"
test ! -e crates/vrfkit/tests/phase_a_reader_qa.rs
test ! -e crates/vrfkit/examples/phase_a_reader_memory.rs
cp docs/verification/phase_a_reader_input.rs crates/vrfkit/tests/phase_a_reader_qa.rs
cp docs/verification/phase_a_reader_memory.rs crates/vrfkit/examples/phase_a_reader_memory.rs
VRFKIT_QA_SPARSE_PATH="$QA_RUN_DIR/input.vrf" cargo +1.86.0 test -p vrfkit --no-default-features --locked --test phase_a_reader_qa
cargo +1.86.0 build -p vrfkit --no-default-features --locked --example phase_a_reader_memory
python3 docs/verification/phase_a_reader_memory.py "${CARGO_TARGET_DIR:-target}/debug/examples/phase_a_reader_memory" "$QA_RUN_DIR/input.vrf" "$QA_RUN_DIR/memory.json"
cargo +1.86.0 test -p vrfkit --no-default-features --locked --test reader_api --test reader_input
cargo +1.86.0 test -p vrf-container --no-default-features --locked --test read_preamble
rm crates/vrfkit/tests/phase_a_reader_qa.rs crates/vrfkit/examples/phase_a_reader_memory.rs
```

To recompute measurement receipts, supply a directory containing the four
`reader-input-*.json` receipts and `reader-public-parity-prefetch.json`, and the
directory containing the performance `phase-a-inline-*.json` receipts:

```bash
python3 docs/verification/phase_a_measurement_review.py "$READER_EVIDENCE_DIR" "$PERFORMANCE_EVIDENCE_DIR" "$QA_RUN_DIR/measurements.json"
```

The reviewer did not mutate source implementations in this Reader pass. The
whole-file memory negative control is an executed control, while the Native
guard mutations are recorded separately in the earlier Native QA receipt.
